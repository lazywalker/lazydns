//! Audit subsystem (web feature internal).
//!
//! Provides DNS query logging and security event tracking via the event bus.
//! Compiled with the `web` feature and operates automatically; no user
//! configuration or plugin registration is required.
//!
//! - Query logging: `PluginHandler::handle()` calls [`plugin::log_query_for_context`]
//!   after each request, publishing to the event bus.
//! - Security events: individual plugins (ratelimit, blackhole, forward, ...)
//!   call `AUDIT_LOGGER.log_security_event()` directly.
//! - Consumers: WebUI SSE streams and the alert engine subscribe to the event bus.

pub mod event;
#[cfg(feature = "web")]
pub mod event_bus;
pub mod logger;
pub mod plugin;

// Public re-exports
pub use event::{AuditEvent, QueryLogEntry, SecurityEventType};
#[cfg(feature = "web")]
pub use event_bus::{
    AuditEventBus, QueryLogSubscriber, SecurityEventSubscriber, event_bus, init_event_bus,
};
pub use logger::{AUDIT_LOGGER, AuditLogger};
pub use plugin::log_query_for_context;
