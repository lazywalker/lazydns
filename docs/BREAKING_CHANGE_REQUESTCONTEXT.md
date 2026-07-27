# Breaking Change (from v0.2.36 to v0.2.40)
# RequestContext Refactor Documentation

## Overview

This document describes the RequestContext refactor that improves the `RequestHandler` trait interface by consolidating request metadata into a unified context object.

## Motivation

Before v0.2.37 the `RequestHandler` trait had a method signature that accepted one parameters for the DNS message and non for the client address. This design had limitations in `ratelimit` extensibility, so we refactored it to add a `client_addr` parameter. However, as we continued to add more metadata (such as protocol type for DoH/DoT/DoQ), it became clear that a more scalable solution was needed.

```rust
async fn handle(&self, request: Message, client_addr: Option<SocketAddr>) -> Result<Message>;
```

This design had several limitations:
1. **Limited extensibility**: Adding new metadata (such as protocol type, TLS info) required changing the trait signature
2. **Unclear semantics**: Multiple `Option` parameters made the API harder to understand
3. **Manual propagation**: Callers had to manually pass both parameters separately

## New Design

The refactored interface uses a unified `RequestContext` struct:

```rust
async fn handle(&self, ctx: RequestContext) -> Result<Message>;
```

### Core Types

#### RequestContext

```rust
pub struct RequestContext {
    pub message: Message,
    pub client_info: Option<ClientInfo>,
    pub protocol: Protocol,
}
```

**Methods:**
- `new(message, protocol)` - Create context without client information
- `with_client(message, client_addr, protocol)` - Create context with client information
- `client_ip()` - Get client IP address (if available)
- `client_addr()` - Get client socket address (if available)
- `into_message()` - Consume context and extract the message
- `into_raw()` - Consume context and extract all components

#### ClientInfo

```rust
pub struct ClientInfo {
    pub addr: SocketAddr,
    pub ip: IpAddr,
    pub port: u16,
}
```

Automatically extracted from `SocketAddr` for convenience.

#### Protocol

```rust
pub enum Protocol {
    Udp,
    Tcp,
    DoH,    // DNS over HTTPS
    DoT,    // DNS over TLS
    DoQ,    // DNS over QUIC
}
```

Identifies which protocol was used for the request.

## Benefits

### 1. Better Extensibility

Adding new metadata is now straightforward without breaking existing code:

```rust
// Future: Add TLS session info, HTTP headers, etc.
pub struct RequestContext {
    pub message: Message,
    pub client_info: Option<ClientInfo>,
    pub protocol: Protocol,
    pub tls_info: Option<TlsInfo>,  // Easy to add
}
```

### 2. Clearer Intent

The context object makes it explicit that all request metadata is grouped together:

```rust
// Before: unclear what the parameters mean
handler.handle(message, Some(addr)).await?

// After: clear that we're passing a request context
let ctx = RequestContext::with_client(message, Some(addr), Protocol::Udp);
handler.handle(ctx).await?
```

### 3. Type Safety

The `Protocol` enum ensures protocol types are well-defined and prevents errors:

```rust
// Type-safe protocol identification
match ctx.protocol {
    Protocol::DoH => { /* DoH-specific handling */ }
    Protocol::Udp | Protocol::Tcp => { /* Standard DNS handling */ }
    _ => { /* ... */ }
}
```

## Implementation Details

### Protocol Detection

Each server implementation creates the appropriate `Protocol` value:

- **UDP Server**: `Protocol::Udp`
- **TCP Server**: `Protocol::Tcp`
- **DoH Server**: `Protocol::DoH` (note: client IP may not be reliable due to proxies)
- **DoT Server**: `Protocol::DoT`
- **DoQ Server**: `Protocol::DoQ`

### Plugin Integration

The `PluginHandler` automatically extracts client information and adds it to plugin metadata:

```rust
impl RequestHandler for PluginHandler {
    async fn handle(&self, ctx: RequestContext) -> Result<Message> {
        // Extract client info from context
        if let Some(client_info) = ctx.client_info {
            context.set_metadata("client_ip", client_info.ip.to_string());
            context.set_metadata("client_port", client_info.port.to_string());
        }
        // ...
    }
}
```

This ensures plugins can access client information through the metadata system.

### Background Tasks

For background tasks (such as cache refresh), create a context without client information:

```rust
// Background refresh - no client information needed
let ctx = RequestContext::new(request, Protocol::Udp);
handler.handle(ctx).await?;
```

## Files Modified

### Core Infrastructure
- `src/server/handler.rs` - Added RequestContext, ClientInfo, Protocol types; updated trait
- `src/server/mod.rs` - Exported new types
- `src/plugin/mod.rs` - Updated PluginHandler to use RequestContext

### Server Implementations
- `src/server/udp.rs` - Updated UDP server
- `src/server/tcp.rs` - Updated TCP server
- `src/server/doh.rs` - Updated DoH server (GET and POST handlers)
- `src/server/dot.rs` - Updated DoT server
- `src/server/doq.rs` - Updated DoQ server

### Plugin Implementations
- `src/plugins/server.rs` - Updated PluginRequestHandler
- `src/plugins/cache.rs` - Updated background refresh handlers

### Tests
- `tests/server_test.rs` - Updated test handlers
- `tests/integration_doq.rs` - Updated test handlers
- `tests/integration_tls_doh_dot.rs` - Updated test handlers

## Testing

All existing tests pass without modification to test logic, demonstrating backward compatibility:

```bash
$ cargo test --lib
test result: ok. 428 passed; 0 failed; 10 ignored

$ cargo test --tests
test result: ok. All integration tests passed
```

New unit tests verify RequestContext functionality:
- `test_request_context_with_client` - Context with client information
- `test_request_context_without_client` - Context without client information

## Future Enhancements

The new design enables several future improvements:

1. **TLS Session Information**: Add TLS certificate details, cipher suites
2. **HTTP Headers**: For DoH, include request headers for logging/routing
3. **Query Metrics**: Track timing information per request
4. **Request Tracing**: Add trace IDs for distributed tracing
5. **Rate Limiting Hints**: Pass rate limit state through context
