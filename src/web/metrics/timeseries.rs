//! Time series data structures for metrics aggregation
//!
//! Provides sliding window time series with configurable granularity.

use parking_lot::RwLock;
use serde::Serialize;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// A single point in a time series
#[derive(Debug, Clone, Serialize)]
pub struct TimeSeriesPoint {
    /// Timestamp (seconds since start of collection)
    pub timestamp: u64,
    /// Value at this point
    pub value: f64,
}

/// Sliding window time series
#[derive(Debug)]
pub struct TimeSeries {
    /// Window duration
    window: Duration,
    /// Bucket duration (granularity)
    bucket_duration: Duration,
    /// Data points
    buckets: RwLock<VecDeque<Bucket>>,
    /// Start time
    start_time: Instant,
}

#[derive(Debug, Clone)]
struct Bucket {
    /// Bucket start time (relative to start_time)
    timestamp: u64,
    /// Sum of values in this bucket
    sum: f64,
    /// Count of values in this bucket
    count: u64,
    /// Minimum value
    min: f64,
    /// Maximum value
    max: f64,
}

impl Bucket {
    fn new(timestamp: u64) -> Self {
        Self {
            timestamp,
            sum: 0.0,
            count: 0,
            min: f64::MAX,
            max: f64::MIN,
        }
    }

    fn add(&mut self, value: f64) {
        self.sum += value;
        self.count += 1;
        self.min = self.min.min(value);
        self.max = self.max.max(value);
    }

    fn average(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum / self.count as f64
        }
    }
}

impl TimeSeries {
    /// Create a new time series with window and bucket duration
    pub fn new(window: Duration, bucket_duration: Duration) -> Self {
        Self {
            window,
            bucket_duration,
            buckets: RwLock::new(VecDeque::new()),
            start_time: Instant::now(),
        }
    }

    /// Create with default 5-minute window and 1-second buckets
    pub fn default_qps() -> Self {
        Self::new(Duration::from_secs(300), Duration::from_secs(1))
    }

    /// Create with 1-hour window and 1-minute buckets
    pub fn hourly() -> Self {
        Self::new(Duration::from_secs(3600), Duration::from_secs(60))
    }

    /// Add a value to the current bucket
    pub fn add(&self, value: f64) {
        let now = self.current_bucket_timestamp();
        let mut buckets = self.buckets.write();

        // Clean up old buckets
        self.cleanup_locked(&mut buckets, now);

        // Find or create current bucket
        if let Some(last) = buckets.back_mut()
            && last.timestamp == now
        {
            last.add(value);
            return;
        }

        // Create new bucket
        let mut bucket = Bucket::new(now);
        bucket.add(value);
        buckets.push_back(bucket);
    }

    /// Increment the current bucket by 1
    pub fn increment(&self) {
        self.add(1.0);
    }

    /// Get current bucket timestamp
    fn current_bucket_timestamp(&self) -> u64 {
        let elapsed = self.start_time.elapsed();
        let bucket_secs = self.bucket_duration.as_secs();
        (elapsed.as_secs() / bucket_secs) * bucket_secs
    }

    /// Earliest bucket timestamp still inside the window, from `now`.
    /// Reads must apply this themselves: cleanup only runs on add(), which
    /// never happens once traffic stops, so unfiltered reads would keep
    /// serving stale buckets forever.
    fn window_cutoff(&self) -> u64 {
        self.current_bucket_timestamp()
            .saturating_sub(self.window.as_secs())
    }

    /// Clean up old buckets (requires write lock)
    fn cleanup_locked(&self, buckets: &mut VecDeque<Bucket>, current_timestamp: u64) {
        let window_secs = self.window.as_secs();
        let cutoff = current_timestamp.saturating_sub(window_secs);

        while let Some(front) = buckets.front() {
            if front.timestamp < cutoff {
                buckets.pop_front();
            } else {
                break;
            }
        }
    }

    /// Get all data points in the window
    pub fn points(&self) -> Vec<TimeSeriesPoint> {
        let buckets = self.buckets.read();

        // Calculate the Unix timestamp of start_time
        let start_unix = self.get_start_time_unix();

        let cutoff = self.window_cutoff();
        buckets
            .iter()
            .filter(|b| b.timestamp >= cutoff)
            .map(|b| TimeSeriesPoint {
                // Convert to absolute Unix timestamp: start_time + bucket_relative_time
                timestamp: start_unix + b.timestamp,
                value: b.sum,
            })
            .collect()
    }

