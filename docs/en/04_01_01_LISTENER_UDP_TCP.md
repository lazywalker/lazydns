# UDP and TCP Listeners

Introduction: Explains how to configure UDP and TCP DNS listeners in lazydns. These are the basic server plugins that handle standard DNS queries over UDP and TCP protocols.

## Configuration Options

Both `udp_server` and `tcp_server` plugins support the following options under `args`:

- `listen`: Listen address (string). Default: "0.0.0.0:53". Supports shorthand like ":5353" (expands to "0.0.0.0:5353").
- `entry`: Entry plugin name (string). Default: "main_sequence". Specifies which plugin sequence to use for processing DNS queries.

## Examples

### Basic UDP Server

```yaml
plugins:
  - tag: udp_server
    type: udp_server
    args:
      listen: :53  # Listen on all interfaces, port 53
      entry: sequence_main
```

### Basic TCP Server

```yaml
plugins:
  - tag: tcp_server
    type: tcp_server
    args:
      listen: :53  # Listen on all interfaces, port 53
      entry: sequence_main
```

### Custom Port (for testing)

```yaml
plugins:
  - tag: udp_test
    type: udp_server
    args:
      listen: 127.0.0.1:5353  # Localhost only, port 5353
      entry: sequence_main

  - tag: tcp_test
    type: tcp_server
    args:
      listen: 127.0.0.1:5353  # Localhost only, port 5353
      entry: sequence_main
```

## Notes

- **Port 53**: Standard DNS port. Requires root privileges on Unix systems. For testing, use higher ports like 5353.
- **IPv6**: Use addresses like "[::]:53" for IPv6.
- **Multiple Servers**: You can run multiple UDP/TCP servers on different addresses/ports.
- **Entry Plugin**: The `entry` should reference a valid plugin sequence (such as `sequence_main` as defined in your config).
- **Performance**: UDP is typically faster for small queries; TCP handles larger responses and is more reliable over unreliable networks.

## Implementation Details

- UDP server uses async UDP sockets with Tokio.
- TCP server handles connections asynchronously, spawning a task per connection.
- Both servers integrate with the plugin system via the `PluginHandler`.
- Errors during startup are logged, and the server won't start if the address is invalid.

See `src/server/launcher.rs` for the launch logic and `src/server/udp.rs`/`src/server/tcp.rs` for server implementations.