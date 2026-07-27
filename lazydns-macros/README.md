# lazydns-macros

Procedural macros used by the lazydns project to simplify plugin registration and lifecycle boilerplate.

This crate provides a small set of proc-macros used by the main `lazydns` crate and third-party plugins.

## Crate features and notes

- This is a `proc-macro` crate (see `Cargo.toml`: `proc-macro = true`).
- `syn` is used with the following features enabled in `Cargo.toml`:
  ```toml
  syn = { version = "2.0", default-features = false, features = [
    "parsing",
    "derive",
    "proc-macro",
  ] }
  ```

## Usage

Add the dependency to your crate:

```toml
[dependencies]
lazydns-macros = "0.2"
```

Use the macros in your plugin implementation. Example (pseudo):

```rust
use lazydns_macros::RegisterExecPlugin;

#[derive(RegisterExecPlugin)]
pub struct MyPlugin {
    // plugin fields
}

impl MyPlugin {
    // plugin methods
}
```

Macros provided (examples; see crate source for exact names/signatures):
- `RegisterPlugin` / `RegisterExecPlugin`: implements plugin registration helpers
- `ShutdownPlugin`: helper derive to wire up shutdown lifecycle methods


## License

This crate is licensed under GPL-3.0-or-later (see `Cargo.toml`).