    /// Get average values for each bucket
    pub fn averages(&self) -> Vec<TimeSeriesPoint> {
        let buckets = self.buckets.read();

        // Calculate the Unix timestamp of start_time
        let start_unix = self.get_start_time_unix();

        let cutoff = self.window_cutoff();
        buckets
            .iter()
            .filter(|b| b.timestamp >= cutoff)
            .map(|b| TimeSeriesPoint {
                // Convert to absolute Unix timestamp: start_time + bucket_relative_time
                timestamp: start_unix + b.timestamp,
                value: b.average(),
            })
            .collect()
    }

    /// Get the Unix timestamp of the start_time
    fn get_start_time_unix(&self) -> u64 {
        use std::time::{Duration, SystemTime, UNIX_EPOCH};

        let now_instant = std::time::Instant::now();
        let elapsed_since_start = now_instant
            .checked_duration_since(self.start_time)
            .unwrap_or(Duration::ZERO);

        // Get current system time
        let now_system = SystemTime::now();
        let unix_now = now_system
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Subtract elapsed time to get start_time's Unix timestamp
        unix_now.saturating_sub(elapsed_since_start.as_secs())
    }

    /// Get the sum of all values in the window
    pub fn sum(&self) -> f64 {
        let cutoff = self.window_cutoff();
        self.buckets
            .read()
            .iter()
            .filter(|b| b.timestamp >= cutoff)
            .map(|b| b.sum)
            .sum()
    }

    /// Get the count of all values in the window
    pub fn count(&self) -> u64 {
        let cutoff = self.window_cutoff();
        self.buckets
            .read()
            .iter()
            .filter(|b| b.timestamp >= cutoff)
            .map(|b| b.count)
            .sum()
    }

    /// Get the current rate (sum per second). While the series is younger
    /// than the window, divide by the elapsed time; the full window would
    /// massively understate a busy server right after startup.
    pub fn rate(&self) -> f64 {
        let sum = self.sum();
        let elapsed = self
            .start_time
            .elapsed()
            .as_secs()
            .max(1)
            .min(self.window.as_secs());
        sum / elapsed as f64
    }

    /// Get min, max, and average over the window
    pub fn stats(&self) -> TimeSeriesStats {
        let buckets = self.buckets.read();

        if buckets.is_empty() {
            return TimeSeriesStats {
                min: 0.0,
                max: 0.0,
                avg: 0.0,
                sum: 0.0,
                count: 0,
            };
        }

        let cutoff = self.window_cutoff();
        let mut total_sum = 0.0;
        let mut total_count = 0u64;
        let mut overall_min = f64::MAX;
        let mut overall_max = f64::MIN;

        for bucket in buckets.iter().filter(|b| b.timestamp >= cutoff) {
            total_sum += bucket.sum;
            total_count += bucket.count;
            if bucket.count > 0 {
                overall_min = overall_min.min(bucket.min);
                overall_max = overall_max.max(bucket.max);
            }
        }

        TimeSeriesStats {
            min: if overall_min == f64::MAX {
                0.0
            } else {
                overall_min
            },
            max: if overall_max == f64::MIN {
                0.0
            } else {
                overall_max
            },
            avg: if total_count > 0 {
                total_sum / total_count as f64
            } else {
                0.0
            },
            sum: total_sum,
            count: total_count,
        }
    }

    /// Clear all data
    pub fn clear(&self) {
        self.buckets.write().clear();
    }
}

/// Statistics for a time series
#[derive(Debug, Clone, Serialize)]
pub struct TimeSeriesStats {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub sum: f64,
    pub count: u64,
}

/// Latency distribution buckets
#[derive(Debug)]
pub struct LatencyDistribution {
    /// Buckets: <1ms, 1-10ms, 10-50ms, 50-100ms, 100-500ms, 500ms-1s, >1s
    buckets: RwLock<[u64; 7]>,
    /// Raw latency samples for percentile calculation (limited to last N samples)
    samples: RwLock<Vec<f64>>,
    /// Maximum samples to keep
    max_samples: usize,
}

impl LatencyDistribution {
    pub fn new() -> Self {
        Self {
            buckets: RwLock::new([0; 7]),
            samples: RwLock::new(Vec::with_capacity(10000)),
            max_samples: 10000,
        }
    }

