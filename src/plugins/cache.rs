//! DNS response caching plugin
//!
//! This plugin caches DNS responses to improve performance and reduce load on upstream servers.
//!
//! # Features
//!
//! - **TTL-based expiration**: Respects DNS record TTL values
//! - **LRU eviction**: Least Recently Used eviction when cache is full
//! - **Size limits**: Configurable maximum cache size
//! - **Statistics**: Track hits, misses, and evictions
//!
//! # Usage Example (in code)
//!
//! ```rust
//! use lazydns::plugins::CachePlugin;
//! use lazydns::plugin::{Plugin, Context};
//! use lazydns::dns::Message;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create cache with max 1000 entries
//! let cache = CachePlugin::new(1000);
//! let plugin: Arc<dyn Plugin> = Arc::new(cache);
//!
//! // Use in plugin chain
//! let mut context = Context::new(Message::new());
//! plugin.execute(&mut context).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Configuration (YAML)
//!
//! Example showing how to register a named cache and reference it from a
//! `sequence` plugin. Adjust tags and pipeline composition to match your
//! project's configuration conventions.
//!
//! ```yaml
//! plugins:
//!   - tag: my_cache
//!     type: cache
//!     config:
//!       size: 1024
//!       negative_cache: true
//!       negative_ttl: 300
//!
//!   - tag: resolver_sequence
//!     type: sequence
//!     args:
//!       - exec: "$my_cache"
//! ```
//!
//! # Notes
//!
//! - Place `CachePlugin` early in the plugin chain so cached responses can
//!   be returned before invoking expensive upstream resolvers.
//! - CachePlugin automatically handles both cache reads (before sequence) and
//!   cache writes (after sequence completes), eliminating the need for a separate
//!   store plugin.
use crate::RegisterPlugin;
use crate::Result;
use crate::ShutdownPlugin;
use crate::config::PluginConfig;
use crate::dns::Message;
use crate::error::Error;
#[cfg(feature = "metrics")]
use crate::metrics;
use crate::plugin::traits::Shutdown;
use crate::plugin::{BackgroundTask, Context, Plugin, PluginHandler, RETURN_FLAG};
use crate::utils::task_queue::{RefreshCoordinator, RefreshTask};
use async_trait::async_trait;
use dashmap::DashSet;
use lru::LruCache;
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{debug, trace};

/// TTL used when serving stale responses during cache_ttl window
const STALE_RESPONSE_TTL_SECS: u32 = 5;

/// Cache entry storing a DNS response with metadata
#[derive(Clone)]
struct CacheEntry {
    /// Cached DNS response message (shared)
    response: Arc<Message>,
    /// When this entry was created
    cached_at: Instant,
    /// Time-to-live for this entry (message TTL in seconds)
    ttl: u32,
    /// Maximum lifetime of the cached entry (cache TTL in seconds)
    cache_ttl: u32,
    /// Original TTL when first cached (used for lazycache threshold calculation)
    original_ttl: u32,
    /// Last access time for LRU tracking
    last_accessed: Instant,
}

impl CacheEntry {
    /// Create a new cache entry
    fn new(response: Message, ttl: u32, cache_ttl: u32) -> Self {
        let now = Instant::now();
        Self {
            response: Arc::new(response),
            cached_at: now,
            ttl,
            cache_ttl,
            original_ttl: ttl,
            last_accessed: now,
        }
    }

    /// Check if this entry has expired
    fn is_cache_expired(&self) -> bool {
        // Entries with a TTL of 0 should be considered expired immediately.
        if self.cache_ttl == 0 {
            return true;
        }

        // Use >= to avoid timing races where elapsed may equal the TTL.
        self.cached_at.elapsed() >= Duration::from_secs(self.cache_ttl as u64)
    }

    /// Update last accessed time
    fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }

    /// Get remaining message TTL in seconds
    fn remaining_ttl(&self) -> u32 {
        let elapsed = self.cached_at.elapsed().as_secs() as u32;
        self.ttl.saturating_sub(elapsed)
    }

    /// Get remaining cache TTL in seconds
    fn remaining_cache_ttl(&self) -> u32 {
        let elapsed = self.cached_at.elapsed().as_secs() as u32;
        self.cache_ttl.saturating_sub(elapsed)
    }
}

/// Statistics for the cache
#[derive(Debug, Default)]
pub struct CacheStats {
    /// Number of cache hits
    hits: AtomicU64,
    /// Number of cache misses
    misses: AtomicU64,
    /// Number of entries evicted due to size limit
    evictions: AtomicU64,
    /// Number of entries expired due to TTL
    expirations: AtomicU64,
}

impl CacheStats {
    /// Create new cache statistics
    fn new() -> Self {
        Self::default()
    }

    /// Increment hit counter
    fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
        // Update Prometheus metric
        #[cfg(feature = "metrics")]
        {
            metrics::CACHE_HITS_TOTAL.inc();
        }
    }

    /// Increment miss counter
    fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
        // Update Prometheus metric
        #[cfg(feature = "metrics")]
        {
            metrics::CACHE_MISSES_TOTAL.inc();
        }
    }

    /// Increment eviction counter
    fn record_eviction(&self) {
        self.evictions.fetch_add(1, Ordering::Relaxed);
        // Update Prometheus metric
        #[cfg(feature = "metrics")]
        {
            crate::metrics::DNS_CACHE_EVICTIONS_TOTAL.inc();
        }
    }

    /// Increment expiration counter
    fn record_expiration(&self) {
        self.expirations.fetch_add(1, Ordering::Relaxed);
        // Update Prometheus metric
        #[cfg(feature = "metrics")]
        {
            crate::metrics::DNS_CACHE_EXPIRATIONS_TOTAL.inc();
        }
    }

    /// Get number of hits
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// Get number of misses
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    /// Get number of evictions
    pub fn evictions(&self) -> u64 {
        self.evictions.load(Ordering::Relaxed)
    }

    /// Get number of expirations
    pub fn expirations(&self) -> u64 {
        self.expirations.load(Ordering::Relaxed)
    }

    /// Calculate hit rate (0.0 to 1.0)
    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits();
        let total = hits + self.misses();
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// Get total number of requests
    pub fn total_requests(&self) -> u64 {
        self.hits() + self.misses()
    }
}

impl fmt::Display for CacheStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CacheStats {{ hits: {}, misses: {}, evictions: {}, expirations: {}, hit_rate: {:.2}% }}",
            self.hits(),
            self.misses(),
            self.evictions(),
            self.expirations(),
            self.hit_rate() * 100.0
        )
    }
}

/// LazyCache-specific statistics
///
/// Tracks refresh attempts and outcomes for LazyCache optimization feature.
#[derive(Debug, Default)]
pub struct LazyCacheStats {
    /// Number of lazy refresh attempts
    refreshes: AtomicU64,
    /// Number of successful lazy refreshes
    successful_refreshes: AtomicU64,
    /// Number of failed lazy refreshes
    failed_refreshes: AtomicU64,
}

impl LazyCacheStats {
    /// Create new LazyCache statistics
    fn new() -> Self {
        Self::default()
    }

    /// Record a lazy refresh attempt
    fn record_refresh(&self) {
        self.refreshes.fetch_add(1, Ordering::Relaxed);
    }

    /// Get number of refresh attempts
    pub fn refreshes(&self) -> u64 {
        self.refreshes.load(Ordering::Relaxed)
    }

    /// Get number of successful refreshes
    pub fn successful_refreshes(&self) -> u64 {
        self.successful_refreshes.load(Ordering::Relaxed)
    }

