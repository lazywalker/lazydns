use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_yaml::Value;
use std::any::Any;

use crate::RegisterPlugin;
use crate::Result;
use crate::config::PluginConfig;
use crate::dns::Message;
use crate::plugin::{Context, Plugin};
use tracing::{debug, warn};

mod builder;
mod engine;
mod types;

pub use types::{LoadBalanceStrategy, Upstream, UpstreamHealth};

use builder::ForwardBuilder;
use engine::Forward;

#[derive(Debug, RegisterPlugin)]
pub struct ForwardPlugin {
    core: Forward,
    /// Round-robin counter for upstream selection.
    current: AtomicUsize,
    /// Race multiple upstreams concurrently; first response wins.
    concurrent_queries: bool,
    tag: Option<String>,
}

impl ForwardPlugin {
    pub fn new(upstreams: Vec<String>) -> Self {
        let ups: Vec<Upstream> = upstreams
            .into_iter()
            .map(|entry| {
                if let Some((addr, tag)) = entry.split_once('|') {
                    Upstream::with_tag(addr.to_string(), tag.to_string())
                } else {
                    Upstream::new(entry)
                }
            })
            .collect();

        let core = Forward::new(ups, Duration::from_secs(5), LoadBalanceStrategy::RoundRobin)
            .with_health_checks(false)
            .with_max_attempts(3);

        ForwardPlugin {
            core,
            current: AtomicUsize::new(0),
            concurrent_queries: false,
            tag: None,
        }
    }

    pub fn upstream_addrs(&self) -> Vec<String> {
        self.core.upstreams.iter().map(|u| u.addr.clone()).collect()
    }

    fn select_upstream(&self) -> Option<usize> {
        let idx = self.current.fetch_add(1, Ordering::Relaxed);
        self.core.select_upstream(idx)
    }

