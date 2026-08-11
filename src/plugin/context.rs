//! Plugin execution context
//!
//! The context holds the DNS query and response, as well as metadata
//! that can be shared between plugins.

use crate::dns::Message;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

/// Plugin execution context
///
/// The context is passed through the plugin chain and contains:
/// - The original DNS query
/// - An optional DNS response
/// - Metadata for inter-plugin communication
///
/// # Example
///
/// ```rust
/// use lazydns::plugin::Context;
/// use lazydns::dns::Message;
///
/// let request = Message::new();
/// let mut ctx = Context::new(request);
///
/// // Set a response
/// let response = Message::new();
/// ctx.set_response(Some(response));
///
/// // Store metadata
/// ctx.set_metadata("key", "value");
/// ```
#[derive(Debug)]
pub struct Context {
    /// The original DNS query
    request: Message,

    /// The DNS response (if set by a plugin). Stored as `Arc<Message>` to enable
    /// sharing without deep cloning; mutation is done via `Arc::make_mut`.
    response: Option<Arc<Message>>,

    /// Metadata for inter-plugin communication
    metadata: HashMap<String, Box<dyn Any + Send + Sync>>,
}

impl Context {
    /// Create a new context with a DNS query
    ///
    /// # Arguments
    ///
    /// * `request` - The DNS query message
    pub fn new(request: Message) -> Self {
        Self {
            request,
            response: None,
            metadata: HashMap::new(),
        }
    }

    /// Get a reference to the DNS query
    pub fn request(&self) -> &Message {
        &self.request
    }

    /// Get a mutable reference to the DNS query
    pub fn request_mut(&mut self) -> &mut Message {
        &mut self.request
    }

    /// Get a reference to the DNS response
    pub fn response(&self) -> Option<&Message> {
        self.response.as_deref()
    }

    /// Get a mutable reference to the DNS response. This will perform copy-on-write
    /// via `Arc::make_mut` if the response is shared by other owners.
    pub fn response_mut(&mut self) -> Option<&mut Message> {
        self.response.as_mut().map(Arc::make_mut)
    }

    /// Set the DNS response from an owned `Message` (wraps in Arc)
    pub fn set_response(&mut self, response: Option<Message>) {
        self.response = response.map(Arc::new);
    }

    /// Set the DNS response from an `Arc<Message>` directly
    pub fn set_response_arc(&mut self, response: Option<Arc<Message>>) {
        self.response = response;
    }

    /// Set a REFUSED response that echoes the request's id and question section.
    pub fn set_refused(&mut self) {
        let mut response = Message::new();
        response.set_id(self.request().id());
        response.set_response(true);
        response.set_response_code(crate::dns::ResponseCode::Refused);
        *response.questions_mut() = self.request().questions().to_vec();
        self.set_response(Some(response));
    }

    /// Take the DNS response, leaving None in its place. Attempts to avoid cloning
    /// when the Arc is uniquely owned; otherwise returns a cloned Message.
    pub fn take_response(&mut self) -> Option<Message> {
        self.response.take().map(|arc| match Arc::try_unwrap(arc) {
            Ok(msg) => msg,
            Err(shared) => (*shared).clone(),
        })
    }

    /// Check if a response has been set
    pub fn has_response(&self) -> bool {
        self.response.is_some()
    }

    /// Store metadata in the context
    ///
    /// This allows plugins to communicate with each other by storing
    /// and retrieving typed data.
    ///
    /// # Arguments
    ///
    /// * `key` - The metadata key
    /// * `value` - The metadata value
    pub fn set_metadata<T: Any + Send + Sync>(&mut self, key: impl Into<String>, value: T) {
        self.metadata.insert(key.into(), Box::new(value));
    }

    /// Retrieve metadata from the context
    ///
    /// # Arguments
    ///
    /// * `key` - The metadata key
    ///
    /// # Returns
    ///
    /// Returns a reference to the metadata value if it exists and has the correct type.
    pub fn get_metadata<T: Any>(&self, key: &str) -> Option<&T> {
        self.metadata.get(key).and_then(|v| v.downcast_ref::<T>())
    }