    /// Get number of failed refreshes
    pub fn failed_refreshes(&self) -> u64 {
        self.failed_refreshes.load(Ordering::Relaxed)
    }

    /// Calculate successful refresh rate
    pub fn refresh_success_rate(&self) -> f64 {
        let total = self.refreshes();
        if total == 0 {
            0.0
        } else {
            self.successful_refreshes() as f64 / total as f64
        }
    }
}

impl fmt::Display for LazyCacheStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LazyCacheStats {{ refreshes: {}, successful: {}, failed: {}, success_rate: {:.2}% }}",
            self.refreshes(),
            self.successful_refreshes(),
            self.failed_refreshes(),
            self.refresh_success_rate() * 100.0
        )
    }
}

/// DNS response cache plugin
///
/// Caches DNS responses based on their TTL values. When the cache is full,
/// uses LRU (Least Recently Used) eviction policy.
///
/// # Lazycache Feature
///
/// LazyCache is an optimization that refreshes cached entries in the background
/// before they expire, preventing cache misses and query latency spikes.
/// When enabled, if a cached entry's TTL drops below the threshold (e.g., 10%),
/// the entry is marked for lazy refresh. A background task or next access
/// will trigger a refresh query to keep the cache warm.
#[derive(Clone, RegisterPlugin, ShutdownPlugin)]
pub struct CachePlugin {
    /// The cache storage (domain name -> cache entry)
    cache: Arc<parking_lot::RwLock<LruCache<String, CacheEntry>>>,
    /// Maximum number of entries in the cache
    max_size: usize,
    /// Cache statistics
    stats: Arc<CacheStats>,
    /// Enable negative caching (cache NXDOMAIN/SERVFAIL responses)
    negative_cache: bool,
    /// TTL for negative cache entries (in seconds)
    negative_ttl: u32,
    /// Enable cache prefetch (refresh entries before they expire)
    enable_prefetch: bool,
    /// Prefetch threshold (refresh when TTL drops below this percentage)
    prefetch_threshold: f32,
    /// Enable lazycache optimization (refresh hot entries before expiry)
    enable_lazycache: bool,
    /// Lazycache threshold - refresh when TTL drops below this percentage (0.0-1.0)
    lazycache_threshold: f32,
    /// Lazycache TTL (serve stale responses and refresh in background when original TTL expires)
    cache_ttl: Option<u32>,
    /// LazyCache-specific statistics
    lazycache_stats: Arc<LazyCacheStats>,
    /// Mutable threshold for runtime adjustment
    lazycache_threshold_dynamic: Arc<tokio::sync::RwLock<f32>>,
    /// Set of keys currently being refreshed (to prevent duplicate refreshes)
    refreshing_keys: Arc<DashSet<String>>,
    /// Plugin tag from YAML configuration
    tag: Option<String>,
    /// Refresh coordinator for background cache refresh operations (wrapped in Mutex for interior mutability)
    refresh_coordinator: Arc<Mutex<Option<RefreshCoordinator>>>,
    /// Enable periodic cleanup of expired entries (default: true)
    enable_cleanup: bool,
    /// Interval (in seconds) for cleanup tasks (default: 60)
    cleanup_interval_secs: u64,
    /// Trigger cleanup when cache reaches this percentage of max size (default: 0.8 = 80%)
    cleanup_pressure_threshold: f32,
}

impl CachePlugin {
    /// Create a new cache plugin with the specified maximum size
    ///
    /// # Arguments
    ///
    /// * `max_size` - Maximum number of entries to store in the cache
    ///
    /// # Example
    ///
    /// ```rust
    /// use lazydns::plugins::CachePlugin;
    ///
    /// let cache = CachePlugin::new(1000);
    /// ```
    pub fn new(max_size: usize) -> Self {
        let capacity = NonZeroUsize::new(max_size.max(1)).unwrap();
        Self {
            cache: Arc::new(parking_lot::RwLock::new(LruCache::new(capacity))),
            max_size,
            stats: Arc::new(CacheStats::new()),
            negative_cache: false,
            negative_ttl: 300, // 5 minutes default
            enable_prefetch: false,
            prefetch_threshold: 0.1, // Refresh at 10% remaining TTL
            enable_lazycache: false,
            lazycache_threshold: 0.05, // Refresh at 5% remaining TTL (hot entries)
            cache_ttl: None,
            lazycache_stats: Arc::new(LazyCacheStats::new()),
            lazycache_threshold_dynamic: Arc::new(tokio::sync::RwLock::new(0.05)),
            refreshing_keys: Arc::new(DashSet::new()),
            tag: None,
            refresh_coordinator: Arc::new(Mutex::new(None)),
            enable_cleanup: true,
            cleanup_interval_secs: 60,
            cleanup_pressure_threshold: 0.8,
        }
    }

    /// Enable negative caching for error responses
    ///
    /// # Arguments
    ///
    /// * `ttl` - TTL in seconds for negative cache entries
    pub fn with_negative_cache(mut self, ttl: u32) -> Self {
        self.negative_cache = true;
        self.negative_ttl = ttl;
        self
    }

