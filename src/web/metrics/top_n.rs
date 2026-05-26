//! Top-N tracking algorithm
//!
//! Efficiently tracks the top N items by count using a min-heap.

use parking_lot::RwLock;
use serde::Serialize;
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

/// Time-stamped count entry
#[derive(Debug, Clone)]
struct TimestampedCount {
    /// Total count
    count: u64,
    /// First occurrence time
    first_seen: Instant,
    /// Last updated time
    last_updated: Instant,
}

/// Top-N tracker using count-min sketch approximation with exact top tracking
#[derive(Debug)]
pub struct TopN<K: Hash + Eq + Clone> {
    /// Maximum number of items to track
    n: usize,
    /// Item counts with timestamps
    counts: RwLock<HashMap<K, TimestampedCount>>,
    /// Maximum items to keep before pruning
    max_items: usize,
}

impl<K: Hash + Eq + Clone> TopN<K> {
    /// Create a new TopN tracker
    pub fn new(n: usize) -> Self {
        Self {
            n,
            counts: RwLock::new(HashMap::new()),
            // Keep 10x items before pruning to avoid frequent pruning
            max_items: n * 10,
        }
    }

    /// Create with custom max items before pruning
    pub fn with_max_items(n: usize, max_items: usize) -> Self {
        Self {
            n,
            counts: RwLock::new(HashMap::new()),
            max_items,
        }
    }

    /// Increment the count for a key
    pub fn increment(&self, key: K) {
        self.add(key, 1);
    }

    /// Add a count for a key
    pub fn add(&self, key: K, count: u64) {
        let now = Instant::now();
        let mut counts = self.counts.write();
        counts
            .entry(key)
            .and_modify(|entry| {
                entry.count += count;
                entry.last_updated = now;
            })
            .or_insert_with(|| TimestampedCount {
                count,
                first_seen: now,
                last_updated: now,
            });

        // Prune if too many items
        if counts.len() > self.max_items {
            self.prune_locked(&mut counts);
        }
    }

    /// Prune to keep only top items (requires write lock already held)
    fn prune_locked(&self, counts: &mut HashMap<K, TimestampedCount>) {
        if counts.len() <= self.n * 2 {
            return;
        }

        // Find threshold (n-th largest count)
        let mut count_values: Vec<u64> = counts.values().map(|v| v.count).collect();
        count_values.sort_unstable_by(|a, b| b.cmp(a));

        if let Some(&threshold) = count_values.get(self.n * 2) {
            counts.retain(|_, v| v.count > threshold);
        }
    }

    /// Get the top N items with their counts
    pub fn top(&self) -> Vec<(K, u64)> {
        let counts = self.counts.read();
        let mut items: Vec<(K, u64)> = counts.iter().map(|(k, v)| (k.clone(), v.count)).collect();

        // Sort by count descending
        items.sort_unstable_by_key(|item| std::cmp::Reverse(item.1));

        // Take top N
        items.truncate(self.n);
        items
    }

    /// Get the top N items within a time window (seconds ago)
    pub fn top_within(&self, window_secs: u64) -> Vec<(K, u64)> {
        let now = Instant::now();
        let window = std::time::Duration::from_secs(window_secs);
        let counts = self.counts.read();

        let mut items: Vec<(K, u64)> = counts
            .iter()
            .filter(|(_, entry)| now.duration_since(entry.last_updated) <= window)
            .map(|(k, entry)| (k.clone(), entry.count))
            .collect();

        // Sort by count descending
        items.sort_unstable_by_key(|item| std::cmp::Reverse(item.1));

        // Take top N
        items.truncate(self.n);
        items
    }

    /// Get the count for a specific key
    pub fn get(&self, key: &K) -> u64 {
        self.counts.read().get(key).map(|v| v.count).unwrap_or(0)
    }

    /// Get total count of all items
    pub fn total(&self) -> u64 {
        self.counts.read().values().map(|v| v.count).sum()
    }

    /// Get number of unique items tracked
    pub fn len(&self) -> usize {
        self.counts.read().len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.counts.read().is_empty()
    }