    /// Add a latency measurement in milliseconds
    pub fn add(&self, latency_ms: f64) {
        {
            let mut samples = self.samples.write();
            if samples.len() >= self.max_samples {
                // Remove oldest 10% when full
                let remove_count = self.max_samples / 10;
                samples.drain(0..remove_count);
            }
            samples.push(latency_ms);
        }

        let bucket_idx = if latency_ms < 1.0 {
            0
        } else if latency_ms < 10.0 {
            1
        } else if latency_ms < 50.0 {
            2
        } else if latency_ms < 100.0 {
            3
        } else if latency_ms < 500.0 {
            4
        } else if latency_ms < 1000.0 {
            5
        } else {
            6
        };

        self.buckets.write()[bucket_idx] += 1;
    }

    /// Calculate percentile from sorted samples
    fn percentile(sorted_samples: &[f64], p: f64) -> f64 {
        if sorted_samples.is_empty() {
            return 0.0;
        }
        let idx = ((sorted_samples.len() as f64 - 1.0) * p / 100.0).round() as usize;
        sorted_samples[idx.min(sorted_samples.len() - 1)]
    }

    /// Get distribution as labeled buckets with percentiles
    pub fn distribution(&self) -> LatencyDistributionSnapshot {
        let buckets = *self.buckets.read();
        let total: u64 = buckets.iter().sum();

        // Calculate percentiles from samples
        let (p50, p95, p99, max_latency, avg) = {
            let samples = self.samples.read();
            if samples.is_empty() {
                (0.0, 0.0, 0.0, 0.0, 0.0)
            } else {
                let mut sorted: Vec<f64> = samples.clone();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let sum: f64 = sorted.iter().sum();
                let avg = sum / sorted.len() as f64;
                let max = sorted.last().copied().unwrap_or(0.0);
                (
                    Self::percentile(&sorted, 50.0),
                    Self::percentile(&sorted, 95.0),
                    Self::percentile(&sorted, 99.0),
                    max,
                    avg,
                )
            }
        };

        LatencyDistributionSnapshot {
            buckets: vec![
                LatencyBucket {
                    label: "<1ms".to_string(),
                    count: buckets[0],
                    percentage: Self::pct(buckets[0], total),
                },
                LatencyBucket {
                    label: "1-10ms".to_string(),
                    count: buckets[1],
                    percentage: Self::pct(buckets[1], total),
                },
                LatencyBucket {
                    label: "10-50ms".to_string(),
                    count: buckets[2],
                    percentage: Self::pct(buckets[2], total),
                },
                LatencyBucket {
                    label: "50-100ms".to_string(),
                    count: buckets[3],
                    percentage: Self::pct(buckets[3], total),
                },
                LatencyBucket {
                    label: "100-500ms".to_string(),
                    count: buckets[4],
                    percentage: Self::pct(buckets[4], total),
                },
                LatencyBucket {
                    label: "500ms-1s".to_string(),
                    count: buckets[5],
                    percentage: Self::pct(buckets[5], total),
                },
                LatencyBucket {
                    label: ">1s".to_string(),
                    count: buckets[6],
                    percentage: Self::pct(buckets[6], total),
                },
            ],
            total,
            p50_ms: p50,
            p95_ms: p95,
            p99_ms: p99,
            max_ms: max_latency,
            avg_ms: avg,
        }
    }

    fn pct(count: u64, total: u64) -> f64 {
        if total == 0 {
            0.0
        } else {
            (count as f64 / total as f64) * 100.0
        }
    }

    /// Clear all data
    pub fn clear(&self) {
        *self.buckets.write() = [0; 7];
        self.samples.write().clear();
    }
}

impl Default for LatencyDistribution {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of latency distribution
#[derive(Debug, Clone, Serialize)]
pub struct LatencyDistributionSnapshot {
    pub buckets: Vec<LatencyBucket>,
    pub total: u64,
    /// 50th percentile latency in ms
    pub p50_ms: f64,
    /// 95th percentile latency in ms
    pub p95_ms: f64,
    /// 99th percentile latency in ms
    pub p99_ms: f64,
    /// Maximum latency in ms
    pub max_ms: f64,
    /// Average latency in ms
    pub avg_ms: f64,
}

/// A single latency bucket
#[derive(Debug, Clone, Serialize)]
pub struct LatencyBucket {
    pub label: String,
    pub count: u64,
    pub percentage: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_series_add() {
        let ts = TimeSeries::new(Duration::from_secs(60), Duration::from_secs(1));
        ts.add(1.0);
        ts.add(2.0);
        ts.add(3.0);

        assert_eq!(ts.count(), 3);
        assert_eq!(ts.sum(), 6.0);
    }

    #[test]
    fn test_time_series_rate() {
        // 100 events within the first second: the rate must reflect the
        // elapsed time, not the full window (which would report 10)
        let ts = TimeSeries::new(Duration::from_secs(10), Duration::from_secs(1));
        for _ in 0..100 {
            ts.increment();
        }

        let rate = ts.rate();
        assert!(rate > 50.0, "warm-up rate understated: {}", rate);
    }