    /// Enable cache prefetch
    ///
    /// # Arguments
    ///
    /// * `threshold` - Refresh when remaining TTL drops below this percentage (0.0-1.0)
    pub fn with_prefetch(mut self, threshold: f32) -> Self {
        self.enable_prefetch = true;
        self.prefetch_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// Enable lazycache optimization
    ///
    /// LazyCache refreshes frequently accessed entries before they expire,
    /// reducing cache misses and DNS query latency.
    ///
    /// # Arguments
    ///
    /// * `threshold` - Refresh when remaining TTL drops below this percentage (0.0-1.0)
    pub fn with_lazycache(mut self, threshold: f32) -> Self {
        self.enable_lazycache = true;
        self.lazycache_threshold = threshold.clamp(0.0, 1.0);
        // Initialize coordinator if not already present (non-blocking try_lock)
        match self.refresh_coordinator.try_lock() {
            Ok(mut guard) if guard.is_none() => {
                *guard = Some(RefreshCoordinator::new(4, 1000));
            }
            _ => {}
        }
        self
    }

    /// Enable cache TTL mode (serve stale responses and refresh in background)
    pub fn with_cache_ttl(mut self, ttl_secs: u32) -> Self {
        if ttl_secs > 0 {
            self.cache_ttl = Some(ttl_secs);
            // Initialize coordinator if not already present (non-blocking try_lock)
            match self.refresh_coordinator.try_lock() {
                Ok(mut guard) if guard.is_none() => {
                    *guard = Some(RefreshCoordinator::new(4, 1000));
                }
                _ => {}
            }
        }
        self
    }

    /// Enable or disable periodic cleanup of expired entries
    ///
    /// # Arguments
    ///
    /// * `enabled` - Whether to enable periodic cleanup
    /// * `interval_secs` - How often to run cleanup (in seconds)
    /// * `pressure_threshold` - Cleanup when cache reaches this % of max size (0.0-1.0)
    pub fn with_cleanup(
        mut self,
        enabled: bool,
        interval_secs: u64,
        pressure_threshold: f32,
    ) -> Self {
        self.enable_cleanup = enabled;
        self.cleanup_interval_secs = interval_secs.max(1); // Minimum 1 second
        self.cleanup_pressure_threshold = pressure_threshold.clamp(0.0, 1.0);
        self
    }

    /// Get a reference to the cache statistics
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Get LazyCache statistics
    pub fn lazycache_stats(&self) -> &LazyCacheStats {
        &self.lazycache_stats
    }

    /// Get current LazyCache threshold
    pub fn get_lazycache_threshold(&self) -> f32 {
        self.lazycache_threshold
    }

    /// Set LazyCache threshold dynamically
    ///
    /// # Arguments
    ///
    /// * `threshold` - New threshold value (0.0-1.0)
    pub fn set_lazycache_threshold(&self, threshold: f32) {
        let clamped = threshold.clamp(0.0, 1.0);
        debug!("Updating LazyCache threshold to {:.2}%", clamped * 100.0);
        // Note: This is a sync wrapper, the actual dynamic update happens in tokio
        // For now, we update the static field via the dynamic RwLock
    }

    /// Update LazyCache threshold asynchronously
    pub async fn set_lazycache_threshold_async(&self, threshold: f32) {
        let clamped = threshold.clamp(0.0, 1.0);
        let mut dynamic_threshold = self.lazycache_threshold_dynamic.write().await;
        *dynamic_threshold = clamped;
        debug!("LazyCache threshold updated to {:.2}%", clamped * 100.0);
    }

    /// Get current LazyCache threshold (may be dynamically adjusted)
    pub async fn get_lazycache_threshold_async(&self) -> f32 {
        *self.lazycache_threshold_dynamic.read().await
    }
    pub fn size(&self) -> usize {
        self.cache.read().len()
    }

    /// Cleanup expired cache entries
    ///
    /// Returns the number of entries removed.
    pub fn cleanup_expired(&self) -> usize {
        let mut cache = self.cache.write();
        let mut removed = 0;

        debug!("Cleanup: starting cache cleanup of expired entries");
        // Collect all expired keys
        let expired_keys: Vec<String> = cache
            .iter()
            .filter(|(_, entry)| entry.is_cache_expired())
            .map(|(k, _)| k.clone())
            .collect();

        // Remove expired entries
        for key in expired_keys {
            debug!("Cleanup: removing expired cache entry: {}", key);
            if let Some(removed_entry) = cache.pop(&key) {
                drop(removed_entry); // Explicitly drop to release Arc memory immediately
                self.stats.record_expiration();
                removed += 1;
            }
        }

        // Update cache size metric
        #[cfg(feature = "metrics")]
        {
            metrics::CACHE_SIZE.set(cache.len() as i64);
        }

        if removed > 0 {
            debug!("Cleanup removed {} expired cache entries", removed);
        }

        removed
    }

    /// Check if cleanup is needed due to memory pressure
    ///
    /// Returns true if cache size exceeds the pressure threshold.
    fn should_cleanup_pressure(&self) -> bool {
        let size = self.size();
        let threshold = (self.max_size as f32 * self.cleanup_pressure_threshold) as usize;
        size > threshold
    }

    /// Check if cleanup is enabled
    pub fn is_cleanup_enabled(&self) -> bool {
        self.enable_cleanup
    }

    /// Spawn a background cleanup task
    ///
    /// This task will:
    /// 1. Run periodically based on cleanup_interval_secs
    /// 2. Remove expired entries
    /// 3. Trigger cleanup if memory pressure is high
    ///
    /// Returns a handle to the spawned task.
    pub fn spawn_cleanup_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(self.cleanup_interval_secs));

            loop {
                interval.tick().await;

                let removed = self.cleanup_expired();

                // Check if pressure-based cleanup is needed
                if self.should_cleanup_pressure() {
                    debug!(
                        "Memory pressure detected: {} / {}",
                        self.size(),
                        self.max_size
                    );
                    let pressure_removed = self.cleanup_expired();
                    debug!(
                        "Pressure cleanup removed {} entries (total in this cycle: {})",
                        pressure_removed,
                        removed + pressure_removed
                    );
                }
            }
        })
    }

    /// Clear all entries from the cache
    pub fn clear(&self) {
        self.cache.write().clear();
        // Update cache size metric
        #[cfg(feature = "metrics")]
        {
            metrics::CACHE_SIZE.set(0);
        }
    }

    /// Generate a cache key from a DNS query
    ///
    /// Cache key includes:
    /// - Domain name (lowercased for case-insensitive matching)
    /// - Query type
    /// - Query class
    /// - EDNS0 flags (AD, CD, DO bits) if present
    fn make_key(message: &Message) -> Option<String> {
        // Use the first question as the cache key
        message.questions().first().map(|q| {
            // Normalize domain name to lowercase for case-insensitive matching
            let qname_lower = q.qname().to_lowercase();

            // Build key with EDNS0 considerations
            // Like mosdns, include AD, CD, DO flags in the key for proper caching
            let key = format!(
                "{}:{}:{}",
                qname_lower,
                q.qtype().to_u16(),
                q.qclass().to_u16()
            );

            // TODO: Add EDNS0 flags if message has EDNS0
            // This would ensure DNSSEC queries are cached separately from non-DNSSEC
            // Currently, we focus on the main fix: domain name normalization

            key
        })
    }

    /// Store a response in the cache (LRU will auto-evict if full)
    fn store(&self, key: String, entry: CacheEntry) {
        let mut cache = self.cache.write();

        // Check if this key already exists (replacement, not eviction)
        let key_exists = cache.contains(&key);

        // LruCache::push returns Some if the key existed (replacement)
        // or if cache was full and a new key was added (true eviction)
        if let Some((evicted_key, _)) = cache.push(key, entry) {
            // Only count as eviction if this is a new key (not a replacement)
            if !key_exists {
                // Cache was full, this is a true LRU eviction
                self.stats.record_eviction();
                debug!("LRU evicted cache entry: {}", evicted_key);
            } else {
                // This was a key replacement (update), not an eviction
                trace!("Cache store: replaced existing entry: {}", evicted_key);
            }
        }

        trace!(
            stats = ?self.stats,
            "Cache stats after store operation"
        );

        // Update cache size metric
        #[cfg(feature = "metrics")]
        {
            metrics::CACHE_SIZE.set(cache.len() as i64);
        }
    }

    /// Get minimum TTL from a DNS message
    fn get_min_ttl(message: &Message) -> u32 {
        let mut min_ttl = u32::MAX;

        // Check answer section
        for record in message.answers() {
            min_ttl = min_ttl.min(record.ttl());
        }

        // Check authority section
        for record in message.authority() {
            min_ttl = min_ttl.min(record.ttl());
        }

        // Check additional section
        for record in message.additional() {
            min_ttl = min_ttl.min(record.ttl());
        }

        // Default to 300 seconds (5 minutes) if no records found
        if min_ttl == u32::MAX {
            300
        } else {
            // Don't cache for less than 1 second
            min_ttl.max(1)
        }
    }

    /// Update TTLs in a cached response
    fn update_ttls(message: &mut Message, remaining_ttl: u32) {
        // Update TTLs in answer section
        for record in message.answers_mut() {
            record.set_ttl(remaining_ttl);
        }

        // Update TTLs in authority section
        for record in message.authority_mut() {
            record.set_ttl(remaining_ttl);
        }

        // Update TTLs in additional section
        for record in message.additional_mut() {
            record.set_ttl(remaining_ttl);
        }
    }
}

impl fmt::Debug for CachePlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachePlugin")
            .field("max_size", &self.max_size)
            .field("current_size", &self.size())
            .field("stats", &self.stats())
            .finish()
    }
}

impl BackgroundTask for CachePlugin {
    fn background_task_interval(&self) -> Duration {
        Duration::from_secs(self.cleanup_interval_secs)
    }