    /// Clear all counts
    pub fn clear(&self) {
        self.counts.write().clear();
    }

    /// Merge another TopN into this one
    pub fn merge(&self, other: &TopN<K>) {
        let other_counts = other.counts.read();
        let mut self_counts = self.counts.write();

        for (key, entry) in other_counts.iter() {
            self_counts
                .entry(key.clone())
                .and_modify(|e| e.count += entry.count)
                .or_insert_with(|| TimestampedCount {
                    count: entry.count,
                    first_seen: entry.first_seen,
                    last_updated: entry.last_updated,
                });
        }

        if self_counts.len() > self.max_items {
            self.prune_locked(&mut self_counts);
        }
    }
}

impl<K: Hash + Eq + Clone + Serialize> TopN<K> {
    /// Get top N as serializable entries
    pub fn top_entries(&self) -> Vec<TopNEntry<K>> {
        self.top()
            .into_iter()
            .enumerate()
            .map(|(rank, (key, count))| TopNEntry {
                rank: rank + 1,
                key,
                count,
            })
            .collect()
    }

    /// Get top N entries within a time window (seconds ago)
    pub fn top_entries_within(&self, window_secs: u64) -> Vec<TopNEntry<K>> {
        self.top_within(window_secs)
            .into_iter()
            .enumerate()
            .map(|(rank, (key, count))| TopNEntry {
                rank: rank + 1,
                key,
                count,
            })
            .collect()
    }
}

/// A single entry in the top-N list
#[derive(Debug, Clone, Serialize)]
pub struct TopNEntry<K> {
    pub rank: usize,
    pub key: K,
    pub count: u64,
}

/// Specialized TopN for domain tracking
pub type TopDomains = TopN<String>;

