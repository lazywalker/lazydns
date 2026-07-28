use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::dns::Message;

#[derive(Clone)]
pub struct CacheEntry {
    pub response: Arc<Message>,
    pub cached_at: Instant,
    pub ttl: u32,
    pub cache_ttl: u32,
    pub original_ttl: u32,
    pub last_accessed: Instant,
    pub cached_at_unix: u64,
}

impl CacheEntry {
    pub(super) fn new(response: Message, ttl: u32, cache_ttl: u32) -> Self {
        let now = Instant::now();
        let cached_at_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            response: Arc::new(response),
            cached_at: now,
            ttl,
            cache_ttl,
            original_ttl: ttl,
            last_accessed: now,
            cached_at_unix,
        }
    }

    pub(super) fn is_cache_expired(&self) -> bool {
        if self.cache_ttl == 0 {
            return true;
        }
        // >= to avoid timing races where elapsed may equal the TTL
        self.cached_at.elapsed() >= Duration::from_secs(self.cache_ttl as u64)
    }

    pub(super) fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }

    pub(super) fn remaining_ttl(&self) -> u32 {
        let elapsed = self.cached_at.elapsed().as_secs() as u32;
        self.ttl.saturating_sub(elapsed)
    }

    pub(super) fn remaining_cache_ttl(&self) -> u32 {
        let elapsed = self.cached_at.elapsed().as_secs() as u32;
        self.cache_ttl.saturating_sub(elapsed)
    }
}
