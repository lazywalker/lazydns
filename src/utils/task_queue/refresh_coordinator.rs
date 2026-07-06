//! Refresh coordinator for background cache refresh operations
//!
//! Manages a worker pool to execute cache refresh tasks with:
//! - Deduplication (same key won't refresh twice concurrently)
//! - Bounded queue with backpressure
//! - Timeout protection
//! - Comprehensive statistics

use super::EnqueueError;
use crate::dns::Message;
use crate::plugin::PluginHandler;
use crate::server::{Protocol, RequestContext, RequestHandler};
use dashmap::DashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, trace, warn};

use super::stats::RefreshStats;

/// Task to refresh a cache entry in the background
#[derive(Clone)]
pub struct RefreshTask {
    /// Cache key being refreshed
    pub key: String,
    /// DNS query message
    pub message: Message,
    /// Plugin handler to execute the query
    pub handler: Arc<PluginHandler>,
    /// Entry name for the handler
    pub entry_name: String,
    /// When this task was created
    pub created_at: Instant,
}

/// Coordinator for cache refresh operations
///
/// Design goals:
/// - Replace unbounded tokio::spawn with bounded worker pool
/// - Deduplicate concurrent refreshes for same key
/// - Provide backpressure when queue is full
/// - Track comprehensive statistics
///
/// Future extensibility:
/// - Can be generalized to handle file reload tasks
/// - Can support priority queues
/// - Can add adaptive worker scaling
//
// Completion callback type: invoked with (key, success) after each task
// finishes. Used by the cache plugin to remove the key from its
// `refreshing_keys` set so the key can be refreshed again later.
type CompletionCallback = dyn Fn(&str, bool) + Send + Sync;

pub struct RefreshCoordinator {
    /// Channel sender for enqueueing tasks
    tx: mpsc::Sender<RefreshTask>,
    /// Set of keys currently being processed (for deduplication)
    processing: Arc<DashSet<String>>,
    /// Statistics tracker
    stats: Arc<RefreshStats>,
    /// Worker task handles for graceful shutdown
    worker_handles: Arc<tokio::sync::Mutex<Vec<JoinHandle<()>>>>,
}

impl RefreshCoordinator {
    /// Create a new refresh coordinator with worker pool
    ///
    /// # Arguments
    /// * `worker_count` - Number of background workers
    /// * `queue_capacity` - Maximum pending tasks in queue
    ///
    /// # Returns
    /// Self with running worker pool
    pub fn new(worker_count: usize, queue_capacity: usize) -> Self {
        Self::new_with_callback(worker_count, queue_capacity, None)
    }

    /// Create a coordinator with a completion callback.
    ///
    /// The callback is invoked after each task finishes processing (regardless
    /// of outcome) with the task key and a `success` flag. The cache plugin
    /// uses this to remove the key from its `refreshing_keys` set so that
    /// subsequent background refreshes are not permanently blocked.
    pub fn new_with_callback(
        worker_count: usize,
        queue_capacity: usize,
        on_complete: Option<Arc<CompletionCallback>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(queue_capacity);
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        let processing = Arc::new(DashSet::new());
        let stats = Arc::new(RefreshStats::new());
        // Default to a no-op callback so worker_loop can always invoke it.
        let on_complete: Arc<CompletionCallback> =
            on_complete.unwrap_or_else(|| Arc::new(|_, _| {}));
        let mut handles = Vec::with_capacity(worker_count);

        debug!(
            worker_count = worker_count,
            queue_capacity = queue_capacity,
            "Starting refresh coordinator worker pool"
        );

        // Spawn worker pool
        for worker_id in 0..worker_count {
            let rx_clone = Arc::clone(&rx);
            let processing_clone = Arc::clone(&processing);
            let stats_clone = Arc::clone(&stats);
            let on_complete_clone = Arc::clone(&on_complete);

            let handle = tokio::spawn(async move {
                Self::worker_loop(
                    worker_id,
                    rx_clone,
                    processing_clone,
                    stats_clone,
                    on_complete_clone,
                )
                .await;
            });

            handles.push(handle);
        }

        Self {
            tx,
            processing,
            stats,
            worker_handles: Arc::new(tokio::sync::Mutex::new(handles)),
        }
    }

