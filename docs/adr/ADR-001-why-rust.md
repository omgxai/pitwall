# ADR-001: Why Rust (not Go)

- **Status:** Accepted (M0).
- **Context:** The spec suggested Go, but Go is not installed on the target
  machine; Rust 1.98 and Cargo are. The MVP needs a single static binary with
  low RAM/CPU for continuous background use.
- **Decision:** Build the core/runtime in Rust.
- **Consequences:** Static ARM64 binary, no runtime to install, `cargo`
  test/clippy/fmt as the standard toolchain. Contributors need Rust stable
  1.80+. Go remains a possible future port, but there is no plan for one.