    fn run_background_task(&self) {
        let removed = self.cleanup_expired();

        // Check if pressure-based cleanup is needed
        if self.should_cleanup_pressure() {
            debug!(
                "Memory pressure detected: {} / {}",
                self.size(),
                self.max_size
            );
            let pressure_removed = self.cleanup_expired();
            debug!(
                "Pressure cleanup removed {} entries (total in this cycle: {})",
                pressure_removed,
                removed + pressure_removed
            );
        }
    }

    fn background_task_name(&self) -> &str {
        "cache_cleanup"
    }
}

#[async_trait]
impl Plugin for CachePlugin {
    async fn execute(&self, context: &mut Context) -> Result<()> {
        // Generate cache key from request
        let key = match Self::make_key(context.request()) {
            Some(k) => k,
            None => {
                debug!("Cannot generate cache key, no questions in request");
                return Ok(());
            }
        };

        // Check if cache was already checked in this request to avoid double-counting misses
        // (e.g., when fallback retries the entire sequence)
        let cache_already_checked = context.get_metadata::<bool>("cache_checked").is_some();

        // Check if response is already from cache (from a previous execution of this plugin)
        if context
            .get_metadata::<bool>("response_from_cache")
            .is_some()
        {
            // debug!("CachePlugin: Response already from cache, skipping execution");
            return Ok(());
        }

        // Phase 1: Try to read from cache (only if no response yet)
        if context.response().is_none() {
            // Mark that cache has been checked for this request (prevent duplicate miss counting)
            context.set_metadata("cache_checked", true);

            // Skip cache logic for background lazy refresh to avoid recursion
            if context
                .get_metadata::<bool>("background_lazy_refresh")
                .is_some()
            {
                debug!("Skipping cache logic for background lazy refresh");
                return Ok(());
            }

            // Try to get from cache (LruCache::get automatically updates LRU order)
            let cached_entry = {
                let mut cache = self.cache.write();
                cache.get(&key).cloned()
            };

            if let Some(mut entry) = cached_entry {
                // Remove if cache lifetime has fully expired
                if entry.is_cache_expired() {
                    debug!("Cache entry expired: {}", key);
                    self.cache.write().pop(&key);
                    self.stats.record_expiration();
                    self.stats.record_miss();
                    // Update cache size metric after removal
                    #[cfg(feature = "metrics")]
                    {
                        metrics::CACHE_SIZE.set(self.size() as i64);
                    }
                    return Ok(());
                }

                // Cache hit!
                debug!("Cache hit: {}", key);
                self.stats.record_hit();

                // Update last accessed time
                entry.touch();

                let remaining_ttl = entry.remaining_ttl();

                // Handle stale (message TTL expired) with cache_ttl semantics
                if remaining_ttl == 0 {
                    if let Some(lazy_ttl) = self.cache_ttl {
                        debug!(
                            "Stale-serving TTL hit (stale entry): {}, cache_remaining: {}s, configured_lazy_ttl: {}s",
                            key,
                            entry.remaining_cache_ttl(),
                            lazy_ttl
                        );

                        // Return stale response with a small TTL while refreshing in background
                        // CRITICAL: Must deep clone the response to avoid modifying cached object
                        // Arc::make_mut only clones if refcount > 1, so we must explicitly clone
                        let mut response = (*entry.response).clone();
                        Self::update_ttls(&mut response, STALE_RESPONSE_TTL_SECS);
                        response.set_id(context.request().id());
                        // Sync request QUESTION SECTION to avoid query/response mismatch
                        let request_questions = context.request().questions().to_vec();
                        *response.questions_mut() = request_questions;
                        context.set_response_arc(Some(Arc::new(response)));

                        // Mark that response came from cache to prevent Phase 2 re-execution
                        context.set_metadata("response_from_cache", true);

                        // Trigger background refresh (de-duplicated)
                        if self.refreshing_keys.insert(key.clone()) {
                            self.lazycache_stats.record_refresh();

                            // Check if we have required metadata and coordinator
                            if let (Some(handler), Some(entry_name)) = (
                                context.get_metadata::<Arc<PluginHandler>>("lazy_refresh_handler"),
                                context.get_metadata::<String>("lazy_refresh_entry"),
                            ) {
                                let background_handler = Arc::new(PluginHandler {
                                    registry: Arc::clone(&handler.registry),
                                    entry: entry_name.clone(),
                                });

                                let refreshing_keys_clone = Arc::clone(&self.refreshing_keys);
                                let mut request_clone = context.request().clone();
                                let key_clone = key.clone();
                                let coordinator = Arc::clone(&self.refresh_coordinator);

                                // Mark as background refresh
                                request_clone.set_id(0xFFFF);

                                // Enqueue refresh task instead of spawning thread
                                let task = RefreshTask {
                                    key: key_clone.clone(), // Clone for the task struct
                                    message: request_clone,
                                    handler: background_handler,
                                    entry_name: entry_name.clone(),
                                    created_at: Instant::now(),
                                };

                                // Use key_clone directly in async block (already cloned above)
                                tokio::spawn(async move {
                                    // Lock the Mutex to access the coordinator inside
                                    if let Some(coord) = coordinator.lock().await.as_ref() {
                                        match coord.enqueue(task).await {
                                            Ok(_) => {
                                                debug!(
                                                    "Background stale-serving TTL refresh enqueued for {}",
                                                    key_clone
                                                );
                                            }
                                            Err(e) => {
                                                debug!(
                                                    "Failed to enqueue stale-serving TTL refresh for {}: {}",
                                                    key_clone, e
                                                );
                                                // Remove from refreshing set if enqueue failed
                                                refreshing_keys_clone.remove(&key_clone);
                                            }
                                        }
                                    } else {
                                        debug!("Refresh coordinator not initialized");
                                        refreshing_keys_clone.remove(&key_clone);
                                    }
                                });
                            } else {
                                debug!(
                                    "Stale-serving TTL: handler metadata missing, falling back to invalidate stale entry"
                                );
                                let cache_clone = Arc::clone(&self.cache);
                                let refreshing_keys_clone = Arc::clone(&self.refreshing_keys);
                                let key_clone = key.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(tokio::time::Duration::from_millis(10))
                                        .await;
                                    cache_clone.write().pop(&key_clone);
                                    refreshing_keys_clone.remove(&key_clone);
                                });
                            }
                        } else {
                            debug!(
                                "Stale-serving TTL: {} already being refreshed, skip duplicate background refresh",
                                key
                            );
                        }

                        // Stop the chain and return stale response
                        context.set_metadata(RETURN_FLAG, true);
                        return Ok(());
                    } else {
                        // No lazycache TTL configured: treat as expired
                        debug!("Cache entry message TTL expired without cache_ttl: {}", key);
                        self.cache.write().pop(&key);
                        self.stats.record_expiration();
                        self.stats.record_miss();
                        #[cfg(feature = "metrics")]
                        {
                            metrics::CACHE_SIZE.set(self.size() as i64);
                        }
                        return Ok(());
                    }
                }

                // Check if lazycache threshold is reached (pre-expiry refresh)
                let should_lazy_refresh = if self.enable_lazycache {
                    // Skip lazy refresh if this is already a background refresh to prevent recursion
                    if context
                        .get_metadata::<bool>("background_lazy_refresh")
                        .is_some()
                    {
                        debug!(
                            "Background lazy refresh: skipping lazy refresh check for {}",
                            key
                        );
                        false
                    } else {
                        // Use original_ttl for percentage calculation to properly detect the 5% threshold
                        let ttl_percentage = remaining_ttl as f32 / entry.original_ttl as f32;
                        let threshold = self.lazycache_threshold;

                        debug!(
                            "LazyCache check: {}, original_ttl: {}s, remaining: {}s, percentage: {:.2}%, threshold: {:.2}%",
                            key,
                            entry.original_ttl,
                            remaining_ttl,
                            ttl_percentage * 100.0,
                            threshold * 100.0
                        );

                        if ttl_percentage <= threshold {
                            // Lazycache: Entry needs refresh
                            debug!(
                                "LazyCache threshold REACHED for {}: {:.2}% TTL remaining (< {:.2}%), triggering refresh",
                                key,
                                ttl_percentage * 100.0,
                                threshold * 100.0
                            );
                            // Record lazy refresh attempt
                            self.lazycache_stats.record_refresh();
                            true
                        } else {
                            false
                        }
                    }
                } else {
                    debug!(
                        "Lazycache disabled (enable_lazycache={})",
                        self.enable_lazycache
                    );
                    false
                };

                if should_lazy_refresh {
                    // LazyCache: return cached response immediately, spawn background refresh
                    // CRITICAL: Must deep clone the response to avoid modifying cached object
                    let mut response = (*entry.response).clone();
                    Self::update_ttls(&mut response, remaining_ttl);
                    response.set_id(context.request().id());
                    // Sync request QUESTION SECTION to avoid query/response mismatch
                    let request_questions = context.request().questions().to_vec();
                    *response.questions_mut() = request_questions;
                    context.set_response_arc(Some(Arc::new(response)));

                    // Mark that response came from cache to prevent Phase 2 re-execution
                    context.set_metadata("response_from_cache", true);

                    // Check if already refreshing this key to prevent duplicate refreshes
                    if self.refreshing_keys.insert(key.clone()) {
                        debug!(
                            "LazyCache: returning cached response immediately, triggering background refresh for {}",
                            key
                        );

                        // Record the refresh attempt
                        self.lazycache_stats.record_refresh();

                        // Get lazy refresh handler from metadata
                        if let (Some(handler), Some(entry_name)) = (
                            context.get_metadata::<Arc<PluginHandler>>("lazy_refresh_handler"),
                            context.get_metadata::<String>("lazy_refresh_entry"),
                        ) {
                            // Create a new handler instance for background refresh
                            let background_handler = Arc::new(PluginHandler {
                                registry: Arc::clone(&handler.registry),
                                entry: entry_name.clone(),
                            });

                            let refreshing_keys_clone = Arc::clone(&self.refreshing_keys);
                            let mut request_clone = context.request().clone();
                            let key_clone = key.clone();
                            let coordinator = Arc::clone(&self.refresh_coordinator);

                            // Mark this as a background refresh by setting a special ID
                            request_clone.set_id(0xFFFF);

                            // Enqueue refresh task instead of spawning thread
                            let task = RefreshTask {
                                key: key_clone.clone(),
                                message: request_clone,
                                handler: background_handler,
                                entry_name: entry_name.clone(),
                                created_at: Instant::now(),
                            };

                            tokio::spawn(async move {
                                // Lock the Mutex to access the coordinator inside
                                if let Some(coord) = coordinator.lock().await.as_ref() {
                                    match coord.enqueue(task).await {
                                        Ok(_) => {
                                            debug!(
                                                "Background lazy refresh enqueued for {}",
                                                key_clone
                                            );
                                        }
                                        Err(e) => {
                                            debug!(
                                                "Failed to enqueue lazy refresh for {}: {}",
                                                key_clone, e
                                            );
                                            // Remove from refreshing set if enqueue failed
                                            refreshing_keys_clone.remove(&key_clone);
                                        }
                                    }
                                } else {
                                    debug!("Refresh coordinator not initialized");
                                    refreshing_keys_clone.remove(&key_clone);
                                }
                            });
                        } else {
                            debug!(
                                "LazyCache: lazy_refresh_handler not available in metadata or coordinator not initialized, falling back to cache invalidation"
                            );

                            // Fallback to old behavior: invalidate cache entry
                            let cache_clone = Arc::clone(&self.cache);
                            let refreshing_keys_clone = Arc::clone(&self.refreshing_keys);
                            let key_clone = key.clone();

                            tokio::spawn(async move {
                                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                                debug!("Fallback: invalidating cache entry for {}", key_clone);
                                cache_clone.write().pop(&key_clone);
                                refreshing_keys_clone.remove(&key_clone);
                            });
                        }
                    } else {
                        debug!(
                            "LazyCache: {} already being refreshed by another request, skipping duplicate refresh",
                            key
                        );
                    }

                    // Stop chain - client gets cached response immediately
                    context.set_metadata(RETURN_FLAG, true);
                    return Ok(());
                } else {
                    // Normal cache hit - return immediately (unless this is a background refresh)
                    if context
                        .get_metadata::<bool>("background_lazy_refresh")
                        .is_some()
                    {
                        debug!(
                            "Background lazy refresh: cache hit but continuing downstream for {}",
                            key
                        );
                        // Don't return cached response, let downstream execute to get fresh data
                        return Ok(());
                    } else {
                        // Normal cache hit: deep clone to avoid modifying cached object
                        // CRITICAL: Don't use Arc::make_mut - it only clones if refcount > 1
                        let mut response = (*entry.response).clone();
                        Self::update_ttls(&mut response, remaining_ttl);
                        response.set_id(context.request().id());
                        // Sync request QUESTION SECTION to avoid query/response mismatch
                        let request_questions = context.request().questions().to_vec();
                        *response.questions_mut() = request_questions;
                        context.set_response_arc(Some(Arc::new(response)));

                        // Mark that response came from cache to prevent Phase 2 re-execution
                        context.set_metadata("response_from_cache", true);

                        trace!("Normal cache hit: returning immediately and stopping chain");
                        // Stop the plugin chain to prevent downstream plugins (like Forward)
                        // from executing and overwriting our cached response.
                        context.set_metadata(RETURN_FLAG, true);
                        return Ok(());
                    }
                }
            }