    /// Try to enqueue a refresh task
    ///
    /// # Deduplication
    /// If the same key is already being processed, the task is rejected with
    /// dedup_skipped counter incremented.
    ///
    /// # Backpressure
    /// If the queue is full, the task is rejected with rejected counter incremented.
    ///
    /// # Arguments
    /// * `task` - Refresh task to enqueue
    ///
    /// # Returns
    /// Ok(()) if enqueued successfully, Err if rejected
    pub async fn enqueue(&self, task: RefreshTask) -> crate::Result<()> {
        // Check if already processing this key
        if !self.processing.insert(task.key.clone()) {
            trace!(key = %task.key, "Refresh already in progress, skipping duplicate");
            self.stats.record_dedup_skipped();
            return Err(EnqueueError::AlreadyProcessing.into());
        }

        // Try to send to queue (non-blocking)
        let key_for_cleanup = task.key.clone();
        match self.tx.try_send(task) {
            Ok(_) => {
                self.stats.record_enqueued();
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Queue full, remove from processing set
                self.processing.remove(&key_for_cleanup);
                self.stats.record_rejected();
                warn!(
                    key = %key_for_cleanup,
                    queue_depth = self.stats.queue_depth(),
                    "Refresh queue full, rejecting task"
                );
                Err(EnqueueError::QueueFull.into())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // Channel closed (should not happen in normal operation)
                self.processing.remove(&key_for_cleanup);
                warn!("Refresh coordinator channel closed");
                Err(EnqueueError::Closed.into())
            }
        }
    }
    /// Get statistics reference
    pub fn stats(&self) -> Arc<RefreshStats> {
        Arc::clone(&self.stats)
    }

    /// Gracefully shutdown the coordinator and wait for all workers to complete
    ///
    /// This method:
    /// 1. Closes the task channel (signaling workers to exit)
    /// 2. Waits for all worker tasks to complete
    /// 3. Allows pending tasks in queue to be processed before exit
    ///
    /// # Returns
    /// Ok(()) on successful shutdown, Err if worker join fails
    pub async fn shutdown(self) -> crate::Result<()> {
        debug!("Shutting down refresh coordinator");

        // Drop the sender to close the channel
        // This signals workers that no more tasks will arrive
        drop(self.tx);

        // Get and wait for all worker handles to complete
        let mut handles_guard = self.worker_handles.lock().await;
        let handles = std::mem::take(&mut *handles_guard);
        drop(handles_guard);

        // Wait for all workers to complete
        for handle in handles {
            match handle.await {
                Ok(_) => {
                    trace!("Worker task completed successfully");
                }
                Err(e) => {
                    warn!("Worker task panicked: {}", e);
                }
            }
        }

        debug!("Refresh coordinator shutdown complete");
        Ok(())
    }

    /// Worker loop - processes tasks from queue
    async fn worker_loop(
        worker_id: usize,
        rx: Arc<tokio::sync::Mutex<mpsc::Receiver<RefreshTask>>>,
        processing: Arc<DashSet<String>>,
        stats: Arc<RefreshStats>,
        on_complete: Arc<CompletionCallback>,
    ) {
        trace!(worker_id = worker_id, "Refresh worker started");

        loop {
            // Lock receiver and wait for next task
            let task = {
                let mut rx_guard = rx.lock().await;
                rx_guard.recv().await
            };

            match task {
                Some(task) => {
                    let start = Instant::now();
                    let key = task.key.clone();
                    let queued_duration = start.duration_since(task.created_at);

                    trace!(
                        worker_id = worker_id,
                        key = %key,
                        queued_ms = queued_duration.as_millis(),
                        "Processing refresh task"
                    );

                    // Execute task with timeout
                    const REFRESH_TIMEOUT: Duration = Duration::from_secs(10);
                    let result =
                        tokio::time::timeout(REFRESH_TIMEOUT, Self::execute_task(&task)).await;

                    let duration = start.elapsed();
                    stats.record_processed();

                    let success = match result {
                        Ok(Ok(_)) => {
                            stats.record_success();
                            debug!(
                                worker_id = worker_id,
                                key = %key,
                                duration_ms = duration.as_millis(),
                                "Refresh succeeded"
                            );
                            true
                        }
                        Ok(Err(e)) => {
                            stats.record_failed();
                            debug!(
                                worker_id = worker_id,
                                key = %key,
                                duration_ms = duration.as_millis(),
                                error = %e,
                                "Refresh failed"
                            );
                            false
                        }
                        Err(_) => {
                            stats.record_timeout();
                            warn!(
                                worker_id = worker_id,
                                key = %key,
                                timeout_secs = REFRESH_TIMEOUT.as_secs(),
                                "Refresh timeout"
                            );
                            false
                        }
                    };

                    // Remove from processing set
                    processing.remove(&key);

                    // Notify completion callback so callers (e.g. CachePlugin)
                    // can clean up their own dedup state. This runs for every
                    // outcome (success/failure/timeout) to guarantee no key is
                    // left permanently locked in the caller's dedup set.
                    on_complete(&key, success);
                }
                None => {
                    // Channel closed, worker exits
                    debug!(
                        worker_id = worker_id,
                        "Refresh worker stopping (channel closed)"
                    );
                    break;
                }
            }
        }

        trace!(worker_id = worker_id, "Refresh worker stopped");
    }

