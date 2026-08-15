//! Alert engine implementation
//!
//! Monitors security events and metrics, triggers alerts based on configured rules.

use crate::Result;
use crate::plugins::audit::{AuditEvent, event_bus};
use crate::web::config::{AlertCondition, AlertConfig, AlertRule, AlertSeverity, WebhookConfig};
use parking_lot::RwLock;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info, trace, warn};

/// Alert instance
#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    /// Unique alert ID
    pub id: String,
    /// Alert rule name
    pub rule_name: String,
    /// Severity level
    pub severity: AlertSeverity,
    /// Alert message
    pub message: String,
    /// Timestamp (Unix seconds) of first occurrence
    pub timestamp: u64,
    /// Timestamp (Unix seconds) of last occurrence
    pub last_updated: u64,
    /// Number of times this alert has been triggered
    pub occurrence_count: u64,
    /// Whether the alert has been acknowledged
    pub acknowledged: bool,
    /// Additional context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<HashMap<String, String>>,
}

impl Alert {
    /// Create a new alert
    pub fn new(rule_name: &str, severity: AlertSeverity, message: String) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            rule_name: rule_name.to_string(),
            severity,
            message,
            timestamp: now,
            last_updated: now,
            occurrence_count: 1,
            acknowledged: false,
            context: None,
        }
    }

    /// Add context to the alert
    pub fn with_context(mut self, key: &str, value: &str) -> Self {
        self.context
            .get_or_insert_with(HashMap::new)
            .insert(key.to_string(), value.to_string());
        self
    }

    /// Update occurrence count and last_updated timestamp
    pub fn increment_occurrence(&mut self) {
        self.occurrence_count += 1;
        self.last_updated = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    /// Check if this alert matches another (for deduplication)
    pub fn matches(&self, other: &Alert) -> bool {
        self.rule_name == other.rule_name && self.message == other.message
    }
}

/// Alert engine for monitoring and alerting
pub struct AlertEngine {
    /// Configuration
    config: AlertConfig,
    /// Recent alerts (newest first)
    alerts: RwLock<VecDeque<Alert>>,
    /// Deduplication cache: rule_name -> last trigger time
    dedup_cache: RwLock<HashMap<String, Instant>>,
    /// Alert counter
    alert_counter: AtomicU64,
    /// HTTP client for webhooks
    http_client: reqwest::Client,
}

