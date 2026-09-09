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
//!
//! ## Identity semantics (M4)
//!
//! `sess_*` names a **terminal context**: one compositor window plus its
//! root process, doing work in one project directory. It deliberately does
//! NOT distinguish three related but different things:
//!
//! - (B) terminal context — what `sess_*` identifies;
//! - (C) agent run — restarting the agent inside the same terminal keeps
//!   the same `sess_*` (the agent kind is recorded as an attribute, and a
//!   checkpoint row is still written per turnover);
//! - (D) human work session — a human task may span terminals, restarts,
//!   and days; Pitwall correlates those via `project_id`, never by
//!   pretending one terminal equals one task.
//!
//! Same project (A) across terminals/restarts always shares `proj_*`.

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
///
/// IMPORTANT: pass the output of [`normalize_project_dir`] here, not a raw
/// observed cwd. Raw cwds vary (`/x/` vs `/x`, symlinked vs real paths) and
/// would split one project into several IDs.
pub fn project_id(canonical_dir: &str) -> String {
    format!("proj_{}", fnv1a_hex(canonical_dir))
}

/// Normalize a project directory for identity purposes (P1):
///
/// - strip trailing `/` (root stays `/`);
/// - resolve symlinks via `std::fs::canonicalize`;
/// - fall back to the stripped raw path when canonicalization fails
///   (vanished dir, permission denied).
///
/// Deterministic: same input always yields the same output on the same
/// machine. Cross-machine stability is not promised (paths differ anyway).
pub fn normalize_project_dir(dir: &str) -> String {
    let stripped = dir.trim_end_matches('/');
    let base = if stripped.is_empty() { "/" } else { stripped };
    std::fs::canonicalize(base)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| base.to_string())
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
    fn normalization_strips_trailing_slash() {
        assert_eq!(normalize_project_dir("/home/guru/Work/"), "/home/guru/Work");
        assert_eq!(
            normalize_project_dir("/home/guru/Work///"),
            "/home/guru/Work"
        );
        assert_eq!(normalize_project_dir("/"), "/");
    }

    #[test]
    fn normalization_falls_back_for_missing_dirs() {
        assert_eq!(normalize_project_dir("/no/such/dir/"), "/no/such/dir");
    }

    #[test]
    fn normalization_resolves_symlinks() {
        let base = std::env::temp_dir().join("pitwall-m2-symlink-test");
        let _ = std::fs::remove_dir_all(&base);
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, base.join("link")).unwrap();
        let via_link = normalize_project_dir(&base.join("link").to_string_lossy());
        let via_real = normalize_project_dir(&real.to_string_lossy());
        assert_eq!(via_link, via_real);
        // …and therefore the same project ID.
        assert_eq!(project_id(&via_link), project_id(&via_real));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn trailing_slash_shares_project_id() {
        let a = normalize_project_dir("/home/guru/Work/");
        let b = normalize_project_dir("/home/guru/Work");
        assert_eq!(project_id(&a), project_id(&b));
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
