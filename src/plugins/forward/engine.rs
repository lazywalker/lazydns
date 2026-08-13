use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use reqwest::Client as HttpClient;
use tokio::net::UdpSocket;
use tokio::sync::{OnceCell, oneshot};
use tracing::{trace, warn};

use super::types::{LoadBalanceStrategy, PendingQuery, UdpMuxState, Upstream};
use crate::Result;
use crate::dns::Message;

/// Core forwarding engine: upstream selection, UDP/DoH transport, timeout.
#[derive(Debug, Clone)]
pub struct Forward {
    pub(crate) upstreams: Vec<Upstream>,
    pub(crate) timeout: Duration,
    pub(crate) strategy: LoadBalanceStrategy,
    pub(crate) health_checks_enabled: bool,
    pub(crate) max_attempts: usize,
    doh_client: Arc<OnceCell<HttpClient>>,
    udp_mux: Arc<OnceCell<Arc<UdpMuxState>>>,
    accept_invalid_certs: bool,
}

impl Forward {
    pub(crate) fn new(
        upstreams: Vec<Upstream>,
        timeout: Duration,
        strategy: LoadBalanceStrategy,
    ) -> Self {
        let accept_invalid_certs =
            cfg!(test) || std::env::var("LAZYDNS_DOH_ACCEPT_INVALID_CERT").is_ok();
        Self {
            upstreams,
            timeout,
            strategy,
            health_checks_enabled: false,
            max_attempts: 3,
            doh_client: Arc::new(OnceCell::new()),
            udp_mux: Arc::new(OnceCell::new()),
            accept_invalid_certs,
        }
    }

    pub(crate) fn with_health_checks(mut self, enabled: bool) -> Self {
        self.health_checks_enabled = enabled;
        self
    }

    pub(crate) fn with_max_attempts(mut self, max: usize) -> Self {
        self.max_attempts = max;
        self
    }

    /// Record a query outcome (success or failure) on the upstream's health
    /// stats and metrics. No-op when health checks are disabled.
    pub(crate) fn record_outcome(&self, upstream: &Upstream, elapsed: Duration, success: bool) {
        if !self.health_checks_enabled {
            return;
        }
        if success {
            upstream.health.record_success(elapsed);
            #[cfg(feature = "metrics")]
            {
                use crate::metrics::{UPSTREAM_DURATION_SECONDS, UPSTREAM_QUERIES_TOTAL};
                UPSTREAM_QUERIES_TOTAL
                    .with_label_values(&[upstream.addr.as_str(), "success"])
                    .inc();
                UPSTREAM_DURATION_SECONDS
                    .with_label_values(&[upstream.addr.as_str()])
                    .observe(elapsed.as_secs_f64());
            }
        } else {
            upstream.health.record_failure();
            #[cfg(feature = "metrics")]
            {
                use crate::metrics::UPSTREAM_QUERIES_TOTAL;
                UPSTREAM_QUERIES_TOTAL
                    .with_label_values(&[upstream.addr.as_str(), "error"])
                    .inc();
            }
        }
    }

    /// Select upstream index by strategy.
    /// Fastest picks the upstream with lowest measured avg response time.
    pub(crate) fn select_upstream(&self, current_idx: usize) -> Option<usize> {
        if self.upstreams.is_empty() {
            return None;
        }

        match self.strategy {
            LoadBalanceStrategy::RoundRobin => Some(current_idx % self.upstreams.len()),
            LoadBalanceStrategy::Random => {
                use std::time::SystemTime;
                let nanos = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                Some((nanos as usize) % self.upstreams.len())
            }
            LoadBalanceStrategy::Fastest => {
                let mut best_idx = 0;
                let mut best_time = self.upstreams[0].health.avg_response_time();

                for (idx, upstream) in self.upstreams.iter().enumerate().skip(1) {
                    let avg_time = upstream.health.avg_response_time();

                    if best_time == Duration::ZERO {
                        continue;
                    }
                    if avg_time == Duration::ZERO || avg_time < best_time {
                        best_idx = idx;
                        best_time = avg_time;
                    }
                }
                Some(best_idx)
            }
        }
    }

    /// Dispatch query to DoH (http/https) or UDP based on address scheme.
    pub(crate) async fn forward_query(
        &self,
        request: &Message,
        upstream: &Upstream,
    ) -> Result<Message> {
        if upstream.addr.starts_with("http://") || upstream.addr.starts_with("https://") {
            self.forward_query_doh(request, &upstream.addr).await
        } else {
            self.forward_query_udp(request, &upstream.addr).await
        }
    }

