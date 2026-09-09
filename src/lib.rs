//! Pitwall core library.
//!
//! M1: workspace/process discovery. The [`collector`] module owns the
//! normalized snapshot types; [`platform`] isolates OS-specific detection
//! behind a trait so the core never touches OS APIs directly.

pub mod agents;
pub mod collector;
pub mod ids;
pub mod output;
pub mod platform;
pub mod resume;
pub mod store;

/// Crate version string, kept in sync with `Cargo.toml`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver_shaped() {
        let v = version();
        assert_eq!(v.split('.').count(), 3, "version should be x.y.z, got {v}");
    }
}