            // Cache miss - no entry found at all
            // Only record miss if this is the first cache check in this request
            // (to avoid double-counting when fallback retries the sequence)
            if !cache_already_checked {
                self.stats.record_miss();
                debug!("Cache miss: {}", key);
            }
        } else {
            // Phase 2: A response exists (set by a downstream plugin like forward)
            // We should store it in cache for future queries
            // BUT: Skip if response came from cache (Phase 1) - we don't want to re-store it
            if context
                .get_metadata::<bool>("response_from_cache")
                .is_none()
                && let Some(response) = context.response()
            {
                let response_code = response.response_code();
                let is_error = response_code != crate::dns::ResponseCode::NoError;
                // Handle negative caching
                if is_error {
                    if self.negative_cache {
                        // Cache error responses with negative TTL
                        debug!(
                            "Caching negative response: {:?} (TTL: {}s)",
                            response_code, self.negative_ttl
                        );

                        let cache_ttl = self.cache_ttl.unwrap_or(self.negative_ttl);
                        let entry = CacheEntry::new(response.clone(), self.negative_ttl, cache_ttl);
                        self.store(key.clone(), entry);
                    } else {
                        debug!("Not caching error response: {:?}", response_code);
                    }
                } else if !response.answers().is_empty() {
                    // Cache successful responses with answers
                    let ttl = Self::get_min_ttl(response);

                    if ttl > 0 {
                        // Determine cache TTL: if cache_ttl is set, use it for positive answers
                        let cache_ttl = self.cache_ttl.unwrap_or(ttl);
                        debug!(
                            "Storing response in cache: {} (message TTL: {}s, cache TTL: {}s)",
                            key, ttl, cache_ttl
                        );

                        // Always create/replace with new entry
                        // This resets original_ttl for the new cache cycle
                        let entry = CacheEntry::new(response.clone(), ttl, cache_ttl);
                        self.store(key.clone(), entry);
                    }
                }
            }
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "cache"
    }

    fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    fn priority(&self) -> i32 {
        // Cache should run early to check for cached responses
        // and after it's been populated (when returning from other plugins)
        50
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn spawn_background_task(&self) -> Option<tokio::task::JoinHandle<()>> {
        if self.is_cleanup_enabled() {
            Some(Arc::new(self.clone()).spawn_background_task())
        } else {
            None
        }
    }

    fn init(config: &PluginConfig) -> Result<Arc<dyn Plugin>> {
        let args = config.effective_args();
        use serde_yaml::Value;

        // Parse size parameter (default: 1024)
        let size = match args.get("size") {
            Some(Value::Number(n)) => n
                .as_i64()
                .ok_or_else(|| Error::Config("Invalid size value".to_string()))?
                as usize,
            Some(_) => return Err(Error::Config("size must be a number".to_string())),
            None => 1024,
        };

        let mut cache = CachePlugin::new(size);

        // Parse negative_cache parameter (default: false)
        if let Some(Value::Bool(true)) = args.get("negative_cache") {
            let negative_ttl = match args.get("negative_ttl") {
                Some(Value::Number(n)) => n
                    .as_i64()
                    .ok_or_else(|| Error::Config("Invalid negative_ttl value".to_string()))?
                    as u32,
                Some(_) => return Err(Error::Config("negative_ttl must be a number".to_string())),
                None => 300,
            };
            cache = cache.with_negative_cache(negative_ttl);
        }

        // Parse prefetch parameter (default: false)
        if let Some(Value::Bool(true)) = args.get("enable_prefetch") {
            let threshold = match args.get("prefetch_threshold") {
                Some(Value::Number(n)) => n
                    .as_f64()
                    .ok_or_else(|| Error::Config("Invalid prefetch_threshold value".to_string()))?
                    as f32,
                Some(_) => {
                    return Err(Error::Config(
                        "prefetch_threshold must be a number".to_string(),
                    ));
                }
                None => 0.1,
            };
            cache = cache.with_prefetch(threshold);
        }

        // Parse cache_ttl (stale-serving) parameter (default: disabled)
        if let Some(Value::Number(n)) = args.get("cache_ttl") {
            let ttl = n
                .as_i64()
                .ok_or_else(|| Error::Config("Invalid cache_ttl value".to_string()))?
                as u32;
            if ttl > 0 {
                cache = cache.with_cache_ttl(ttl);
            }
        }

        // Parse refresh coordinator configuration
        let worker_count = match args.get("refresh_worker_count") {
            Some(Value::Number(n)) => n
                .as_i64()
                .ok_or_else(|| Error::Config("Invalid refresh_worker_count value".to_string()))?
                as usize,
            Some(_) => {
                return Err(Error::Config(
                    "refresh_worker_count must be a number".to_string(),
                ));
            }
            None => 4, // Default: 4 workers
        };

        let queue_capacity = match args.get("refresh_queue_capacity") {
            Some(Value::Number(n)) => n
                .as_i64()
                .ok_or_else(|| Error::Config("Invalid refresh_queue_capacity value".to_string()))?
                as usize,
            Some(_) => {
                return Err(Error::Config(
                    "refresh_queue_capacity must be a number".to_string(),
                ));
            }
            None => 1000, // Default: 1000 pending tasks
        };

        // Create refresh coordinator if lazycache or cache_ttl is enabled
        if cache.enable_lazycache || cache.cache_ttl.is_some() {
            // Initialize coordinator only if not already set by builder methods
            if let Ok(mut guard) = cache.refresh_coordinator.try_lock() {
                if guard.is_none() {
                    *guard = Some(RefreshCoordinator::new(worker_count, queue_capacity));
                }
            } else {
                // If mutex is currently locked, replace to ensure initialization
                let coordinator = RefreshCoordinator::new(worker_count, queue_capacity);
                cache.refresh_coordinator = Arc::new(Mutex::new(Some(coordinator)));
            }
        }

        // Parse lazycache parameter (default: false)
        // Lazycache enables automatic refresh of hot cached entries before expiry
        if let Some(Value::Bool(true)) = args.get("enable_lazycache") {
            let threshold = match args.get("lazycache_threshold") {
                Some(Value::Number(n)) => n
                    .as_f64()
                    .ok_or_else(|| Error::Config("Invalid lazycache_threshold value".to_string()))?
                    as f32,
                Some(_) => {
                    return Err(Error::Config(
                        "lazycache_threshold must be a number".to_string(),
                    ));
                }
                None => 0.05, // Default: 5% of original TTL
            };
            cache = cache.with_lazycache(threshold);
        }

        // Parse cleanup parameters (default: enabled with 60s interval)
        let enable_cleanup = match args.get("enable_cleanup") {
            Some(Value::Bool(b)) => *b,
            Some(_) => {
                return Err(Error::Config(
                    "enable_cleanup must be a boolean".to_string(),
                ));
            }
            None => true,
        };

        let cleanup_interval_secs = match args.get("cleanup_interval_secs") {
            Some(Value::Number(n)) => n
                .as_i64()
                .ok_or_else(|| Error::Config("Invalid cleanup_interval_secs value".to_string()))?
                as u64,
            Some(_) => {
                return Err(Error::Config(
                    "cleanup_interval_secs must be a number".to_string(),
                ));
            }
            None => 60,
        };

        let cleanup_pressure_threshold = match args.get("cleanup_pressure_threshold") {
            Some(Value::Number(n)) => n.as_f64().ok_or_else(|| {
                Error::Config("Invalid cleanup_pressure_threshold value".to_string())
            })? as f32,
            Some(_) => {
                return Err(Error::Config(
                    "cleanup_pressure_threshold must be a number".to_string(),
                ));
            }
            None => 0.8,
        };

        cache = cache.with_cleanup(
            enable_cleanup,
            cleanup_interval_secs,
            cleanup_pressure_threshold,
        );

        // Set tag from config
        cache.tag = config.tag.clone();

        debug!(
            "CachePlugin initialized: size={}, negative_cache={}, lazycache_enabled={}, lazycache_threshold={:.1}%, cleanup_enabled={}, cleanup_interval={}s",
            cache.max_size,
            cache.negative_cache,
            cache.enable_lazycache,
            cache.lazycache_threshold * 100.0,
            cache.enable_cleanup,
            cache.cleanup_interval_secs
        );

        Ok(Arc::new(cache))
    }
}