    /// Remove metadata from the context
    ///
    /// # Arguments
    ///
    /// * `key` - The metadata key
    ///
    /// # Returns
    ///
    /// Returns the metadata value if it existed.
    pub fn remove_metadata(&mut self, key: &str) -> bool {
        self.metadata.remove(key).is_some()
    }

    /// Check if metadata exists
    ///
    /// # Arguments
    ///
    /// * `key` - The metadata key
    pub fn has_metadata(&self, key: &str) -> bool {
        self.metadata.contains_key(key)
    }

    /// Clear all metadata
    pub fn clear_metadata(&mut self) {
        self.metadata.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::{Question, RecordClass, RecordType};

    #[test]
    fn test_context_creation() {
        let request = Message::new();
        let ctx = Context::new(request);

        assert!(!ctx.has_response());
        assert_eq!(ctx.request().question_count(), 0);
    }

    #[test]
    fn test_request_access() {
        let mut request = Message::new();
        request.set_id(1234);
        request.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        let ctx = Context::new(request);

        assert_eq!(ctx.request().id(), 1234);
        assert_eq!(ctx.request().question_count(), 1);
    }

    #[test]
    fn test_response_handling() {
        let request = Message::new();
        let mut ctx = Context::new(request);

        assert!(!ctx.has_response());
        assert!(ctx.response().is_none());

        let mut response = Message::new();
        response.set_id(5678);
        ctx.set_response(Some(response));

        assert!(ctx.has_response());
        assert_eq!(ctx.response().unwrap().id(), 5678);

        let taken = ctx.take_response();
        assert!(taken.is_some());
        assert!(!ctx.has_response());
    }

    #[test]
    fn test_metadata() {
        let request = Message::new();
        let mut ctx = Context::new(request);

        // Store different types
        ctx.set_metadata("string", "test");
        ctx.set_metadata("number", 42i32);
        ctx.set_metadata("bool", true);

        // Retrieve with correct type
        assert_eq!(ctx.get_metadata::<&str>("string"), Some(&"test"));
        assert_eq!(ctx.get_metadata::<i32>("number"), Some(&42));
        assert_eq!(ctx.get_metadata::<bool>("bool"), Some(&true));

        // Wrong type returns None
        assert!(ctx.get_metadata::<i64>("number").is_none());

        // Check existence
        assert!(ctx.has_metadata("string"));
        assert!(!ctx.has_metadata("nonexistent"));

        // Remove metadata
        assert!(ctx.remove_metadata("string"));
        assert!(!ctx.has_metadata("string"));
        assert!(!ctx.remove_metadata("string"));
    }

    #[test]
    fn test_clear_metadata() {
        let request = Message::new();
        let mut ctx = Context::new(request);

        ctx.set_metadata("key1", "value1");
        ctx.set_metadata("key2", 123);

        assert!(ctx.has_metadata("key1"));
        assert!(ctx.has_metadata("key2"));

        ctx.clear_metadata();

        assert!(!ctx.has_metadata("key1"));
        assert!(!ctx.has_metadata("key2"));
    }

    #[test]
    fn test_mutable_request() {
        let request = Message::new();
        let mut ctx = Context::new(request);

        ctx.request_mut().set_id(9999);
        assert_eq!(ctx.request().id(), 9999);

        ctx.request_mut()
            .add_question(Question::new("test.com", RecordType::A, RecordClass::IN));
        assert_eq!(ctx.request().question_count(), 1);
    }

    #[test]
    fn test_mutable_response() {
        let request = Message::new();
        let mut ctx = Context::new(request);

        let response = Message::new();
        ctx.set_response(Some(response));

        if let Some(resp) = ctx.response_mut() {
            resp.set_id(1111);
        }

        assert_eq!(ctx.response().unwrap().id(), 1111);
    }
}
