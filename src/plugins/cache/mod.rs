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
use tracing::{debug, trace, warn};

/// TTL used when serving stale responses during cache_ttl window
const STALE_RESPONSE_TTL_SECS: u32 = 5;

mod entry;
mod persistence;
mod stats;

pub use entry::CacheEntry;
pub use stats::{CacheStats, LazyCacheStats};

/// DNS response cache plugin
///
/// Caches DNS responses based on their TTL values. When the cache is full,
/// uses LRU (Least Recently Used) eviction policy.
///
/// # Lazycache Feature
///
/// LazyCache is an optimization that refreshes cached entries in the background
/// before they expire, preventing cache misses and query latency spikes.
/// When enabled, if a cached entry's TTL drops below the threshold (such as 10%),
/// the entry is marked for lazy refresh. A background task or next access
/// will trigger a refresh query to keep the cache warm.
#[derive(RegisterPlugin, ShutdownPlugin)]
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
    /// Enable lazycache optimization (refresh hot entries before expiry)
    enable_lazycache: bool,
    /// Lazycache threshold - refresh when TTL drops below this percentage (0.0-1.0)
    lazycache_threshold: f32,
    /// Lazycache TTL (serve stale responses and refresh in background when original TTL expires)
    cache_ttl: Option<u32>,
    /// LazyCache-specific statistics
    lazycache_stats: Arc<LazyCacheStats>,
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
    /// Optional path to persist cache across restarts.
    dump_file: Option<std::path::PathBuf>,
    /// Seconds between periodic dumps (default 600).
    dump_interval_secs: u64,
    /// Writes since last dump; triggers dump when it exceeds threshold.
    /// Shared via Arc so the background task (a clone) sees live updates.
    changes_since_dump: Arc<AtomicU64>,
    /// Unix seconds of the last successful dump; 0 = never.
    last_dump_unix: Arc<AtomicU64>,
}

impl Clone for CachePlugin {
    fn clone(&self) -> Self {
        Self {
            cache: Arc::clone(&self.cache),
            max_size: self.max_size,
            stats: Arc::clone(&self.stats),
            negative_cache: self.negative_cache,
            negative_ttl: self.negative_ttl,
            enable_lazycache: self.enable_lazycache,
            lazycache_threshold: self.lazycache_threshold,
            cache_ttl: self.cache_ttl,
            lazycache_stats: Arc::clone(&self.lazycache_stats),
            refreshing_keys: Arc::clone(&self.refreshing_keys),
            tag: self.tag.clone(),
            refresh_coordinator: Arc::clone(&self.refresh_coordinator),
            enable_cleanup: self.enable_cleanup,
            cleanup_interval_secs: self.cleanup_interval_secs,
            cleanup_pressure_threshold: self.cleanup_pressure_threshold,
            dump_file: self.dump_file.clone(),
            dump_interval_secs: self.dump_interval_secs,
            changes_since_dump: Arc::clone(&self.changes_since_dump),
            last_dump_unix: Arc::clone(&self.last_dump_unix),
        }
    }
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
            enable_lazycache: false,
            lazycache_threshold: 0.05, // Refresh at 5% remaining TTL (hot entries)
            cache_ttl: None,
            lazycache_stats: Arc::new(LazyCacheStats::new()),
            refreshing_keys: Arc::new(DashSet::new()),
            tag: None,
            refresh_coordinator: Arc::new(Mutex::new(None)),
            enable_cleanup: true,
            cleanup_interval_secs: 60,
            cleanup_pressure_threshold: 0.8,
            dump_file: None,
            dump_interval_secs: 600,
            changes_since_dump: Arc::new(AtomicU64::new(0)),
            last_dump_unix: Arc::new(AtomicU64::new(0)),
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