impl AlertEngine {
    /// Create a new alert engine
    pub fn new(config: &AlertConfig) -> Result<Self> {
        for rule in &config.rules {
            if !matches!(rule.condition, AlertCondition::SecurityEvent { .. }) {
                warn!(
                    rule = %rule.name,
                    "alert rule condition is not evaluated yet and will never fire"
                );
            }
        }
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| crate::Error::Config(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            config: config.clone(),
            alerts: RwLock::new(VecDeque::with_capacity(config.max_alerts)),
            dedup_cache: RwLock::new(HashMap::new()),
            alert_counter: AtomicU64::new(0),
            http_client,
        })
    }

    /// Run the alert engine (subscribes to security events)
    pub async fn run(&self) -> Result<()> {
        if !self.config.enabled {
            info!("Alert engine disabled");
            return Ok(());
        }

        let bus = match event_bus() {
            Some(bus) => bus,
            None => {
                info!("Event bus not initialized, alert engine not starting");
                return Ok(());
            }
        };

        let mut subscriber = bus.subscribe_security();
        info!(rules = self.config.rules.len(), "Alert engine started");

        loop {
            match subscriber.recv().await {
                Some(event) => {
                    trace!("Processing security event for alerts");
                    self.process_event(&event).await;
                }
                None => {
                    info!("Event bus closed, alert engine stopping");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Process a security event and check alert rules
    async fn process_event(&self, event: &AuditEvent) {
        for rule in &self.config.rules {
            if self.matches_rule(rule, event) {
                // Check deduplication
                if self.should_deduplicate(&rule.name) {
                    trace!(rule = %rule.name, "Alert deduplicated");
                    continue;
                }

                // Create alert
                let message = self.format_message(rule, event);
                let alert = Alert::new(&rule.name, rule.severity, message);

                debug!(
                    rule = %rule.name,
                    severity = ?rule.severity,
                    "Triggering alert"
                );

                // Store alert
                self.add_alert(alert.clone());

                // Send webhook notification
                if let Some(ref webhook) = self.config.webhook {
                    self.send_webhook(webhook, &alert).await;
                }
            }
        }
    }

    /// Check if an event matches a rule condition
    fn matches_rule(&self, rule: &AlertRule, event: &AuditEvent) -> bool {
        match &rule.condition {
            AlertCondition::SecurityEvent { event_type } => {
                if let AuditEvent::Security { event_type: et, .. } = event {
                    et.as_str() == event_type
                } else {
                    false
                }
            }
            AlertCondition::RateThreshold { .. } => {
                // Rate threshold requires metrics integration, not implemented here
                false
            }
            AlertCondition::UpstreamHealth { .. } => {
                // Upstream health requires forward plugin integration
                false
            }
            AlertCondition::ErrorRate { .. } => {
                // Error rate requires metrics integration
                false
            }
        }
    }

    /// Check if an alert should be deduplicated
    fn should_deduplicate(&self, rule_name: &str) -> bool {
        let dedup_window = Duration::from_secs(self.config.dedup_window_secs);
        let mut cache = self.dedup_cache.write();

        if let Some(last_trigger) = cache.get(rule_name)
            && last_trigger.elapsed() < dedup_window
        {
            return true;
        }

        cache.insert(rule_name.to_string(), Instant::now());
        false
    }

    /// Format alert message
    fn format_message(&self, rule: &AlertRule, event: &AuditEvent) -> String {
        if let Some(ref template) = rule.message {
            // Simple template substitution
            let mut msg = template.clone();
            if let AuditEvent::Security {
                event_type,
                message,
                client_ip,
                qname,
                ..
            } = event
            {
                msg = msg.replace("{event_type}", event_type.as_str());
                msg = msg.replace("{message}", message);
                if let Some(ip) = client_ip {
                    msg = msg.replace("{client_ip}", &ip.to_string());
                }
                if let Some(domain) = qname {
                    msg = msg.replace("{domain}", domain);
                }
            }
            msg
        } else {
            // Default message
            match event {
                AuditEvent::Security { message, .. } => message.clone(),
                AuditEvent::Query(_) => "Query alert".to_string(),
            }
        }
    }

    /// Add an alert to storage, aggregating duplicates
    fn add_alert(&self, alert: Alert) {
        let mut alerts = self.alerts.write();

        // Check if a matching alert already exists in recent history (last 10 alerts)
        for existing in alerts.iter_mut().take(10) {
            if existing.matches(&alert) && !existing.acknowledged {
                // Found matching alert - increment occurrence count instead of creating new
                existing.increment_occurrence();
                debug!(
                    rule = %alert.rule_name,
                    occurrences = existing.occurrence_count,
                    "Alert aggregated"
                );
                return;
            }
        }

        // No matching alert found, add new one
        // Remove oldest if at capacity
        while alerts.len() >= self.config.max_alerts {
            alerts.pop_back();
        }

        alerts.push_front(alert);
        self.alert_counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Send webhook notification
    async fn send_webhook(&self, webhook: &WebhookConfig, alert: &Alert) {
        let payload = serde_json::json!({
            "alert": alert,
            "source": "lazydns",
        });

        let mut request = self
            .http_client
            .post(&webhook.url)
            .json(&payload)
            .timeout(Duration::from_secs(webhook.timeout_secs));

        if let Some(ref auth) = webhook.auth_header {
            request = request.header("Authorization", auth);
        }

        let mut retries = 0;
        loop {
            match request.try_clone() {
                Some(req) => match req.send().await {
                    Ok(response) => {
                        if response.status().is_success() {
                            debug!(url = %webhook.url, "Webhook sent successfully");
                            return;
                        } else {
                            warn!(
                                url = %webhook.url,
                                status = %response.status(),
                                "Webhook returned error status"
                            );
                        }
                    }
                    Err(e) => {
                        warn!(
                            url = %webhook.url,
                            error = %e,
                            retry = retries,
                            "Webhook request failed"
                        );
                    }
                },
                None => {
                    error!("Failed to clone webhook request");
                    return;
                }
            }

            retries += 1;
            if retries >= webhook.retries {
                error!(
                    url = %webhook.url,
                    "Webhook failed after {} retries",
                    webhook.retries
                );
                return;
            }

            // Exponential backoff, capped so the shift cannot overflow
            let backoff_ms = 100u64
                .checked_shl(retries.min(20))
                .unwrap_or(u64::MAX >> 20);
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
        }
    }

    /// Get recent alerts
    pub fn recent_alerts(&self, limit: usize) -> Vec<Alert> {
        self.alerts.read().iter().take(limit).cloned().collect()
    }

    /// Get count of recent alerts
    pub fn recent_alert_count(&self) -> usize {
        self.alerts.read().len()
    }

    /// Get total alert count
    pub fn total_alerts(&self) -> u64 {
        self.alert_counter.load(Ordering::Relaxed)
    }

    /// Acknowledge an alert by ID
    pub fn acknowledge(&self, alert_id: &str) -> bool {
        let mut alerts = self.alerts.write();
        for alert in alerts.iter_mut() {
            if alert.id == alert_id {
                alert.acknowledged = true;
                return true;
            }
        }
        false
    }

    /// Acknowledge all alerts
    pub fn acknowledge_all(&self) {
        let mut alerts = self.alerts.write();
        for alert in alerts.iter_mut() {
            alert.acknowledged = true;
        }
    }

    /// Clear all alerts
    pub fn clear(&self) {
        self.alerts.write().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> AlertConfig {
        AlertConfig {
            enabled: true,
            rules: vec![AlertRule {
                name: "test_rule".to_string(),
                condition: AlertCondition::SecurityEvent {
                    event_type: "rate_limit_exceeded".to_string(),
                },
                severity: AlertSeverity::Warning,
                message: Some("Rate limit exceeded for {client_ip}".to_string()),
            }],
            dedup_window_secs: 60,
            max_alerts: 100,
            webhook: None,
        }
    }

    #[test]
    fn test_alert_creation() {
        let alert = Alert::new("test", AlertSeverity::Warning, "Test message".to_string());
        assert!(!alert.id.is_empty());
        assert!(!alert.acknowledged);
    }

    #[test]
    fn test_engine_creation() {
        let config = sample_config();
        let engine = AlertEngine::new(&config).unwrap();
        assert_eq!(engine.recent_alert_count(), 0);
    }

    #[test]
    fn test_add_alert() {
        let config = sample_config();
        let engine = AlertEngine::new(&config).unwrap();

        let alert = Alert::new("test", AlertSeverity::Warning, "Test".to_string());
        engine.add_alert(alert);

        assert_eq!(engine.recent_alert_count(), 1);
    }

    #[test]
    fn test_alert_aggregation() {
        let config = sample_config();
        let engine = AlertEngine::new(&config).unwrap();

        // Add same alert multiple times
        let alert1 = Alert::new("test", AlertSeverity::Warning, "Same alert".to_string());
        let alert2 = Alert::new("test", AlertSeverity::Warning, "Same alert".to_string());
        let alert3 = Alert::new("test", AlertSeverity::Warning, "Same alert".to_string());

        engine.add_alert(alert1);
        engine.add_alert(alert2);
        engine.add_alert(alert3);

        // Should only have 1 alert with occurrence_count = 3
        assert_eq!(engine.recent_alert_count(), 1);
        let alerts = engine.recent_alerts(1);
        assert_eq!(alerts[0].occurrence_count, 3);
    }

    #[test]
    fn test_alert_different_messages() {
        let config = sample_config();
        let engine = AlertEngine::new(&config).unwrap();

        // Add alerts with different messages
        let alert1 = Alert::new("test", AlertSeverity::Warning, "Alert A".to_string());
        let alert2 = Alert::new("test", AlertSeverity::Warning, "Alert B".to_string());

        engine.add_alert(alert1);
        engine.add_alert(alert2);

        // Should have 2 alerts
        assert_eq!(engine.recent_alert_count(), 2);
    }

    #[test]
    fn test_acknowledge() {
        let config = sample_config();
        let engine = AlertEngine::new(&config).unwrap();

        let alert = Alert::new("test", AlertSeverity::Warning, "Test".to_string());
        let id = alert.id.clone();
        engine.add_alert(alert);

        assert!(engine.acknowledge(&id));
        let alerts = engine.recent_alerts(1);
        assert!(alerts[0].acknowledged);
    }

    #[test]
    fn test_deduplication() {
        let mut config = sample_config();
        config.dedup_window_secs = 60;
        let engine = AlertEngine::new(&config).unwrap();

        // First trigger - should not deduplicate
        assert!(!engine.should_deduplicate("test_rule"));

        // Second trigger - should deduplicate
        assert!(engine.should_deduplicate("test_rule"));

        // Different rule - should not deduplicate
        assert!(!engine.should_deduplicate("other_rule"));
    }
}
