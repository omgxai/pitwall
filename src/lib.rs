//! Pitwall core library.
//!
//! M0: crate skeleton only. Functional modules (collector, state,
//! summary, platform) land in M1+ behind the `platform` abstraction
//! so OS-specific code never leaks into the core.

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