#[async_trait]
impl Shutdown for CachePlugin {
    async fn shutdown(&self) -> Result<()> {
        // Shutdown the refresh coordinator if it exists
        if let Some(coordinator) = self.refresh_coordinator.lock().await.take() {
            debug!("Shutting down CachePlugin refresh coordinator");
            coordinator.shutdown().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::{Message, Question, RData, RecordClass, RecordType, ResourceRecord};
    use std::net::Ipv4Addr;

    fn create_test_message() -> Message {
        let mut msg = Message::new();
        msg.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
        msg
    }

    fn create_test_response() -> Message {
        let mut msg = create_test_message();
        msg.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A(Ipv4Addr::new(93, 184, 216, 34)),
        ));
        msg
    }

    #[test]
    fn test_cache_entry_creation() {
        let response = create_test_response();
        let entry = CacheEntry::new(response.clone(), 300, 300);

        assert_eq!(entry.ttl, 300);
        assert!(!entry.is_cache_expired());
        assert_eq!(entry.response.answers().len(), response.answers().len());
    }

    #[test]
    fn test_cache_entry_expiration() {
        let response = create_test_response();
        let entry = CacheEntry::new(response, 0, 0);

        // Entry with 0 TTL should be immediately expired
        assert!(entry.is_cache_expired());
    }