/// Specialized TopN for client IP tracking
pub type TopClients = TopN<String>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_increment() {
        let top = TopN::new(3);
        top.increment("a".to_string());
        top.increment("a".to_string());
        top.increment("b".to_string());

        assert_eq!(top.get(&"a".to_string()), 2);
        assert_eq!(top.get(&"b".to_string()), 1);
        assert_eq!(top.get(&"c".to_string()), 0);
    }

    #[test]
    fn test_top_n() {
        let top = TopN::new(2);
        top.add("a".to_string(), 100);
        top.add("b".to_string(), 50);
        top.add("c".to_string(), 200);
        top.add("d".to_string(), 10);

        let result = top.top();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "c");
        assert_eq!(result[0].1, 200);
        assert_eq!(result[1].0, "a");
        assert_eq!(result[1].1, 100);
    }

    #[test]
    fn test_total() {
        let top = TopN::new(10);
        top.add("a".to_string(), 10);
        top.add("b".to_string(), 20);
        top.add("c".to_string(), 30);

        assert_eq!(top.total(), 60);
    }

    #[test]
    fn test_prune() {
        let top = TopN::with_max_items(2, 5);

        for i in 0..20 {
            top.add(format!("item{}", i), i as u64);
        }

        // Should have pruned to keep only high-count items
        assert!(top.len() <= 10); // Should be around 2*n after pruning
    }

    #[test]
    fn test_merge() {
        let top1 = TopN::new(5);
        top1.add("a".to_string(), 10);
        top1.add("b".to_string(), 20);

        let top2 = TopN::new(5);
        top2.add("a".to_string(), 5);
        top2.add("c".to_string(), 30);

        top1.merge(&top2);

        assert_eq!(top1.get(&"a".to_string()), 15);
        assert_eq!(top1.get(&"b".to_string()), 20);
        assert_eq!(top1.get(&"c".to_string()), 30);
    }

    #[test]
    fn test_empty_top_n() {
        let top = TopN::<String>::new(5);

        assert!(top.is_empty());
        assert_eq!(top.len(), 0);
        assert_eq!(top.total(), 0);
        assert!(top.top().is_empty());
    }

    #[test]
    fn test_clear() {
        let top = TopN::new(5);
        top.add("a".to_string(), 10);
        top.add("b".to_string(), 20);

        assert_eq!(top.len(), 2);

        top.clear();

        assert!(top.is_empty());
        assert_eq!(top.total(), 0);
    }

    #[test]
    fn test_top_entries() {
        let top = TopN::new(3);
        top.add("first".to_string(), 100);
        top.add("second".to_string(), 50);
        top.add("third".to_string(), 25);

        let entries = top.top_entries();
        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].rank, 1);
        assert_eq!(entries[0].key, "first");
        assert_eq!(entries[0].count, 100);

        assert_eq!(entries[1].rank, 2);
        assert_eq!(entries[1].key, "second");
        assert_eq!(entries[1].count, 50);

        assert_eq!(entries[2].rank, 3);
        assert_eq!(entries[2].key, "third");
        assert_eq!(entries[2].count, 25);
    }

    #[test]
    fn test_top_n_respects_limit() {
        let top = TopN::new(3);

        for i in 0..10 {
            top.add(format!("item{}", i), (i + 1) as u64);
        }

        let result = top.top();
        assert_eq!(result.len(), 3);

        // Should contain highest counts
        assert!(result.iter().any(|(k, _)| k == "item9"));
        assert!(result.iter().any(|(k, _)| k == "item8"));
        assert!(result.iter().any(|(k, _)| k == "item7"));
    }

    #[test]
    fn test_cumulative_add() {
        let top = TopN::new(5);

        top.add("item".to_string(), 5);
        top.add("item".to_string(), 10);
        top.add("item".to_string(), 15);

        assert_eq!(top.get(&"item".to_string()), 30);
    }

    #[test]
    fn test_single_item() {
        let top = TopN::new(10);
        top.increment("single".to_string());

        assert_eq!(top.len(), 1);
        assert_eq!(top.get(&"single".to_string()), 1);

        let result = top.top();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "single");
    }

    #[test]
    fn test_integer_keys() {
        let top = TopN::new(3);
        top.add(1, 100);
        top.add(2, 200);
        top.add(3, 50);

        assert_eq!(top.get(&2), 200);
        let result = top.top();
        assert_eq!(result[0].0, 2);
    }

    #[test]
    fn test_merge_empty() {
        let top1 = TopN::new(5);
        top1.add("a".to_string(), 10);

        let top2 = TopN::<String>::new(5);

        top1.merge(&top2);

        assert_eq!(top1.get(&"a".to_string()), 10);
        assert_eq!(top1.len(), 1);
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let top = Arc::new(TopN::new(10));
        let mut handles = vec![];

        for i in 0..10 {
            let top_clone = Arc::clone(&top);
            handles.push(thread::spawn(move || {
                for j in 0..100 {
                    top_clone.increment(format!("item{}", (i + j) % 20));
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(top.total(), 1000);
    }

    #[test]
    fn test_prune_keeps_high_counts() {
        let top = TopN::with_max_items(3, 6);

        // Add items with varying counts
        top.add("low1".to_string(), 1);
        top.add("low2".to_string(), 2);
        top.add("low3".to_string(), 3);
        top.add("high1".to_string(), 100);
        top.add("high2".to_string(), 200);
        top.add("high3".to_string(), 300);
        top.add("low4".to_string(), 4);

        // After pruning, high counts should be retained
        let result = top.top();
        assert!(result.iter().any(|(k, _)| k == "high3"));
        assert!(result.iter().any(|(k, _)| k == "high2"));
        assert!(result.iter().any(|(k, _)| k == "high1"));
    }

    #[test]
    fn test_get_nonexistent() {
        let top = TopN::new(5);
        top.add("exists".to_string(), 10);

        assert_eq!(top.get(&"exists".to_string()), 10);
        assert_eq!(top.get(&"nonexistent".to_string()), 0);
    }

    #[test]
    fn test_is_empty_after_add() {
        let top = TopN::new(5);
        assert!(top.is_empty());

        top.increment("item".to_string());
        assert!(!top.is_empty());
    }
}
