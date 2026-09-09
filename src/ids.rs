//! Deterministic local identifiers.
//!
//! Later milestones (state DB, checkpoints, panel refresh correlation) need
//! to recognize "the same project/session" across snapshots. These IDs are:
//!
//! - deterministic: same inputs always yield the same ID;
//! - local-only: stable on this machine, meaningless elsewhere;
//! - opaque: `proj_<hex>` / `sess_<hex>`, FNV-1a 64 over documented inputs.
//!
//! Deliberately not UUIDs/random: no RNG, no clock, no stored mapping needed.

use std::fmt::Write as _;

/// FNV-1a 64-bit hash, hand-rolled to keep the crate dependency-free.
pub fn fnv1a_hex(input: &str) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    let mut out = String::with_capacity(16);
    write!(out, "{hash:016x}").expect("writing to String cannot fail");
    out
}

/// Stable project identity: derived from the canonical project directory.
/// The same directory always maps to the same ID, across restarts and
/// refreshes.
pub fn project_id(canonical_dir: &str) -> String {
    format!("proj_{}", fnv1a_hex(canonical_dir))
}

/// Session identity: derived from project ID + Hyprland window address +
/// session root PID. Stable across refreshes while the window/process lives;
/// a restarted terminal is a new session (by design — its context is new).
pub fn session_id(project_id: &str, window_address: &str, root_pid: u32) -> String {
    format!(
        "sess_{}",
        fnv1a_hex(&format!("{project_id}|{window_address}|{root_pid}"))
    )
}

/// Human fallback when no project directory is known: identity of the
/// headless/windowless grouping key, still deterministic.
pub fn unknown_project_id(window_address: &str, root_pid: u32) -> String {
    project_id(&format!("unknown:{window_address}:{root_pid}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_is_deterministic_and_hex_shaped() {
        let a = fnv1a_hex("proj:/home/guru/Projects/pitwall");
        let b = fnv1a_hex("proj:/home/guru/Projects/pitwall");
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn fnv1a_distinguishes_inputs() {
        assert_ne!(fnv1a_hex("a"), fnv1a_hex("b"));
        assert_ne!(fnv1a_hex(""), fnv1a_hex("/"));
    }

    #[test]
    fn project_id_is_stable_per_directory() {
        assert_eq!(project_id("/home/guru/Work"), project_id("/home/guru/Work"));
        assert_ne!(project_id("/home/guru/Work"), project_id("/home/guru"));
        assert!(project_id("/x").starts_with("proj_"));
    }

    #[test]
    fn session_id_changes_with_window_or_pid() {
        let p = project_id("/home/guru/Work");
        let s1 = session_id(&p, "0xabc", 100);
        assert_eq!(s1, session_id(&p, "0xabc", 100));
        assert_ne!(s1, session_id(&p, "0xdef", 100));
        assert_ne!(s1, session_id(&p, "0xabc", 101));
        assert!(s1.starts_with("sess_"));
    }
}