    #[test]
    fn test_time_series_decays_when_idle() {
        // no new adds: old buckets must fall out of the window on reads
        let ts = TimeSeries::new(Duration::from_secs(1), Duration::from_secs(1));
        for _ in 0..50 {
            ts.increment();
        }
        assert!(ts.sum() > 0.0);

        std::thread::sleep(Duration::from_millis(2200));
        assert_eq!(ts.sum(), 0.0);
        assert_eq!(ts.rate(), 0.0);
    }

    #[test]
    fn test_latency_distribution() {
        let dist = LatencyDistribution::new();
        dist.add(0.5); // <1ms
        dist.add(5.0); // 1-10ms
        dist.add(25.0); // 10-50ms
        dist.add(75.0); // 50-100ms
        dist.add(200.0); // 100-500ms
        dist.add(750.0); // 500ms-1s
        dist.add(2000.0); // >1s

        let snapshot = dist.distribution();
        assert_eq!(snapshot.total, 7);
        assert_eq!(snapshot.buckets[0].count, 1);
        assert_eq!(snapshot.buckets[6].count, 1);
    }

    #[test]
    fn test_time_series_empty() {
        let ts = TimeSeries::new(Duration::from_secs(60), Duration::from_secs(1));

        assert_eq!(ts.count(), 0);
        assert_eq!(ts.sum(), 0.0);
        assert_eq!(ts.rate(), 0.0);
        assert!(ts.points().is_empty());
    }

    #[test]
    fn test_time_series_stats() {
        let ts = TimeSeries::new(Duration::from_secs(60), Duration::from_secs(1));
        ts.add(10.0);
        ts.add(20.0);
        ts.add(30.0);

        let stats = ts.stats();
        assert_eq!(stats.count, 3);
        assert_eq!(stats.sum, 60.0);
        assert_eq!(stats.avg, 20.0);
        assert_eq!(stats.min, 10.0);
        assert_eq!(stats.max, 30.0);
    }

    #[test]
    fn test_time_series_stats_empty() {
        let ts = TimeSeries::new(Duration::from_secs(60), Duration::from_secs(1));

        let stats = ts.stats();
        assert_eq!(stats.count, 0);
        assert_eq!(stats.sum, 0.0);
        assert_eq!(stats.avg, 0.0);
        assert_eq!(stats.min, 0.0);
        assert_eq!(stats.max, 0.0);
    }

    #[test]
    fn test_time_series_increment() {
        let ts = TimeSeries::new(Duration::from_secs(60), Duration::from_secs(1));

        for _ in 0..50 {
            ts.increment();
        }

        assert_eq!(ts.count(), 50);
        assert_eq!(ts.sum(), 50.0);
    }

    #[test]
    fn test_time_series_clear() {
        let ts = TimeSeries::new(Duration::from_secs(60), Duration::from_secs(1));
        ts.add(10.0);
        ts.add(20.0);

        assert_eq!(ts.count(), 2);

        ts.clear();

        assert_eq!(ts.count(), 0);
        assert_eq!(ts.sum(), 0.0);
    }

    #[test]
    fn test_time_series_averages() {
        let ts = TimeSeries::new(Duration::from_secs(60), Duration::from_secs(1));
        ts.add(10.0);
        ts.add(20.0);
        ts.add(30.0);

        let averages = ts.averages();
        assert!(!averages.is_empty());
        // All values in same bucket, average should be 20.0
        assert_eq!(averages[0].value, 20.0);
    }

    #[test]
    fn test_time_series_points() {
        let ts = TimeSeries::new(Duration::from_secs(60), Duration::from_secs(1));
        ts.add(5.0);
        ts.add(15.0);

        let points = ts.points();
        assert!(!points.is_empty());
        assert_eq!(points[0].value, 20.0); // Sum of values in bucket
    }

    #[test]
    fn test_time_series_default_qps() {
        let ts = TimeSeries::default_qps();
        ts.increment();
        assert_eq!(ts.count(), 1);
    }

    #[test]
    fn test_time_series_hourly() {
        let ts = TimeSeries::hourly();
        ts.add(100.0);
        assert_eq!(ts.sum(), 100.0);
    }