    /// UDP forward with qid multiplexing on a shared socket.
    /// Avoids cross-request response pollution under concurrency.
    async fn forward_query_udp(&self, request: &Message, upstream: &str) -> Result<Message> {
        let upstream_addr = SocketAddr::from_str(upstream)
            .map_err(|e| crate::Error::Config(format!("Invalid upstream address: {}", e)))?;

        let mux = self
            .udp_mux
            .get_or_try_init(|| async {
                let socket = UdpSocket::bind("0.0.0.0:0").await?;
                let state = Arc::new(UdpMuxState {
                    socket,
                    pending: DashMap::new(),
                    next_qid: AtomicU16::new(1),
                });
                let state_clone = Arc::clone(&state);
                tokio::spawn(Self::read_loop(state_clone));
                Ok::<_, crate::Error>(state)
            })
            .await?;

        let assigned_qid = loop {
            let qid = mux.next_qid.fetch_add(1, Ordering::Relaxed);
            if qid != 0 && !mux.pending.contains_key(&qid) {
                break qid;
            }
        };

        let original_qid = request.id();
        let mut request_data = crate::dns::wire::serialize_message(request)?;
        request_data[0] = (assigned_qid >> 8) as u8;
        request_data[1] = (assigned_qid & 0xFF) as u8;

        let (tx, mut rx) = oneshot::channel();
        mux.pending.insert(
            assigned_qid,
            PendingQuery {
                peer: upstream_addr,
                tx,
            },
        );

        let sent = mux.socket.send_to(&request_data, upstream_addr).await?;
        trace!(
            "Sent {} bytes to {} (qid {} remapped to {})",
            sent, upstream_addr, original_qid, assigned_qid
        );

        // biased select: response wins over timeout at race edges
        tokio::select! {
            biased;
            received = &mut rx => {
                match received {
                    Ok(mut response) => {
                        response.set_id(original_qid);
                        Ok(response)
                    }
                    Err(_) => Err(crate::Error::Connection {
                        address: upstream_addr.to_string(),
                        reason: "response channel closed".to_string(),
                    })
                }
            }
            _ = tokio::time::sleep(self.timeout) => {
                mux.pending.remove(&assigned_qid);
                warn!("Timeout waiting for response from {}", upstream_addr);
                Err(crate::Error::UpstreamTimeout {
                    upstream: upstream_addr.to_string(),
                    timeout_ms: self.timeout.as_millis() as u64,
                })
            }
        }
    }

    /// Background loop: demultiplex UDP responses by qid to the correct waiter.
    async fn read_loop(state: Arc<UdpMuxState>) {
        loop {
            let mut buf = vec![0u8; 4096];
            match state.socket.recv_from(&mut buf).await {
                Ok((len, addr)) => match crate::dns::wire::parse_message(&buf[..len]) {
                    Ok(response) => {
                        let qid = response.id();
                        // copy peer out first: remove on a held guard deadlocks
                        let expected_peer = state.pending.get(&qid).map(|e| e.value().peer);
                        match expected_peer {
                            // only the queried upstream may answer; a guessed
                            // qid from another source is a spoofing attempt
                            Some(peer) if addr == peer => {
                                if let Some((_, pending)) = state.pending.remove(&qid) {
                                    let _ = pending.tx.send(response);
                                }
                            }
                            Some(peer) => {
                                warn!(
                                    "DNS response qid {} from {} does not match upstream {}, ignored",
                                    qid, addr, peer
                                );
                            }
                            None => {}
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse DNS response from {}: {}", addr, e);
                    }
                },
                Err(e) => {
                    warn!("UDP recv error: {}", e);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// DoH forward via shared HTTP client (connection pooling).
    async fn forward_query_doh(&self, request: &Message, upstream_url: &str) -> Result<Message> {
        let accept_invalid = self.accept_invalid_certs;
        let client = self
            .doh_client
            .get_or_try_init(|| async {
                let mut builder = HttpClient::builder()
                    .pool_max_idle_per_host(10)
                    .pool_idle_timeout(Duration::from_secs(90));
                if accept_invalid {
                    builder = builder.danger_accept_invalid_certs(true);
                }
                builder
                    .build()
                    .map_err(|e| crate::Error::Other(e.to_string()))
            })
            .await?;

        let request_data = crate::dns::wire::serialize_message(request)?;

        let resp = tokio::time::timeout(self.timeout, async {
            client
                .post(upstream_url)
                .header("Content-Type", "application/dns-message")
                .body(request_data)
                .send()
                .await
        })
        .await
        .map_err(|_| crate::Error::UpstreamTimeout {
            upstream: upstream_url.to_string(),
            timeout_ms: self.timeout.as_millis() as u64,
        })?
        .map_err(|e| crate::Error::Other(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(crate::Error::Other(format!(
                "DoH upstream returned: {}",
                resp.status()
            )));
        }

        let bytes = tokio::time::timeout(self.timeout, resp.bytes())
            .await
            .map_err(|_| crate::Error::UpstreamTimeout {
                upstream: upstream_url.to_string(),
                timeout_ms: self.timeout.as_millis() as u64,
            })?
            .map_err(|e| crate::Error::Other(e.to_string()))?;

        crate::dns::wire::parse_message(&bytes)
    }
}