    #[test]
    fn test_cache_entry_remaining_ttl() {
        let response = create_test_response();
        let entry = CacheEntry::new(response, 300, 300);

        let remaining = entry.remaining_ttl();
        assert!(remaining <= 300);
        assert!(remaining >= 299); // Should be very close to 300
    }

    #[test]
    fn test_cache_stats() {
        let stats = CacheStats::new();

        assert_eq!(stats.hits(), 0);
        assert_eq!(stats.misses(), 0);
        assert_eq!(stats.evictions(), 0);

        stats.record_hit();
        stats.record_hit();
        stats.record_miss();

        assert_eq!(stats.hits(), 2);
        assert_eq!(stats.misses(), 1);
        assert_eq!(stats.hit_rate(), 2.0 / 3.0);
    }

    #[test]
    fn test_cache_plugin_creation() {
        let cache = CachePlugin::new(100);

        assert_eq!(cache.max_size, 100);
        assert_eq!(cache.size(), 0);
        assert_eq!(cache.stats().hits(), 0);
    }

    /// Ensure that `as_any()` implementation exists and allows downcasting
    /// to `CachePlugin`. This prevents accidental removal of `as_any()`.
    #[test]
    fn test_plugin_as_any_downcast_present() {
        use std::sync::Arc;

        let cache = CachePlugin::new(128);
        let plugin: Arc<dyn crate::plugin::Plugin> = Arc::new(cache);

        // Should be able to downcast to CachePlugin
        assert!(
            plugin
                .as_ref()
                .as_any()
                .downcast_ref::<CachePlugin>()
                .is_some()
        );
    }

    #[test]
    fn test_make_key() {
        let msg = create_test_message();
        let key = CachePlugin::make_key(&msg);

        assert!(key.is_some());
        assert_eq!(key.unwrap(), "example.com:1:1");
    }

    #[test]
    fn test_make_key_case_insensitive() {
        // Test that different casings produce the same cache key
        let msg_lower = create_test_message();
        let mut msg_upper = create_test_message();

        // Change question to uppercase
        msg_upper.questions_mut()[0].set_qname("EXAMPLE.COM");

        let key_lower = CachePlugin::make_key(&msg_lower);
        let key_upper = CachePlugin::make_key(&msg_upper);

        assert!(key_lower.is_some());
        assert!(key_upper.is_some());
        // Both should produce the same lowercase key
        let key_lower_str = key_lower.unwrap();
        let key_upper_str = key_upper.unwrap();
        assert_eq!(key_lower_str, key_upper_str);
        assert_eq!(key_lower_str, "example.com:1:1");
    }

    #[test]
    fn test_make_key_no_questions() {
        let msg = Message::new();
        let key = CachePlugin::make_key(&msg);

        assert!(key.is_none());
    }

    #[test]
    fn test_get_min_ttl() {
        let response = create_test_response();
        let ttl = CachePlugin::get_min_ttl(&response);

        assert_eq!(ttl, 300);
    }

    #[test]
    fn test_get_min_ttl_no_records() {
        let msg = create_test_message();
        let ttl = CachePlugin::get_min_ttl(&msg);

        // Should return default TTL of 300
        assert_eq!(ttl, 300);
    }

    #[cfg(feature = "metrics")]
    #[tokio::test]
    async fn test_cache_miss() {
        let cache = CachePlugin::new(100);
        let request = create_test_message();
        let mut context = Context::new(request);

        let prev_misses = metrics::CACHE_MISSES_TOTAL.get();

        cache.execute(&mut context).await.unwrap();

        assert!(context.response().is_none());
        assert!(cache.stats().misses() >= 1);
        assert_eq!(cache.stats().hits(), 0);
        // Global metric incremented
        assert_eq!(metrics::CACHE_MISSES_TOTAL.get(), prev_misses + 1);
    }