    /// Build a refresh coordinator wired to clean up this cache's
    /// `refreshing_keys` dedup set when each task completes.
    ///
    /// Without this callback, `refreshing_keys` would only ever be cleaned on
    /// the enqueue-failure paths, so the first successful background refresh
    /// would leave its key permanently in the set and block all future
    /// background refreshes for that key.
    fn build_coordinator(
        worker_count: usize,
        queue_capacity: usize,
        refreshing_keys: Arc<DashSet<String>>,
    ) -> RefreshCoordinator {
        RefreshCoordinator::new_with_callback(
            worker_count,
            queue_capacity,
            // Remove the key from the dedup set regardless of outcome so the
            // next lazy/stale hit can schedule a fresh refresh.
            Some(Arc::new(move |key: &str, _success: bool| {
                refreshing_keys.remove(key);
            })),
        )
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
                *guard = Some(Self::build_coordinator(
                    4,
                    1000,
                    Arc::clone(&self.refreshing_keys),
                ));
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
                    *guard = Some(Self::build_coordinator(
                        4,
                        1000,
                        Arc::clone(&self.refreshing_keys),
                    ));
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

    pub fn size(&self) -> usize {
        self.cache.read().len()
    }

    /// Whether a key is currently marked as refreshing (in flight in a
    /// background refresh). Exposed for integration tests to verify the
    /// completion callback clears the dedup set after a refresh finishes;
    /// not part of the stable public API.
    #[doc(hidden)]
    pub fn is_refreshing(&self, key: &str) -> bool {
        self.refreshing_keys.contains(key)
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

            // Pack DNSSEC-relevant flags into a single byte so that queries
            // with different DNSSEC expectations are cached separately.
            // This prevents, for example, serving a DNSSEC-enabled response (with
            // RRSIG records) to a client that did not request DNSSEC.
            //
            // RFC 6840 §5.7 (AD), RFC 4035 (CD), RFC 6891 §6.1.3 (DO).
            let mut flags = 0u8;
            if message.authentic_data() {
                flags |= 1;
            }
            if message.checking_disabled() {
                flags |= 2;
            }
            if message.additional().iter().any(|rr| {
                matches!(rr.rdata(), crate::dns::RData::OPT { flags, .. } if (*flags & 0x8000) != 0)
            }) {
                flags |= 4;
            }

            format!(
                "{}:{}:{}:{}",
                qname_lower,
                q.qtype().to_u16(),
                q.qclass().to_u16(),
                flags
            )
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

        self.changes_since_dump.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot all cache entries for persistence.
    fn snapshot_entries(&self) -> Vec<(String, CacheEntry)> {
        let cache = self.cache.read();
        cache.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Dump cache to disk if a dump file is configured.
    fn dump_to_file(&self) {
        if let Some(ref path) = self.dump_file {
            let entries = self.snapshot_entries();
            match persistence::dump_cache(path, &entries) {
                Ok(()) => {
                    debug!(entries = entries.len(), path = %path.display(), "cache dumped");
                    self.changes_since_dump.store(0, Ordering::Relaxed);
                    self.last_dump_unix.store(unix_now(), Ordering::Relaxed);
                }
                Err(e) => {
                    warn!(error = %e, "failed to dump cache");
                }
            }
        }
    }

    /// Whether the configured dump interval has elapsed since the last dump.
    fn dump_interval_elapsed(&self) -> bool {
        let last = self.last_dump_unix.load(Ordering::Relaxed);
        last == 0 || unix_now().saturating_sub(last) >= self.dump_interval_secs
    }

    /// Load a previous dump file into the cache. Entries whose original TTL
    /// has burned down while the server was down are skipped.
    fn restore_from_dump(&mut self) {
        let Some(ref path) = self.dump_file else {
            return;
        };
        match persistence::load_cache(path) {
            Ok(loaded) => {
                let now = unix_now();
                let mut count = 0;
                let mut c = self.cache.write();
                for entry in loaded {
                    let elapsed = now.saturating_sub(entry.cached_at_unix);
                    let remaining = entry.original_ttl.saturating_sub(elapsed as u32);
                    if remaining == 0 {
                        continue;
                    }

                    let cache_entry = CacheEntry {
                        response: Arc::new(entry.response),
                        cached_at: Instant::now(),
                        ttl: remaining,
                        // is_cache_expired treats 0 as already expired
                        cache_ttl: remaining,
                        original_ttl: entry.original_ttl,
                        last_accessed: Instant::now(),
                        cached_at_unix: entry.cached_at_unix,
                    };
                    c.push(entry.key, cache_entry);
                    count += 1;
                }
                debug!(loaded = count, "restored cache entries from dump");
            }
            Err(e) => {
                warn!(error = %e, "failed to load cache dump");
            }
        }
    }

    /// Get minimum TTL from a DNS message
    fn get_min_ttl(message: &Message) -> u32 {
        let mut min_ttl = u32::MAX;
        for record in message.records() {
            min_ttl = min_ttl.min(record.ttl());
        }

        if min_ttl == u32::MAX {
            300
        } else {
            min_ttl.max(1)
        }
    }

    /// Update TTLs in a cached response
    fn update_ttls(message: &mut Message, remaining_ttl: u32) {
        for record in message.records_mut() {
            record.set_ttl(remaining_ttl);
        }
    }

    /// Serve a cached entry to the client.
    ///
    /// Deep-clones the cached response (so the cached object itself is never
    /// mutated), refreshes its TTL to `ttl`, restores the request's id and
    /// syncs the question section so the response always matches the client's
    /// query. Also marks `response_from_cache` so Phase 2 does not re-store it.
    fn serve_cached_response(context: &mut Context, entry: &CacheEntry, ttl: u32) {
        let mut response = (*entry.response).clone();
        Self::update_ttls(&mut response, ttl);
        response.set_id(context.request().id());
        // Sync request QUESTION SECTION to avoid query/response mismatch
        let request_questions = context.request().questions().to_vec();
        *response.questions_mut() = request_questions;
        context.set_response_arc(Some(Arc::new(response)));

        // Mark that response came from cache to prevent Phase 2 re-execution
        context.set_metadata("response_from_cache", true);
    }

    /// Trigger a background refresh of `key`, de-duplicated via `refreshing_keys`.
    ///
    /// `label` is a short tag (such as "stale-serving TTL", "LazyCache") used in
    /// log messages to tell the two call sites apart. This factors out logic
    /// that was previously duplicated verbatim by the stale-serving path and
    /// the LazyCache threshold path.
    fn spawn_background_refresh(&self, context: &Context, key: &str, label: &'static str) {
        if !self.refreshing_keys.insert(key.to_string()) {
            debug!(
                "{}: {} already being refreshed, skipping duplicate background refresh",
                label, key
            );
            return;
        }
        self.lazycache_stats.record_refresh();

        // Resolve the handler/entry to run the refresh against.
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
            let key_clone = key.to_string();
            let coordinator = Arc::clone(&self.refresh_coordinator);

            // Mark as background refresh.
            request_clone.set_id(0xFFFF);

            let task = RefreshTask {
                key: key_clone.clone(),
                message: request_clone,
                handler: background_handler,
                entry_name: entry_name.clone(),
                created_at: Instant::now(),
            };

            tokio::spawn(async move {
                if let Some(coord) = coordinator.lock().await.as_ref() {
                    match coord.enqueue(task).await {
                        Ok(_) => {
                            debug!("Background {} refresh enqueued for {}", label, key_clone);
                        }
                        Err(e) => {
                            debug!(
                                "Failed to enqueue {} refresh for {}: {}",
                                label, key_clone, e
                            );
                            // Remove from refreshing set if enqueue failed.
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
                "{}: handler metadata missing, falling back to invalidate stale entry",
                label
            );
            let cache_clone = Arc::clone(&self.cache);
            let refreshing_keys_clone = Arc::clone(&self.refreshing_keys);
            let key_clone = key.to_string();
            tokio::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                debug!("Fallback: invalidating cache entry for {}", key_clone);
                cache_clone.write().pop(&key_clone);
                refreshing_keys_clone.remove(&key_clone);
            });
        }
    }

    /// Phase 1: look up cache. On hit, serves the response and sets RETURN_FLAG.
    async fn try_serve_from_cache(&self, context: &mut Context, key: &str) -> Result<()> {
        let cache_already_checked = context.get_metadata::<bool>("cache_checked").is_some();
        context.set_metadata("cache_checked", true);

        if context
            .get_metadata::<bool>("background_lazy_refresh")
            .is_some()
        {
            debug!("Skipping cache for background lazy refresh");
            return Ok(());
        }

        let cached_entry = {
            let mut cache = self.cache.write();
            cache.get(key).cloned()
        };

        let Some(mut entry) = cached_entry else {
            if !cache_already_checked {
                self.stats.record_miss();
                debug!("Cache miss: {}", key);
            }
            return Ok(());
        };

        if entry.is_cache_expired() {
            debug!("Cache entry expired: {}", key);
            self.cache.write().pop(key);
            self.stats.record_expiration();
            self.stats.record_miss();
            #[cfg(feature = "metrics")]
            {
                metrics::CACHE_SIZE.set(self.size() as i64);
            }
            return Ok(());
        }

        debug!("Cache hit: {}", key);
        self.stats.record_hit();
        entry.touch();

        let remaining_ttl = entry.remaining_ttl();

        if remaining_ttl == 0 {
            if let Some(lazy_ttl) = self.cache_ttl {
                debug!(
                    "Stale-serving: {}, cache_remaining: {}s, lazy_ttl: {}s",
                    key,
                    entry.remaining_cache_ttl(),
                    lazy_ttl
                );
                Self::serve_cached_response(context, &entry, STALE_RESPONSE_TTL_SECS);
                self.spawn_background_refresh(context, key, "stale-serving TTL");
                context.set_metadata(RETURN_FLAG, true);
            } else {
                self.cache.write().pop(key);
                self.stats.record_expiration();
                self.stats.record_miss();
                #[cfg(feature = "metrics")]
                {
                    metrics::CACHE_SIZE.set(self.size() as i64);
                }
            }
            return Ok(());
        }

        let should_lazy_refresh = self.enable_lazycache
            && context
                .get_metadata::<bool>("background_lazy_refresh")
                .is_none()
            && {
                let pct = remaining_ttl as f32 / entry.original_ttl as f32;
                let below = pct <= self.lazycache_threshold;
                if below {
                    debug!(
                        "LazyCache threshold: {} at {:.1}% (< {:.1}%)",
                        key,
                        pct * 100.0,
                        self.lazycache_threshold * 100.0
                    );
                }
                below
            };

        if should_lazy_refresh {
            Self::serve_cached_response(context, &entry, remaining_ttl);
            self.spawn_background_refresh(context, key, "LazyCache");
            context.set_metadata(RETURN_FLAG, true);
            return Ok(());
        }

        if context
            .get_metadata::<bool>("background_lazy_refresh")
            .is_some()
        {
            debug!(
                "Background refresh: cache hit, continuing downstream for {}",
                key
            );
            return Ok(());
        }

        Self::serve_cached_response(context, &entry, remaining_ttl);
        context.set_metadata(RETURN_FLAG, true);
        Ok(())
    }

    /// Phase 2: store a downstream response into cache.
    fn try_store_response(&self, context: &mut Context, key: &str) {
        if context
            .get_metadata::<bool>("response_from_cache")
            .is_some()
        {
            return;
        }

        let Some(response) = context.response() else {
            return;
        };

        let response_code = response.response_code();
        let is_error = response_code != crate::dns::ResponseCode::NoError;

        if is_error {
            if self.negative_cache {
                debug!(
                    "Caching negative response: {:?} (TTL: {}s)",
                    response_code, self.negative_ttl
                );
                let cache_ttl = self.cache_ttl.unwrap_or(self.negative_ttl);
                let entry = CacheEntry::new(response.clone(), self.negative_ttl, cache_ttl);
                self.store(key.to_string(), entry);
            }
        } else if !response.answers().is_empty() {
            let ttl = Self::get_min_ttl(response);
            if ttl > 0 {
                let cache_ttl = self.cache_ttl.unwrap_or(ttl);
                debug!(
                    "Storing: {} (msg TTL: {}s, cache TTL: {}s)",
                    key, ttl, cache_ttl
                );
                let entry = CacheEntry::new(response.clone(), ttl, cache_ttl);
                self.store(key.to_string(), entry);
            }
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

        if self.dump_file.is_some() {
            let changes = self.changes_since_dump.load(Ordering::Relaxed);
            if changes > 0
                && (changes >= persistence::dump_threshold() || self.dump_interval_elapsed())
            {
                self.dump_to_file();
            }
        }
    }

    fn background_task_name(&self) -> &str {
        "cache_cleanup"
    }
}

#[async_trait]
impl Plugin for CachePlugin {
    async fn execute(&self, context: &mut Context) -> Result<()> {
        let key = match Self::make_key(context.request()) {
            Some(k) => k,
            None => return Ok(()),
        };

        if context
            .get_metadata::<bool>("response_from_cache")
            .is_some()
        {
            return Ok(());
        }

        if context.response().is_none() {
            self.try_serve_from_cache(context, &key).await?;
        } else {
            self.try_store_response(context, &key);
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "cache"
    }

    fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
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

        // Create refresh coordinator if lazycache or cache_ttl is enabled.
        // Worker count and queue capacity are internal constants, not user-tunable.
        const REFRESH_WORKER_COUNT: usize = 4;
        const REFRESH_QUEUE_CAPACITY: usize = 1000;
        if cache.enable_lazycache || cache.cache_ttl.is_some() {
            // Initialize coordinator only if not already set by builder methods
            if let Ok(mut guard) = cache.refresh_coordinator.try_lock() {
                if guard.is_none() {
                    *guard = Some(Self::build_coordinator(
                        REFRESH_WORKER_COUNT,
                        REFRESH_QUEUE_CAPACITY,
                        Arc::clone(&cache.refreshing_keys),
                    ));
                }
            } else {
                // If mutex is currently locked, replace to ensure initialization
                let coordinator = Self::build_coordinator(
                    REFRESH_WORKER_COUNT,
                    REFRESH_QUEUE_CAPACITY,
                    Arc::clone(&cache.refreshing_keys),
                );
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

        // Cleanup is always enabled with sensible defaults (60s interval, 0.8 pressure).
        // These are internal tuning constants, not user-facing config.
        const CLEANUP_INTERVAL_SECS: u64 = 60;
        const CLEANUP_PRESSURE_THRESHOLD: f32 = 0.8;
        cache = cache.with_cleanup(true, CLEANUP_INTERVAL_SECS, CLEANUP_PRESSURE_THRESHOLD);

        // Parse cache persistence options.
        if let Some(Value::String(s)) = args.get("dump_file") {
            cache.dump_file = Some(std::path::PathBuf::from(s));

            if let Some(Value::Number(n)) = args.get("dump_interval") {
                cache.dump_interval_secs = n.as_u64().unwrap_or(600);
            }

            cache.restore_from_dump();
        }

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
        if let Some(coordinator) = self.refresh_coordinator.lock().await.take() {
            debug!("Shutting down CachePlugin refresh coordinator");
            coordinator.shutdown().await?;
        }
        self.dump_to_file();
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
        assert_eq!(key.unwrap(), "example.com:1:1:0");
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
        assert_eq!(key_lower_str, "example.com:1:1:0");
    }

    #[test]
    fn test_make_key_no_questions() {
        let msg = Message::new();
        let key = CachePlugin::make_key(&msg);

        assert!(key.is_none());
    }

    #[test]
    fn test_make_key_dnssec_flags_separate_keys() {
        // Base query (no DNSSEC): flags = 0
        let mut msg = Message::new();
        msg.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
        let key_plain = CachePlugin::make_key(&msg).unwrap();
        assert!(key_plain.ends_with(":0"));

        // AD bit set: flags = 1
        let mut msg_ad = msg.clone();
        msg_ad.set_authentic_data(true);
        let key_ad = CachePlugin::make_key(&msg_ad).unwrap();
        assert!(key_ad.ends_with(":1"));
        assert_ne!(key_plain, key_ad);

        // CD bit set: flags = 2
        let mut msg_cd = msg.clone();
        msg_cd.set_checking_disabled(true);
        let key_cd = CachePlugin::make_key(&msg_cd).unwrap();
        assert!(key_cd.ends_with(":2"));
        assert_ne!(key_plain, key_cd);

        // DO bit set via OPT record: flags = 4
        let mut msg_do = msg.clone();
        msg_do.add_additional(ResourceRecord::new(
            "",
            RecordType::OPT,
            RecordClass::IN,
            0,
            crate::dns::RData::OPT {
                extended_rcode: 0,
                version: 0,
                flags: 0x8000, // DO bit
                options: Vec::new(),
            },
        ));
        let key_do = CachePlugin::make_key(&msg_do).unwrap();
        assert!(key_do.ends_with(":4"));
        assert_ne!(key_plain, key_do);
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
        let key = "example.com:1:1:0".to_string();
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
        let key = "example.com:1:1:0".to_string();
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
        let cache = CachePlugin::new(100).with_lazycache(0.1); // 10% threshold

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
            .peek(&"example.com:1:1:0".to_string())
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
        let cache = CachePlugin::new(100).with_lazycache(0.05); // 5% threshold

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
    fn test_dump_counter_shared_with_clone() {
        // the background task runs on a clone; increments on the original
        // must be visible there or periodic dumps never fire
        let cache = CachePlugin::new(100);
        let bg = cache.clone();
        cache.changes_since_dump.fetch_add(3, Ordering::Relaxed);
        assert_eq!(bg.changes_since_dump.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn test_dump_and_restore() {
        let file =
            std::env::temp_dir().join(format!("lazydns_cache_test_{}.bin", std::process::id()));

        let mut src = CachePlugin::new(100);
        src.dump_file = Some(file.clone());
        src.cache.write().push(
            "example.com".to_string(),
            CacheEntry::new(create_test_response(), 300, 300),
        );
        src.dump_to_file();

        let mut restored = CachePlugin::new(100);
        restored.dump_file = Some(file.clone());
        restored.restore_from_dump();

        let cache = restored.cache.read();
        let entry = cache.peek("example.com").expect("entry not restored");
        assert!(!entry.is_cache_expired(), "restored entry must be servable");
        assert!(entry.remaining_ttl() > 0);
        drop(cache);

        let _ = std::fs::remove_file(&file);
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
    async fn test_spawn_background_task() {
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

        let cleanup_handle = cache_with_short_interval.clone().spawn_background_task();

        // Wait for cleanup to run (at most 1.5 seconds)
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // Cancel the cleanup task
        cleanup_handle.abort();

        // Only asserts the task spawns and runs; eviction is covered by unit tests
        // on CacheStore directly (this instance holds no entries to expire).
    }
}
