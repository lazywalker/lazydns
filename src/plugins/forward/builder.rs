use std::collections::HashMap;
use std::time::Duration;

use serde_yaml::Value;

use super::engine::Forward;
use super::types::{LoadBalanceStrategy, Upstream};

/// Parses plugin args into a Forward engine.
pub(crate) struct ForwardBuilder {
    upstreams: Vec<Upstream>,
    timeout: Duration,
    strategy: LoadBalanceStrategy,
    health_checks_enabled: bool,
    max_attempts: usize,
}

impl ForwardBuilder {
    pub(crate) fn new() -> Self {
        Self {
            upstreams: Vec::new(),
            timeout: Duration::from_secs(5),
            strategy: LoadBalanceStrategy::RoundRobin,
            health_checks_enabled: false,
            max_attempts: 3,
        }
    }

    pub(crate) fn add_upstream(mut self, u: Upstream) -> Self {
        self.upstreams.push(u);
        self
    }

    pub(crate) fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub(crate) fn strategy(mut self, strategy: LoadBalanceStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    pub(crate) fn enable_health_checks(mut self, enabled: bool) -> Self {
        self.health_checks_enabled = enabled;
        self
    }

    pub(crate) fn max_attempts(mut self, max: usize) -> Self {
        self.max_attempts = max;
        self
    }

    pub(crate) fn build(self) -> Forward {
        Forward::new(self.upstreams, self.timeout, self.strategy)
            .with_health_checks(self.health_checks_enabled)
            .with_max_attempts(self.max_attempts)
    }

    pub(crate) fn from_args(args: &HashMap<String, Value>) -> crate::Result<Forward> {
        let upstreams_val = args.get("upstreams").ok_or_else(|| {
            crate::Error::Config("upstreams is required for forward plugin".to_string())
        })?;

        let mut upstreams = Vec::new();

        match upstreams_val {
            Value::Sequence(seq) => {
                for item in seq {
                    match item {
                        Value::String(s) => {
                            let entry = normalize_addr(s);
                            if let Some((addr, tag)) = entry.split_once('|') {
                                upstreams
                                    .push(Upstream::with_tag(addr.to_string(), tag.to_string()));
                            } else {
                                upstreams.push(Upstream::new(entry));
                            }
                        }
                        Value::Mapping(map) => {
                            let addr = map
                                .get(Value::String("addr".to_string()))
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| {
                                    crate::Error::Config(
                                        "upstream mapping must contain addr".to_string(),
                                    )
                                })?;
                            let addr = normalize_addr(addr);
                            let tag = map
                                .get(Value::String("tag".to_string()))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            if let Some(t) = tag {
                                upstreams.push(Upstream::with_tag(addr, t));
                            } else {
                                upstreams.push(Upstream::new(addr));
                            }
                        }
                        _ => {
                            return Err(crate::Error::Config(
                                "upstreams must be array of strings or mappings".to_string(),
                            ));
                        }
                    }
                }
            }
            _ => {
                return Err(crate::Error::Config(
                    "upstreams must be an array".to_string(),
                ));
            }
        }

        let mut builder = ForwardBuilder::new();

        if let Some(Value::Number(n)) = args.get("timeout") {
            let secs = n
                .as_i64()
                .ok_or_else(|| crate::Error::Config("Invalid timeout value".to_string()))?;
            builder = builder.timeout(Duration::from_secs(secs as u64));
        }

        if let Some(Value::String(s)) = args.get("strategy") {
            let strategy = match s.as_str() {
                "round_robin" | "roundrobin" => LoadBalanceStrategy::RoundRobin,
                "random" => LoadBalanceStrategy::Random,
                "fastest" => LoadBalanceStrategy::Fastest,
                _ => return Err(crate::Error::Config(format!("Unknown strategy: {}", s))),
            };
            builder = builder.strategy(strategy);
        }

        #[cfg(feature = "web")]
        let default_health_checks = true;
        #[cfg(not(feature = "web"))]
        let default_health_checks = false;

        let health_checks_enabled = if let Some(Value::Bool(enabled)) = args.get("health_checks") {
            *enabled
        } else {
            default_health_checks
        };
        builder = builder.enable_health_checks(health_checks_enabled);

        if let Some(Value::Number(n)) = args.get("max_attempts") {
            let max = n
                .as_i64()
                .ok_or_else(|| crate::Error::Config("Invalid max_attempts value".to_string()))?
                as usize;
            builder = builder.max_attempts(max);
        }

        for u in upstreams {
            builder = builder.add_upstream(u);
        }

        Ok(builder.build())
    }
}

impl Default for ForwardBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Strip udp:// / tcp:// prefixes and append :53 if no port.
/// Preserves DoH URLs (http:// / https://) unchanged.
fn normalize_addr(input: &str) -> String {
    if input.starts_with("http://") || input.starts_with("https://") {
        return input.to_string();
    }
    let mut addr = input
        .trim_start_matches("udp://")
        .trim_start_matches("tcp://")
        .to_string();
    if !addr.contains(':') {
        addr.push_str(":53");
    }
    addr
}