    /// Execute a single refresh task
    async fn execute_task(task: &RefreshTask) -> crate::Result<()> {
        trace!(key = %task.key, "Executing refresh query");

        let ctx = RequestContext::new(task.message.clone(), Protocol::Udp);

        match task.handler.handle(ctx).await {
            Ok(response) => {
                if response.response_code() == crate::dns::ResponseCode::NoError {
                    trace!(key = %task.key, "Refresh query returned NoError");
                    Ok(())
                } else {
                    trace!(
                        key = %task.key,
                        rcode = ?response.response_code(),
                        "Refresh query returned error response"
                    );
                    Err(crate::Error::Plugin(format!(
                        "Response code: {:?}",
                        response.response_code()
                    )))
                }
            }
            Err(e) => {
                trace!(key = %task.key, error = %e, "Refresh query failed");
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_refresh_coordinator_shutdown() {
        // Create coordinator with small pool
        let coordinator = RefreshCoordinator::new(2, 10);

        // Verify coordinator is running
        {
            let handles = coordinator.worker_handles.lock().await;
            assert_eq!(handles.len(), 2, "Should have 2 worker handles");
        }

        // Shutdown coordinator
        let result = coordinator.shutdown().await;
        assert!(result.is_ok(), "Shutdown should succeed");

        debug!("Test passed: coordinator shutdown successful");
    }

    #[tokio::test]
    async fn test_refresh_coordinator_created() {
        let coordinator = RefreshCoordinator::new(1, 10);

        // Verify stats reference is available
        let stats = coordinator.stats();
        assert!(stats.enqueued.load(std::sync::atomic::Ordering::Relaxed) == 0);

        // Shutdown
        let _ = coordinator.shutdown().await;
        debug!("Test passed: coordinator created and shutdown successfully");
    }

    #[tokio::test]
    async fn test_refresh_coordinator_multiple_workers() {
        let coordinator = RefreshCoordinator::new(4, 100);

        {
            let handles = coordinator.worker_handles.lock().await;
            assert_eq!(handles.len(), 4, "Should have 4 worker handles");
        }

        let _ = coordinator.shutdown().await;
    }

    #[tokio::test]
    async fn test_stats_initial_values() {
        let coordinator = RefreshCoordinator::new(1, 10);
        let stats = coordinator.stats();

        assert_eq!(stats.total_enqueued(), 0);
        assert_eq!(stats.total_processed(), 0);
        assert_eq!(stats.total_failed(), 0);
        assert_eq!(stats.total_rejected(), 0);
        assert_eq!(stats.total_dedup_skipped(), 0);

        let _ = coordinator.shutdown().await;
    }

    #[tokio::test]
    async fn test_stats_arc_clone() {
        let coordinator = RefreshCoordinator::new(1, 10);
        let stats1 = coordinator.stats();
        let stats2 = coordinator.stats();

        // Both should point to the same underlying stats
        assert_eq!(Arc::strong_count(&stats1), Arc::strong_count(&stats2));

        let _ = coordinator.shutdown().await;
    }
}
