use lru::LruCache;
use parking_lot::RwLock;
use std::num::NonZeroUsize;

use super::entry::CacheEntry;

/// Shard count; powers of two keep the mask cheap.
const SHARDS: usize = 16;

/// LRU split into independent shards. A single `LruCache` behind one lock
/// would serialize every cache read (recency updates need `&mut`); with
/// shards, lookups on different keys run in parallel. A skewed shard may
/// evict slightly before the total capacity is reached.
pub(super) struct ShardedLruCache {
    shards: Vec<RwLock<LruCache<String, CacheEntry>>>,
    mask: usize,
}

impl ShardedLruCache {
    /// Capacity is distributed exactly across `min(SHARDS, capacity)` shards.
    pub(super) fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let shard_count = SHARDS.min(capacity);
        let per_shard = capacity / shard_count;
        let remainder = capacity % shard_count;

        let shards = (0..shard_count)
            .map(|i| {
                let cap = per_shard + usize::from(i < remainder);
                RwLock::new(LruCache::new(NonZeroUsize::new(cap.max(1)).unwrap()))
            })
            .collect();

        Self {
            shards,
            mask: shard_count - 1,
        }
    }

    fn shard_for(&self, key: &str) -> &RwLock<LruCache<String, CacheEntry>> {
        // a cheap stable hash; distribution matters more than strength here.
        // u64, not usize: 32-bit targets cannot hold the FNV constants.
        let mut h: u64 = 0xcbf29ce484222325;
        for b in key.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        &self.shards[(h as usize) & self.mask]
    }

    /// Lookup with recency update (exclusive lock on one shard only).
    pub(super) fn get(&self, key: &str) -> Option<CacheEntry> {
        self.shard_for(key).write().get(key).cloned()
    }

    /// Lookup without recency update (shared lock, no mutation).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn peek(&self, key: &str) -> Option<CacheEntry> {
        self.shard_for(key).read().peek(key).cloned()
    }

    pub(super) fn contains(&self, key: &str) -> bool {
        self.shard_for(key).read().contains(key)
    }

    /// Insert; returns the evicted pair when the shard was full.
    pub(super) fn push(&self, key: String, entry: CacheEntry) -> Option<(String, CacheEntry)> {
        self.shard_for(&key).write().push(key, entry)
    }

    pub(super) fn pop(&self, key: &str) -> Option<CacheEntry> {
        self.shard_for(key).write().pop(key)
    }

    pub(super) fn len(&self) -> usize {
        self.shards.iter().map(|s| s.read().len()).sum()
    }

    pub(super) fn clear(&self) {
        for shard in &self.shards {
            shard.write().clear();
        }
    }

    /// Full copy of every live entry.
    pub(super) fn entries(&self) -> Vec<(String, CacheEntry)> {
        let mut out = Vec::new();
        for shard in &self.shards {
            out.extend(shard.read().iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        out
    }

    /// Drop every entry matching `keep` == false; returns the number dropped.
    pub(super) fn retain<F: Fn(&CacheEntry) -> bool>(&self, keep: F) -> usize {
        let mut dropped = 0;
        for shard in &self.shards {
            let mut cache = shard.write();
            let stale: Vec<String> = cache
                .iter()
                .filter(|(_, entry)| !keep(entry))
                .map(|(k, _)| k.clone())
                .collect();
            for key in stale {
                if cache.pop(&key).is_some() {
                    dropped += 1;
                }
            }
        }
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::{Message, Question, RecordClass, RecordType};

    fn entry(ttl: u32) -> CacheEntry {
        let mut msg = Message::new();
        msg.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
        CacheEntry::new(msg, ttl, ttl)
    }

    #[test]
    fn capacity_is_exact_across_shards() {
        let c = ShardedLruCache::new(10);
        assert_eq!(c.shards.len(), 10);
        assert_eq!(c.len(), 0);

        let c = ShardedLruCache::new(1000);
        assert_eq!(c.shards.len(), SHARDS);
        let total: usize = c.shards.iter().map(|s| s.read().cap().get()).sum();
        assert_eq!(total, 1000);
    }

    #[test]
    fn get_and_eviction_work_per_shard() {
        let c = ShardedLruCache::new(1);
        assert!(c.push("a".to_string(), entry(300)).is_none());
        let evicted = c.push("b".to_string(), entry(300)).unwrap();
        assert_eq!(evicted.0, "a");
        assert!(c.get("a").is_none());
        assert!(c.get("b").is_some());
    }

    #[test]
    fn entries_spans_all_shards() {
        let c = ShardedLruCache::new(64);
        for i in 0..16 {
            c.push(format!("key{}", i), entry(300));
        }
        assert_eq!(c.len(), 16);
        assert_eq!(c.entries().len(), 16);
    }
}