    fn record_upstream_health(&self, upstream: &Upstream, elapsed: Duration, success: bool) {
        if !self.core.health_checks_enabled {
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

    fn extract_answer_addresses(response: &Message) -> Vec<String> {
        response
            .answers()
            .iter()
            .filter_map(|rr| match rr.rdata() {
                crate::dns::RData::A(ipv4) => Some(ipv4.to_string()),
                crate::dns::RData::AAAA(ipv6) => Some(ipv6.to_string()),
                _ => None,
            })
            .collect()
    }

    async fn forward_query_with_health(
        &self,
        request: &Message,
        upstream_idx: usize,
    ) -> Result<Message> {
        let upstream = &self.core.upstreams[upstream_idx];
        let start = std::time::Instant::now();

        match self.core.forward_query(request, upstream).await {
            Ok(response) => {
                let elapsed = start.elapsed();
                self.record_upstream_health(upstream, elapsed, true);

                let (queries, successes, failures) = upstream.health.counters();
                let addrs = Self::extract_answer_addresses(&response);

                debug!(
                    upstream = upstream.addr.as_str(),
                    elapsed_ms = elapsed.as_millis(),
                    queries, successes, failures,
                    avg_resp_us = upstream.health.avg_response_time_us_raw(),
                    addrs = ?addrs,
                    "Query succeeded"
                );
                Ok(response)
            }
            Err(e) => {
                self.record_upstream_health(upstream, start.elapsed(), false);
                let (queries, successes, failures) = upstream.health.counters();
                warn!(
                    upstream = upstream.addr.as_str(),
                    error = %e, queries, successes, failures,
                    "Query failed"
                );
                Err(e)
            }
        }
    }

    /// Race all upstreams concurrently, return the first success.
    /// Tasks are awaited in completion order; losers are aborted.
    async fn execute_concurrent(&self, request: Arc<Message>) -> Result<Message> {
        use tokio::task::JoinSet;

        let mut set: JoinSet<Result<Message>> = JoinSet::new();

        for idx in 0..self.core.upstreams.len() {
            let req = Arc::clone(&request);
            let core = self.core.clone();

            set.spawn(async move {
                let upstream = &core.upstreams[idx];
                let start = std::time::Instant::now();

                match core.forward_query(&req, upstream).await {
                    Ok(response) => {
                        let elapsed = start.elapsed();
                        if core.health_checks_enabled {
                            upstream.health.record_success(elapsed);
                            #[cfg(feature = "metrics")]
                            {
                                use crate::metrics::{
                                    UPSTREAM_DURATION_SECONDS, UPSTREAM_QUERIES_TOTAL,
                                };
                                UPSTREAM_QUERIES_TOTAL
                                    .with_label_values(&[upstream.addr.as_str(), "success"])
                                    .inc();
                                UPSTREAM_DURATION_SECONDS
                                    .with_label_values(&[upstream.addr.as_str()])
                                    .observe(elapsed.as_secs_f64());
                            }
                        }
                        Ok(response)
                    }
                    Err(e) => {
                        if core.health_checks_enabled {
                            upstream.health.record_failure();
                            #[cfg(feature = "metrics")]
                            {
                                use crate::metrics::UPSTREAM_QUERIES_TOTAL;
                                UPSTREAM_QUERIES_TOTAL
                                    .with_label_values(&[upstream.addr.as_str(), "error"])
                                    .inc();
                            }
                        }
                        Err(e)
                    }
                }
            });
        }

        let mut last_error: Option<crate::Error> = None;
        while let Some(res) = set.join_next().await {
            match res {
                Ok(Ok(response)) => {
                    set.abort_all();
                    return Ok(response);
                }
                Ok(Err(e)) => last_error = Some(e),
                Err(join_err) => {
                    last_error = Some(crate::Error::Other(format!(
                        "concurrent task failed: {join_err}"
                    )));
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| crate::Error::Other("All concurrent queries failed".to_string())))
    }

    /// Try upstreams one at a time, rotating via strategy on failure.
    async fn execute_sequential(&self, ctx: &mut Context, request: &Message) -> Result<()> {
        let mut attempts = 0;
        let mut last_error = None;

        while attempts < self.core.max_attempts && attempts < self.core.upstreams.len() {
            let upstream_idx = match self.select_upstream() {
                Some(idx) => idx,
                None => {
                    return Err(crate::Error::Config(
                        "No upstream servers configured".to_string(),
                    ));
                }
            };

            match self.forward_query_with_health(request, upstream_idx).await {
                Ok(response) => {
                    ctx.set_response(Some(response));
                    return Ok(());
                }
                Err(e) => {
                    #[cfg(feature = "web")]
                    if let Some(q) = request.questions().first() {
                        let qname = q.qname().to_string();
                        let client_ip = ctx.get_metadata::<std::net::IpAddr>("client_ip").copied();
                        match &e {
                            crate::Error::UpstreamTimeout {
                                upstream,
                                timeout_ms,
                            } => {
                                crate::plugins::AUDIT_LOGGER
                                    .log_security_event(
                                        crate::plugins::SecurityEventType::QueryTimeout,
                                        format!("Timeout: {} ({} ms)", upstream, timeout_ms),
                                        client_ip,
                                        Some(qname),
                                    )
                                    .await;
                            }
                            _ => {
                                crate::plugins::AUDIT_LOGGER
                                    .log_security_event(
                                        crate::plugins::SecurityEventType::UpstreamFailure,
                                        format!(
                                            "Failed: {}: {}",
                                            self.core.upstreams[upstream_idx].addr, e
                                        ),
                                        client_ip,
                                        Some(qname.clone()),
                                    )
                                    .await;
                            }
                        }
                    }

                    last_error = Some(e);
                    attempts += 1;
                    if !self.core.health_checks_enabled {
                        break;
                    }
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| crate::Error::Other("All upstream servers failed".to_string())))
    }
}

impl Deref for ForwardPlugin {
    type Target = Forward;
    fn deref(&self) -> &Forward {
        &self.core
    }
}

#[async_trait]
impl Plugin for ForwardPlugin {
    fn init(config: &PluginConfig) -> Result<Arc<dyn Plugin>> {
        let args = config.effective_args();
        let core = ForwardBuilder::from_args(&args)?;

        // concurrent > 1 enables race mode (legacy behavior)
        let concurrent = match args.get("concurrent") {
            Some(Value::Number(n)) => n.as_i64().unwrap_or(1) > 1,
            _ => false,
        };

        let _plugin_tag = config.tag.clone().unwrap_or_else(|| "forward".to_string());

        let plugin = ForwardPlugin {
            core,
            current: AtomicUsize::new(0),
            concurrent_queries: concurrent,
            tag: config.tag.clone(),
        };

        #[cfg(feature = "web")]
        {
            for upstream in &plugin.core.upstreams {
                let key = format!("{}:{}", _plugin_tag, upstream.addr);
                let address = upstream.addr.clone();
                let tag = upstream.tag.clone();
                let plugin_name = _plugin_tag.clone();
                let health = Arc::clone(&upstream.health);

                crate::web::upstream_registry::register_upstream(
                    key,
                    address,
                    tag,
                    plugin_name,
                    move || {
                        let (queries, successes, failures) = health.counters();
                        crate::web::upstream_registry::UpstreamHealthData {
                            queries,
                            successes,
                            failures,
                            avg_response_time_us: health.avg_response_time_us_raw(),
                            last_success: health.last_success_at(),
                        }
                    },
                );
            }
        }

        Ok(Arc::new(plugin))
    }

    async fn execute(&self, ctx: &mut Context) -> Result<()> {
        if ctx.has_response() {
            return Ok(());
        }

        let request_arc = Arc::new(ctx.request().clone());

        if self.concurrent_queries
            && self.core.upstreams.len() > 1
            && let Ok(response) = self.execute_concurrent(Arc::clone(&request_arc)).await
        {
            ctx.set_response(Some(response));
            return Ok(());
        }

        self.execute_sequential(ctx, &request_arc).await
    }

    fn name(&self) -> &str {
        "forward"
    }
    fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::types::{RecordClass, RecordType};
    use crate::dns::{Message, Question, RData, ResourceRecord};
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, UdpSocket};

    // ============ Tests from core Forward logic ============

    /// Read a DoH POST body from `stream`.
    async fn read_doh_request<R: AsyncRead + Unpin>(
        stream: &mut R,
        headers: &str,
        initial: &[u8],
    ) -> Option<Message> {
        let mut body = initial.to_vec();

        let mut content_length = 0usize;
        for line in headers.lines() {
            if line.to_lowercase().starts_with("content-length:")
                && let Some(v) = line.split(':').nth(1)
            {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }

        while body.len() < content_length {
            let mut more = vec![0u8; 1024];
            let m = stream.read(&mut more).await.unwrap_or(0);
            if m == 0 {
                break;
            }
            body.extend_from_slice(&more[..m]);
        }

        crate::dns::wire::parse_message(&body[..content_length.min(body.len())]).ok()
    }

    fn build_doh_response(req_msg: &Message, ip: std::net::Ipv4Addr) -> Option<Vec<u8>> {
        let q = req_msg.questions().first()?;
        let mut resp = req_msg.clone();
        resp.set_response(true);
        resp.add_answer(ResourceRecord::new(
            q.qname(),
            RecordType::A,
            RecordClass::IN,
            60,
            RData::A(ip),
        ));
        resp.set_id(req_msg.id());
        crate::dns::wire::serialize_message(&resp).ok()
    }

    async fn spawn_udp_upstream(answer_ip: &str, delay: Option<Duration>) -> String {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap().to_string();
        let ip = answer_ip.to_string();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let (len, peer) = socket.recv_from(&mut buf).await.unwrap();
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            if let Ok(req) = crate::dns::wire::parse_message(&buf[..len])
                && let Some(q) = req.questions().first()
            {
                let mut resp = Message::new();
                resp.set_id(req.id());
                resp.set_response(true);
                resp.add_question(q.clone());
                resp.add_answer(ResourceRecord::new(
                    q.qname(),
                    q.qtype(),
                    q.qclass(),
                    60,
                    RData::A(ip.parse().unwrap()),
                ));
                if let Ok(data) = crate::dns::wire::serialize_message(&resp) {
                    let _ = socket.send_to(&data, peer).await;
                }
            }
        });
        addr
    }

    #[tokio::test]
    async fn test_forward_udp_delivers_response_within_timeout() {
        let upstream = spawn_udp_upstream("9.9.9.9", None).await;
        let core = Forward::new(
            vec![Upstream::new(upstream)],
            Duration::from_secs(2),
            LoadBalanceStrategy::RoundRobin,
        );

        let mut req = Message::new();
        req.set_id(0x1234);
        req.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        let response = core
            .forward_query(&req, &core.upstreams[0])
            .await
            .expect("response within timeout");

        assert_eq!(response.id(), 0x1234);
        let answer = response
            .answers()
            .iter()
            .find_map(|rr| match rr.rdata() {
                RData::A(ip) => Some(*ip),
                _ => None,
            })
            .expect("an A record answer");
        assert_eq!(answer.to_string(), "9.9.9.9");
    }

    #[tokio::test]
    async fn test_forward_udp_times_out_when_no_response() {
        let black_hole = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let upstream = black_hole.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                if black_hole.recv_from(&mut buf).await.is_err() {
                    break;
                }
            }
        });