    #[test]
    fn test_latency_distribution_percentiles() {
        let dist = LatencyDistribution::new();

        // Add samples with known distribution
        for i in 1..=100 {
            dist.add(i as f64);
        }

        let snapshot = dist.distribution();
        assert_eq!(snapshot.total, 100);

        // P50 should be around 50
        assert!((snapshot.p50_ms - 50.0).abs() < 2.0);
        // P95 should be around 95
        assert!((snapshot.p95_ms - 95.0).abs() < 2.0);
        // P99 should be around 99
        assert!((snapshot.p99_ms - 99.0).abs() < 2.0);
        // Max should be 100
        assert_eq!(snapshot.max_ms, 100.0);
        // Avg should be 50.5
        assert!((snapshot.avg_ms - 50.5).abs() < 0.1);
    }

    #[test]
    fn test_latency_distribution_buckets() {
        let dist = LatencyDistribution::new();

        // Add values to specific buckets
        dist.add(0.1); // <1ms
        dist.add(0.5); // <1ms
        dist.add(5.0); // 1-10ms
        dist.add(15.0); // 10-50ms
        dist.add(30.0); // 10-50ms
        dist.add(75.0); // 50-100ms
        dist.add(300.0); // 100-500ms
        dist.add(600.0); // 500ms-1s
        dist.add(1500.0); // >1s

        let snapshot = dist.distribution();
        assert_eq!(snapshot.buckets[0].count, 2); // <1ms
        assert_eq!(snapshot.buckets[1].count, 1); // 1-10ms
        assert_eq!(snapshot.buckets[2].count, 2); // 10-50ms
        assert_eq!(snapshot.buckets[3].count, 1); // 50-100ms
        assert_eq!(snapshot.buckets[4].count, 1); // 100-500ms
        assert_eq!(snapshot.buckets[5].count, 1); // 500ms-1s
        assert_eq!(snapshot.buckets[6].count, 1); // >1s
    }

    #[test]
    fn test_latency_distribution_percentage() {
        let dist = LatencyDistribution::new();

        // 50 in first bucket, 50 in second bucket
        for _ in 0..50 {
            dist.add(0.5); // <1ms
        }
        for _ in 0..50 {
            dist.add(5.0); // 1-10ms
        }

        let snapshot = dist.distribution();
        assert!((snapshot.buckets[0].percentage - 50.0).abs() < 0.1);
        assert!((snapshot.buckets[1].percentage - 50.0).abs() < 0.1);
    }

    #[test]
    fn test_latency_distribution_empty() {
        let dist = LatencyDistribution::new();
        let snapshot = dist.distribution();

        assert_eq!(snapshot.total, 0);
        assert_eq!(snapshot.p50_ms, 0.0);
        assert_eq!(snapshot.p95_ms, 0.0);
        assert_eq!(snapshot.p99_ms, 0.0);
        assert_eq!(snapshot.max_ms, 0.0);
        assert_eq!(snapshot.avg_ms, 0.0);
    }

    #[test]
    fn test_latency_distribution_clear() {
        let dist = LatencyDistribution::new();

        for i in 0..100 {
            dist.add((i + 1) as f64);
        }

        assert_eq!(dist.distribution().total, 100);

        dist.clear();

        let snapshot = dist.distribution();
        assert_eq!(snapshot.total, 0);
        assert!(snapshot.buckets.iter().all(|b| b.count == 0));
    }

    #[test]
    fn test_latency_distribution_sample_pruning() {
        let dist = LatencyDistribution::new();

        // Add more than max_samples (10000)
        for i in 0..15000 {
            dist.add((i % 1000) as f64);
        }

        // Distribution should still work correctly
        let snapshot = dist.distribution();
        assert!(snapshot.total > 0);
        assert!(snapshot.avg_ms > 0.0);
    }

    #[test]
    fn test_bucket_operations() {
        let mut bucket = Bucket::new(100);

        assert_eq!(bucket.timestamp, 100);
        assert_eq!(bucket.count, 0);
        assert_eq!(bucket.sum, 0.0);
        assert_eq!(bucket.average(), 0.0);

        bucket.add(10.0);
        bucket.add(20.0);
        bucket.add(30.0);

        assert_eq!(bucket.count, 3);
        assert_eq!(bucket.sum, 60.0);
        assert_eq!(bucket.average(), 20.0);
        assert_eq!(bucket.min, 10.0);
        assert_eq!(bucket.max, 30.0);
    }

    #[test]
    fn test_time_series_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let ts = Arc::new(TimeSeries::new(
            Duration::from_secs(60),
            Duration::from_secs(1),
        ));
        let mut handles = vec![];

        for _ in 0..10 {
            let ts_clone = Arc::clone(&ts);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    ts_clone.increment();
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(ts.count(), 1000);
    }
}
