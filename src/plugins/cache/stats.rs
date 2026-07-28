use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct CacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    expirations: AtomicU64,
}

impl CacheStats {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "metrics")]
        {
            crate::metrics::CACHE_HITS_TOTAL.inc();
        }
    }

    pub(super) fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "metrics")]
        {
            crate::metrics::CACHE_MISSES_TOTAL.inc();
        }
    }

    pub(super) fn record_eviction(&self) {
        self.evictions.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "metrics")]
        {
            crate::metrics::DNS_CACHE_EVICTIONS_TOTAL.inc();
        }
    }

    pub(super) fn record_expiration(&self) {
        self.expirations.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "metrics")]
        {
            crate::metrics::DNS_CACHE_EXPIRATIONS_TOTAL.inc();
        }
    }

    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    pub fn evictions(&self) -> u64 {
        self.evictions.load(Ordering::Relaxed)
    }

    pub fn expirations(&self) -> u64 {
        self.expirations.load(Ordering::Relaxed)
    }

    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits();
        let total = hits + self.misses();
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

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

#[derive(Debug, Default)]
pub struct LazyCacheStats {
    refreshes: AtomicU64,
    successful_refreshes: AtomicU64,
    failed_refreshes: AtomicU64,
}

impl LazyCacheStats {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn record_refresh(&self) {
        self.refreshes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn refreshes(&self) -> u64 {
        self.refreshes.load(Ordering::Relaxed)
    }

    pub fn successful_refreshes(&self) -> u64 {
        self.successful_refreshes.load(Ordering::Relaxed)
    }

    pub fn failed_refreshes(&self) -> u64 {
        self.failed_refreshes.load(Ordering::Relaxed)
    }

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
