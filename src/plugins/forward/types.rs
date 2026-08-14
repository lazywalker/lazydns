use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::time::Duration;
use tokio::time::Instant;

use dashmap::DashMap;
use tokio::net::UdpSocket;
use tokio::sync::oneshot;

use crate::dns::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadBalanceStrategy {
    RoundRobin,
    Random,
    Fastest,
}

/// Atomic health counters for a single upstream.
#[derive(Debug)]
pub struct UpstreamHealth {
    pub(crate) queries: AtomicU64,
    pub(crate) successes: AtomicU64,
    pub(crate) failures: AtomicU64,
    pub(crate) avg_response_time_us: AtomicU64,
    pub(crate) last_success: parking_lot::Mutex<Option<Instant>>,
}

impl UpstreamHealth {
    pub fn new() -> Self {
        Self {
            queries: AtomicU64::new(0),
            successes: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            avg_response_time_us: AtomicU64::new(0),
            last_success: parking_lot::Mutex::new(None),
        }
    }

    pub fn record_success(&self, response_time: Duration) {
        self.queries.fetch_add(1, Ordering::Relaxed);
        let success_count = self.successes.fetch_add(1, Ordering::Relaxed);

        let new_time = response_time.as_micros() as u64;
        self.avg_response_time_us
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old_avg| {
                Some(if success_count == 0 {
                    new_time
                } else {
                    (old_avg * success_count + new_time) / (success_count + 1)
                })
            })
            .ok();

        *self.last_success.lock() = Some(Instant::now());
    }

    pub fn record_failure(&self) {
        self.queries.fetch_add(1, Ordering::Relaxed);
        self.failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn success_rate(&self) -> f64 {
        let total = self.queries.load(Ordering::Relaxed);
        if total == 0 {
            return 1.0;
        }
        let successes = self.successes.load(Ordering::Relaxed);
        successes as f64 / total as f64
    }

    pub fn counters(&self) -> (u64, u64, u64) {
        (
            self.queries.load(Ordering::Relaxed),
            self.successes.load(Ordering::Relaxed),
            self.failures.load(Ordering::Relaxed),
        )
    }

    pub fn avg_response_time(&self) -> Duration {
        Duration::from_micros(self.avg_response_time_us.load(Ordering::Relaxed))
    }

    pub(crate) fn avg_response_time_us_raw(&self) -> u64 {
        self.avg_response_time_us.load(Ordering::Relaxed)
    }

    #[cfg(feature = "web")]
    pub(crate) fn last_success_at(&self) -> Option<Instant> {
        *self.last_success.lock()
    }
}

impl Default for UpstreamHealth {
    fn default() -> Self {
        Self::new()
    }
}

/// One configured upstream server with its health tracker.
#[derive(Debug, Clone)]
pub struct Upstream {
    pub(crate) addr: String,
    #[cfg_attr(not(feature = "web"), allow(dead_code))]
    pub(crate) tag: Option<String>,
    pub(crate) health: Arc<UpstreamHealth>,
}

impl Upstream {
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            tag: None,
            health: Arc::new(UpstreamHealth::new()),
        }
    }

    pub fn with_tag(addr: impl Into<String>, tag: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            tag: Some(tag.into()),
            health: Arc::new(UpstreamHealth::new()),
        }
    }
}

/// A query in flight on the shared UDP socket, awaiting its response.
#[derive(Debug)]
pub(crate) struct PendingQuery {
    /// upstream the query was sent to; responses from any other source are
    /// dropped (guessed-qid spoofing)
    pub(crate) peer: std::net::SocketAddr,
    pub(crate) tx: oneshot::Sender<Message>,
}

/// Shared UDP socket state for qid-based response multiplexing.
#[derive(Debug)]
pub(crate) struct UdpMuxState {
    pub(crate) socket: UdpSocket,
    pub(crate) pending: DashMap<u16, PendingQuery>,
    pub(crate) next_qid: AtomicU16,
}