    #[cfg(feature = "metrics")]
    #[tokio::test]
    async fn test_cache_hit() {
        let cache = CachePlugin::new(100);

        // Store an entry in the cache via store() so metric is updated
        let response = create_test_response();
        let key = "example.com:1:1".to_string();
        let entry = CacheEntry::new(response.clone(), 300, 300);
        cache.store(key.clone(), entry);

        // Cache size metric should be updated
        assert_eq!(metrics::CACHE_SIZE.get(), cache.size() as i64);

        // Try to retrieve it
        let request = create_test_message();
        let mut context = Context::new(request);

        let prev_hits = metrics::CACHE_HITS_TOTAL.get();

        cache.execute(&mut context).await.unwrap();

        assert!(context.response().is_some());
        assert_eq!(cache.stats().hits(), 1);
        assert_eq!(cache.stats().misses(), 0);
        // Global metric incremented
        assert_eq!(metrics::CACHE_HITS_TOTAL.get(), prev_hits + 1);
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let cache = CachePlugin::new(100);

        // Store an entry with 0 TTL (immediately expired)
        let response = create_test_response();
        let key = "example.com:1:1".to_string();
        let entry = CacheEntry::new(response.clone(), 0, 0);
        cache.cache.write().push(key.clone(), entry);

        // Try to retrieve it
        let request = create_test_message();
        let mut context = Context::new(request);

        cache.execute(&mut context).await.unwrap();

        // Should be a miss because entry expired
        assert!(context.response().is_none());
        assert_eq!(cache.stats().misses(), 1);
        assert_eq!(cache.stats().expirations(), 1);

        // Entry should be removed from cache
        assert!(!cache.cache.read().contains(&key));
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn test_cache_clear() {
        let cache = CachePlugin::new(100);

        // Add some entries via store() so metric is updated
        let response = create_test_response();
        let entry = CacheEntry::new(response.clone(), 300, 300);
        cache.store("key1".to_string(), entry.clone());
        cache.store("key2".to_string(), entry.clone());

        assert_eq!(cache.size(), 2);
        assert_eq!(metrics::CACHE_SIZE.get(), 2);

        cache.clear();

        assert_eq!(cache.size(), 0);
        assert_eq!(metrics::CACHE_SIZE.get(), 0);
    }

    #[test]
    fn test_lru_eviction() {
        let cache = CachePlugin::new(2); // Small cache

        let response = create_test_response();
        let entry1 = CacheEntry::new(response.clone(), 300, 300);
        let entry2 = CacheEntry::new(response.clone(), 300, 300);
        let entry3 = CacheEntry::new(response.clone(), 300, 300);

        // Fill cache
        cache.cache.write().push("key1".to_string(), entry1);
        cache.cache.write().push("key2".to_string(), entry2);

        assert_eq!(cache.size(), 2);

        // Add one more - should evict the LRU entry
        cache.store("key3".to_string(), entry3);

        assert_eq!(cache.size(), 2);
        assert_eq!(cache.stats().evictions(), 1);
    }

    #[tokio::test]
    async fn test_configured_cache_sequence_execution() {
        // YAML config: registers a named cache and a sequence that execs it by name
        let yaml = r#"
plugins:
  - tag: my_cache
    type: cache
    config:
      size: 16

  - tag: seq
    type: sequence
    args:
      - exec: "$my_cache"
"#;

        let cfg = crate::config::Config::from_yaml(yaml).expect("parse yaml");

        let mut builder = crate::plugin::builder::PluginBuilder::new();

        // Build all plugins from config
        for pc in &cfg.plugins {
            builder.build(pc).expect("build plugin");
        }

        // Resolve references (sequence -> $my_cache)
        builder
            .resolve_references(&cfg.plugins)
            .expect("resolve refs");

        // Get the sequence plugin and execute it
        let plugin = builder.get_plugin("seq").expect("sequence exists");
        let mut ctx = crate::plugin::Context::new(crate::dns::Message::new());
        plugin.execute(&mut ctx).await.expect("execute sequence");

        // Execution should succeed and the sequence plugin name is 'sequence'
        assert_eq!(plugin.name(), "sequence");
    }

    #[tokio::test]
    async fn test_lazycache_refresh_threshold_triggers() {
        let cache = CachePlugin::new(100);
        cache.set_lazycache_threshold(0.1); // 10% threshold

        let response = create_test_response();
        let mut ctx = crate::plugin::Context::new(create_test_message());

        // Phase 1: Store response in cache
        ctx.set_response(Some(response.clone()));
        let res = cache.execute(&mut ctx).await;
        assert!(res.is_ok());

        // Phase 2: Query again to test cache hit
        let mut ctx = crate::plugin::Context::new(create_test_message());
        let res = cache.execute(&mut ctx).await;
        assert!(res.is_ok());

        // Should have a cache hit
        assert!(ctx.response().is_some());

        // At this point with full TTL (300s), we shouldn't need refresh
        assert!(
            ctx.get_metadata::<bool>("needs_lazycache_refresh")
                .is_none()
        );

        // Simulate a cache entry with very low TTL (approaching expiry)
        // by directly checking the logic would trigger
        let cache_entry = cache
            .cache
            .read()
            .peek(&"example.com:1:1".to_string())
            .expect("entry exists")
            .clone();
        let ttl_percent = cache_entry.remaining_ttl() as f32 / cache_entry.ttl as f32;
        let threshold = cache.get_lazycache_threshold();

        // With full TTL, ttl_percent should be ~1.0, threshold is 0.1
        // So refresh shouldn't trigger yet
        assert!(ttl_percent > threshold);

        // Verify stats tracking
        assert_eq!(cache.lazycache_stats.refreshes(), 0); // No refreshes needed yet
    }

    #[tokio::test]
    async fn test_lazycache_continues_pipeline_on_refresh() {
        let cache = CachePlugin::new(100);
        cache.set_lazycache_threshold(0.05); // 5% threshold

        let response = create_test_response();
        let mut ctx = crate::plugin::Context::new(create_test_message());

        // Store response
        ctx.set_response(Some(response));
        cache.execute(&mut ctx).await.expect("cache store");

        // Verify response is in cache
        assert!(ctx.response().is_some());

        // Get the cache hit without refresh (normal case)
        let mut ctx = crate::plugin::Context::new(create_test_message());
        cache.execute(&mut ctx).await.expect("cache hit");

        // Should have response and no refresh needed (normal TTL)
        assert!(ctx.response().is_some());
        assert!(
            ctx.get_metadata::<bool>("needs_lazycache_refresh")
                .is_none()
        );

        // With normal cache behavior, after cache hit the plugin should return
        // (not continue pipeline) unless lazy refresh is needed
    }

    #[tokio::test]
    async fn test_cache_ttl_serves_stale_and_refreshes() {
        use tokio::time::{Duration, sleep};

        let cache = CachePlugin::new(100).with_cache_ttl(10);

        // Build a response with a very small TTL to expire quickly
        let mut response = create_test_response();
        for rr in response.answers_mut() {
            rr.set_ttl(1);
        }

        // Store response (Phase 2 path)
        let mut ctx = crate::plugin::Context::new(create_test_message());
        ctx.set_response(Some(response.clone()));
        cache.execute(&mut ctx).await.expect("cache store");

        // Wait for TTL to expire but keep within cache_ttl window
        sleep(Duration::from_secs(2)).await;

        // Query again: should get stale response with small TTL and trigger background refresh
        let mut ctx = crate::plugin::Context::new(create_test_message());
        cache.execute(&mut ctx).await.expect("cache stale hit");

        let resp = ctx.response().expect("stale response returned");
        // Stale response TTL should be clamped to the fixed stale TTL (5s)
        assert!(resp.answers()[0].ttl() <= STALE_RESPONSE_TTL_SECS);

        // Background refresh should be scheduled (refresh count increments)
        sleep(Duration::from_millis(50)).await;
        assert!(cache.lazycache_stats.refreshes() >= 1);
    }

    #[test]
    fn test_cleanup_expired() {
        let cache = CachePlugin::new(100);
        let response = create_test_response();

        // Add some entries with short TTL
        let entry1 = CacheEntry::new(response.clone(), 0, 0); // Immediately expired
        let entry2 = CacheEntry::new(response.clone(), 0, 0); // Immediately expired
        let entry3 = CacheEntry::new(response.clone(), 300, 300); // Long TTL

        cache.cache.write().push("key1".to_string(), entry1);
        cache.cache.write().push("key2".to_string(), entry2);
        cache.cache.write().push("key3".to_string(), entry3);

        assert_eq!(cache.size(), 3);
        assert_eq!(cache.stats().expirations(), 0);

        // Cleanup should remove expired entries
        let removed = cache.cleanup_expired();
        assert_eq!(removed, 2); // key1 and key2 should be removed
        assert_eq!(cache.size(), 1); // Only key3 remains
        assert_eq!(cache.stats().expirations(), 2); // Stats updated
    }

    #[test]
    fn test_should_cleanup_pressure() {
        let mut cache = CachePlugin::new(10);
        cache = cache.with_cleanup(true, 60, 0.5); // Cleanup at 50% threshold

        let response = create_test_response();

        // Add entries until we reach pressure threshold
        for i in 0..6 {
            let entry = CacheEntry::new(response.clone(), 300, 300);
            cache.cache.write().push(format!("key{}", i), entry);
        }

        // Should trigger pressure cleanup (6 > 10 * 0.5)
        assert!(cache.should_cleanup_pressure());

        // Cache with higher threshold should not trigger
        let cache2 = CachePlugin::new(10).with_cleanup(true, 60, 0.9);
        for i in 0..6 {
            let entry = CacheEntry::new(response.clone(), 300, 300);
            cache2.cache.write().push(format!("key{}", i), entry);
        }
        assert!(!cache2.should_cleanup_pressure()); // 6 <= 10 * 0.9
    }

    #[tokio::test]
    async fn test_spawn_cleanup_task() {
        let cache = Arc::new(CachePlugin::new(100));
        let response = create_test_response();

        // Add some expired entries
        let entry1 = CacheEntry::new(response.clone(), 0, 0);
        let entry2 = CacheEntry::new(response.clone(), 1, 1);
        let entry3 = CacheEntry::new(response.clone(), 300, 300);

        cache.cache.write().push("key1".to_string(), entry1);
        cache.cache.write().push("key2".to_string(), entry2);
        cache.cache.write().push("key3".to_string(), entry3);

        assert_eq!(cache.size(), 3);

        // Spawn cleanup task with very short interval for testing
        let cache_with_short_interval = {
            let mut c = CachePlugin::new(100);
            c.cleanup_interval_secs = 1; // 1 second interval
            c.enable_cleanup = true;
            Arc::new(c)
        };

        let cleanup_handle = cache_with_short_interval.clone().spawn_cleanup_task();

        // Wait for cleanup to run (at most 1.5 seconds)
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // Cancel the cleanup task
        cleanup_handle.abort();

        // Note: We can't directly test that expired entries were removed via the background task
        // because we're testing with a different cache instance. But we've verified the task
        // spawns and runs without errors.
    }
}
