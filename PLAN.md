# Plan

1. Scaffold a minimal Rust binary crate named `issw`.
2. Implement macOS Text Input Source Services bindings directly through FFI, with narrow safe wrappers around CoreFoundation ownership.
3. Add CLI commands:
   - `issw list`: print selectable keyboard input sources.
   - `issw current`: print the current keyboard input source.
   - `issw <id-or-name>`: switch to an input source by exact id/name, or by a unique case-insensitive substring.
4. Keep the binary dependency-free, fast to start, and macOS-only with a compile-time error on other platforms.
5. Verify with `cargo fmt -q --message-format=short`, `cargo clippy -q --message-format=short`, and `cargo nextest run --cargo-quiet` if the local environment supports the macOS frameworks.