        let core = Forward::new(
            vec![Upstream::new(upstream)],
            Duration::from_millis(150),
            LoadBalanceStrategy::RoundRobin,
        );

        let mut req = Message::new();
        req.set_id(0x5678);
        req.add_question(Question::new(
            "slow.example.com",
            RecordType::A,
            RecordClass::IN,
        ));

        let result = core.forward_query(&req, &core.upstreams[0]).await;
        assert!(
            matches!(result, Err(crate::Error::UpstreamTimeout { .. })),
            "expected UpstreamTimeout, got {:?}",
            result
        );
    }

    #[test]
    fn test_select_upstream_random_and_fastest() {
        let upstreams = vec![Upstream::new("8.8.8.8:53"), Upstream::new("1.1.1.1:53")];
        let core = Forward::new(
            upstreams.clone(),
            Duration::from_secs(5),
            LoadBalanceStrategy::Random,
        );
        for _ in 0..10 {
            let idx = core.select_upstream(0).unwrap();
            assert!(idx < core.upstreams.len());
        }

        let ups = upstreams;
        let core2 = Forward::new(
            ups.clone(),
            Duration::from_secs(5),
            LoadBalanceStrategy::Fastest,
        );
        let idx_initial = core2.select_upstream(0).unwrap();
        assert_eq!(idx_initial, 0);

        ups[1].health.record_success(Duration::from_millis(5));
        ups[0].health.record_success(Duration::from_millis(100));
        let core3 = Forward::new(ups, Duration::from_secs(5), LoadBalanceStrategy::Fastest);
        let idx_after = core3.select_upstream(0).unwrap();
        assert_eq!(idx_after, 1);
    }

    #[test]
    fn test_record_success_concurrent_updates_keep_avg_in_range() {
        use std::sync::Arc;
        use std::thread;

        let health = Arc::new(UpstreamHealth::new());
        const THREADS: u64 = 8;
        const ITERATIONS: u64 = 500;
        const MIN_MS: u64 = 1;
        const MAX_MS: u64 = 20;

        let mut handles = Vec::new();
        for _ in 0..THREADS {
            let h = Arc::clone(&health);
            handles.push(thread::spawn(move || {
                for _ in 0..ITERATIONS {
                    h.record_success(Duration::from_millis(MAX_MS));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let expected = THREADS * ITERATIONS;
        assert_eq!(health.successes.load(Ordering::Relaxed), expected);
        let avg_us = health.avg_response_time_us_raw();
        let max_us = MAX_MS * 1000;
        let min_us = MIN_MS * 1000;
        assert!(
            (min_us..=max_us).contains(&avg_us),
            "average {avg_us}us drifted outside [{min_us}, {max_us}]"
        );
    }

    #[test]
    fn test_serialize_parse_roundtrip() {
        let mut msg = Message::new();
        msg.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        let data = crate::dns::wire::serialize_message(&msg).expect("serialize");
        let parsed = crate::dns::wire::parse_message(&data).expect("parse");
        assert_eq!(parsed.questions().len(), 1);
        assert_eq!(parsed.questions()[0].qname(), "example.com");
    }

    #[tokio::test]
    async fn test_forward_plugin_no_upstreams() {
        let plugin = ForwardPlugin::new(vec![]);
        let mut ctx = Context::new(Message::new());

        let result = plugin.execute(&mut ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_forward_plugin_skips_if_response_set() {
        let plugin = ForwardPlugin::new(vec!["8.8.8.8:53".to_string()]);
        let mut ctx = Context::new(Message::new());
        ctx.set_response(Some(Message::new()));

        let result = plugin.execute(&mut ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_forward_plugin_doh_http_post_basic() {
        let (upstream_addr, server_task) = spawn_doh_http_server("1.2.3.4").await;
        let core = ForwardBuilder::new()
            .add_upstream(Upstream::new(upstream_addr.clone()))
            .timeout(Duration::from_secs(2))
            .enable_health_checks(true)
            .build();
        let plugin = ForwardPlugin {
            core,
            current: AtomicUsize::new(0),
            concurrent_queries: false,
            tag: None,
        };

        let mut req = Message::new();
        req.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        let mut ctx = Context::new(req);
        let res = plugin.execute(&mut ctx).await;
        assert!(res.is_ok());
        assert!(ctx.response().is_some());
        let resp = ctx.response().unwrap();
        assert!(resp.answer_count() >= 1);

        let mut found = false;
        for rr in resp.answers() {
            if rr.rtype() == RecordType::A
                && let RData::A(ip) = rr.rdata()
            {
                assert_eq!(ip.to_string(), "1.2.3.4");
                found = true;
            }
        }
        assert!(found);

        let _ = server_task.await;
    }

    #[cfg(all(feature = "rustls", any(feature = "doh", feature = "dot")))]
    #[tokio::test]
    async fn test_upstream_health_counters_on_success_and_failure() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let (upstream_addr, server_task) = spawn_doh_https_server("1.2.3.4").await;
        let core = ForwardBuilder::new()
            .add_upstream(Upstream::new(upstream_addr.clone()))
            .timeout(Duration::from_secs(2))
            .enable_health_checks(true)
            .build();
        let plugin = ForwardPlugin {
            core,
            current: AtomicUsize::new(0),
            concurrent_queries: false,
            tag: None,
        };

        let mut req = Message::new();
        req.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        let (q0, s0, f0) = plugin.upstreams[0].health.counters();
        assert_eq!(q0, 0);
        assert_eq!(s0, 0);
        assert_eq!(f0, 0);
        assert!(plugin.health_checks_enabled);

        let mut ctx = Context::new(req.clone());
        let res = plugin.execute(&mut ctx).await;
        assert!(res.is_ok());

        let (q1, s1, f1) = plugin.upstreams[0].health.counters();
        assert_eq!(q1, 1);
        assert_eq!(s1, 1);
        assert_eq!(f1, 0);

        let bad_core = ForwardBuilder::new()
            .add_upstream(Upstream::new("127.0.0.1:43210".to_string()))
            .timeout(Duration::from_secs(1))
            .enable_health_checks(true)
            .build();
        let bad_plugin = ForwardPlugin {
            core: bad_core,
            current: AtomicUsize::new(0),
            concurrent_queries: false,
            tag: None,
        };
        let mut ctx2 = Context::new(req);
        let _res = bad_plugin.execute(&mut ctx2).await;
        let (q2, s2, f2) = bad_plugin.upstreams[0].health.counters();
        assert_eq!(q2, 1);
        assert_eq!(s2, 0);
        assert_eq!(f2, 1);

        let _ = server_task.await;
    }

    #[test]
    fn test_builder_pattern() {
        let core = ForwardBuilder::new()
            .add_upstream(Upstream::new("8.8.8.8:53".to_string()))
            .add_upstream(Upstream::new("1.1.1.1:53".to_string()))
            .timeout(Duration::from_secs(10))
            .strategy(LoadBalanceStrategy::Fastest)
            .enable_health_checks(true)
            .max_attempts(5)
            .build();
        let plugin = ForwardPlugin {
            core,
            current: AtomicUsize::new(0),
            concurrent_queries: false,
            tag: None,
        };

        assert_eq!(plugin.upstreams.len(), 2);
        assert_eq!(plugin.timeout, Duration::from_secs(10));
        assert_eq!(plugin.strategy, LoadBalanceStrategy::Fastest);
        assert!(plugin.health_checks_enabled);
    }

    #[tokio::test]
    async fn test_forward_plugin_doh_http_post() {
        let (url, server_task) = spawn_doh_http_server("9.9.9.9").await;

        let core = ForwardBuilder::new()
            .add_upstream(Upstream::new(url))
            .timeout(Duration::from_secs(2))
            .enable_health_checks(true)
            .build();
        let plugin = ForwardPlugin {
            core,
            current: AtomicUsize::new(0),
            concurrent_queries: false,
            tag: None,
        };

        let mut req = Message::new();
        req.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        let mut ctx = Context::new(req);
        let res = plugin.execute(&mut ctx).await;
        assert!(res.is_ok());
        assert!(ctx.response().is_some());
        let resp = ctx.response().unwrap();

        let mut found = false;
        for rr in resp.answers() {
            if rr.rtype() == RecordType::A
                && let RData::A(ip) = rr.rdata()
            {
                assert_eq!(ip.to_string(), "9.9.9.9");
                found = true;
            }
        }
        assert!(found);

        let _ = server_task.await;
    }

    #[test]
    fn test_add_upstream_with_tag_parses_tag() {
        let core = ForwardBuilder::new()
            .add_upstream(Upstream::with_tag(
                "8.8.8.8:53".to_string(),
                "google".to_string(),
            ))
            .build();
        let plugin = ForwardPlugin {
            core,
            current: AtomicUsize::new(0),
            concurrent_queries: false,
            tag: None,
        };

        assert_eq!(plugin.upstreams.len(), 1);
        assert_eq!(plugin.upstreams[0].addr, "8.8.8.8:53");
        assert_eq!(plugin.upstreams[0].tag.as_deref(), Some("google"));
    }

    #[tokio::test]
    #[cfg(any(feature = "doh", feature = "dot"))]
    async fn test_forward_plugin_doh_https_post_with_self_signed_cert() {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let (url, server_task) = spawn_doh_https_server("4.4.4.4").await;

        let core = ForwardBuilder::new()
            .add_upstream(Upstream::new(url))
            .timeout(Duration::from_secs(2))
            .enable_health_checks(true)
            .build();
        let plugin = ForwardPlugin {
            core,
            current: AtomicUsize::new(0),
            concurrent_queries: false,
            tag: None,
        };

        let mut req = Message::new();
        req.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        let mut ctx = Context::new(req);
        let res = plugin.execute(&mut ctx).await;
        assert!(res.is_ok());
        assert!(ctx.response().is_some());
        let resp = ctx.response().unwrap();

        let mut found = false;
        for rr in resp.answers() {
            if rr.rtype() == RecordType::A
                && let RData::A(ip) = rr.rdata()
            {
                assert_eq!(ip.to_string(), "4.4.4.4");
                found = true;
            }
        }
        assert!(found);

        let _ = server_task.await;
        unsafe {
            std::env::remove_var("LAZYDNS_DOH_ACCEPT_INVALID_CERT");
        }
    }

    async fn spawn_doh_http_server(response_ip: &str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        let ip: std::net::Ipv4Addr = response_ip.parse().unwrap();

        let handle = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);

                let Some((headers, body)) = req.split_once("\r\n\r\n") else {
                    return;
                };
                if let Some(req_msg) = read_doh_request(&mut socket, headers, body.as_bytes()).await
                    && let Some(data) = build_doh_response(&req_msg, ip)
                {
                    let resp_hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\n\r\n",
                        data.len()
                    );
                    let _ = socket.write_all(resp_hdr.as_bytes()).await;
                    let _ = socket.write_all(&data).await;
                }
            }
        });

        let url = format!("http://127.0.0.1:{}/dns-query", local_addr.port());
        (url, handle)
    }

    #[cfg(any(feature = "doh", feature = "dot"))]
    async fn spawn_doh_https_server(response_ip: &str) -> (String, tokio::task::JoinHandle<()>) {
        use rcgen::generate_simple_self_signed;
        use rustls::ServerConfig;
        use rustls::pki_types::PrivateKeyDer;
        use tokio_rustls::TlsAcceptor;

        unsafe {
            std::env::set_var("LAZYDNS_DOH_ACCEPT_INVALID_CERT", "1");
        }

        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_der = cert.cert.der().clone();
        let key_der = cert.signing_key.serialize_der();

        let certs = vec![cert_der.clone()];
        let priv_key = PrivateKeyDer::Pkcs8(key_der.clone().into());
        let server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, priv_key)
            .unwrap();

        let acceptor = TlsAcceptor::from(Arc::new(server_config));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        let ip: std::net::Ipv4Addr = response_ip.parse().unwrap();

        let handle = tokio::spawn(async move {
            if let Ok((socket, _)) = listener.accept().await
                && let Ok(mut tls_stream) = acceptor.accept(socket).await
            {
                let mut buf = vec![0u8; 8192];
                let n = tls_stream.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);

                let Some((headers, body)) = req.split_once("\r\n\r\n") else {
                    return;
                };
                if let Some(req_msg) =
                    read_doh_request(&mut tls_stream, headers, body.as_bytes()).await
                    && let Some(data) = build_doh_response(&req_msg, ip)
                {
                    let resp_hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\n\r\n",
                        data.len()
                    );
                    let _ = tls_stream.write_all(resp_hdr.as_bytes()).await;
                    let _ = tls_stream.write_all(&data).await;
                }
            }
        });

        let url = format!("https://localhost:{}/dns-query", local_addr.port());
        (url, handle)
    }
}
