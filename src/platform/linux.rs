//! Linux platform implementation (Omarchy-first).
//!
//! Sources, all read-only:
//!
//! - `/proc/<pid>/{stat,comm,cmdline,cwd}` for process truth;
//! - `hyprctl clients -j` for compositor windows (minimal hand-rolled
//!   parser for the six fields we need — keeps the crate dependency-free
//!   and offline-buildable);
//! - `.git/HEAD` file read for the branch (no subprocess);
//! - `git status --porcelain=v1` subprocess (only when `git` exists) for
//!   working-tree cleanliness;
//! - `/proc/sys/kernel/hostname` for the host name.

use super::{GitInfo, Platform, RawProcess, WindowInfo};
use std::fs;
use std::process::Command;

/// Upper bound on stored command length (chars). Bounds snapshot size and
/// limits how much potentially sensitive argv content we retain.
pub const MAX_COMMAND_CHARS: usize = 256;

/// Upper bound on processes returned per snapshot (safety cap).
pub const MAX_PROCESSES: usize = 4096;

pub struct LinuxPlatform;

/// Parsed fields from one `/proc/<pid>/stat` line.
#[derive(Debug, PartialEq, Eq)]
struct StatFields {
    ppid: u32,
    state_code: char,
    starttime_ticks: i64,
}

/// Parse a `/proc/<pid>/stat` line. `comm` may contain spaces or parens,
/// so split at the *last* `)`: everything before it is `pid (comm)`,
/// everything after is the remaining whitespace-separated fields where
/// field 3 is state, 4 ppid, 22 starttime.
fn parse_stat_line(line: &str) -> Option<StatFields> {
    let close = line.rfind(')')?;
    let after = line[close + 1..].split_whitespace().collect::<Vec<_>>();
    // after[0]=state(3) [1]=ppid(4) … [11]=utime(14) [12]=stime(15) … [19]=starttime(22)
    if after.len() < 20 {
        return None;
    }
    Some(StatFields {
        state_code: after[0].chars().next()?,
        ppid: after[1].parse().ok()?,
        starttime_ticks: after[19].parse().ok()?,
    })
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

fn read_one_proc(pid: u32) -> Option<RawProcess> {
    let base = format!("/proc/{pid}");
    let stat = fs::read_to_string(format!("{base}/stat")).ok()?;
    let fields = parse_stat_line(&stat)?;
    let name = fs::read_to_string(format!("{base}/comm"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let cmdline = fs::read(format!("{base}/cmdline")).unwrap_or_default();
    let command = truncate_chars(
        &cmdline
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(String::from_utf8_lossy)
            .collect::<Vec<_>>()
            .join(" "),
        MAX_COMMAND_CHARS,
    );
    let cwd = std::fs::read_link(format!("{base}/cwd"))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Some(RawProcess {
        pid,
        ppid: fields.ppid,
        name,
        command,
        cwd,
        state_code: fields.state_code,
        starttime_ticks: fields.starttime_ticks,
    })
}

impl Platform for LinuxPlatform {
    fn processes(&self) -> Vec<RawProcess> {
        let entries = fs::read_dir("/proc").map_or(Vec::new(), |rd| {
            rd.filter_map(Result::ok).collect::<Vec<_>>()
        });
        let mut out = Vec::new();
        for entry in entries {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Ok(pid) = name.parse::<u32>() {
                if let Some(proc) = read_one_proc(pid) {
                    out.push(proc);
                    if out.len() >= MAX_PROCESSES {
                        break;
                    }
                }
            }
        }
        out
    }

    fn windows(&self) -> Vec<WindowInfo> {
        let output = Command::new("hyprctl").args(["clients", "-j"]).output();
        let stdout = match output {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
            _ => return Vec::new(),
        };
        parse_hypr_clients(&stdout)
    }

    fn git_info(&self, dir: &str) -> GitInfo {
        // Branch from the .git/HEAD file: no subprocess, works even for
        // repos with hooks/aliases configured. Handles `.git` dirs; linked
        // worktrees (`.git` files) report is_repo=true with unknown branch
        // rather than a guessed one.
        let head_path = format!("{dir}/.git/HEAD");
        let head = fs::read_to_string(&head_path).ok();
        let branch = head.as_deref().and_then(|h| {
            let h = h.trim();
            h.strip_prefix("ref: refs/heads/")
                .map(str::to_string)
                .filter(|b| !b.is_empty())
        });
        // A `.git` file (worktree pointer) still marks a repo.
        let git_pointer = std::path::Path::new(&format!("{dir}/.git")).exists();
        if head.is_none() && !git_pointer {
            return GitInfo::default();
        }
        let clean = git_status_clean(dir);
        GitInfo {
            is_repo: true,
            branch,
            clean,
        }
    }

    fn boot_epoch(&self) -> i64 {
        fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|s| {
                s.lines().find_map(|l| {
                    l.strip_prefix("btime ")
                        .and_then(|v| v.trim().parse::<i64>().ok())
                })
            })
            .unwrap_or(-1)
    }

    fn clock_ticks_per_sec(&self) -> i64 {
        100 // Linux CLK_TCK; universal on supported kernels, documented here.
    }

    fn hostname(&self) -> String {
        fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".to_string())
    }
}

/// `git status --porcelain=v1` cleanliness check. `None` when git is
/// missing, the dir is unreadable, or git fails — unknown, not clean.
fn git_status_clean(dir: &str) -> Option<bool> {
    let output = Command::new("git")
        .args(["-C", dir, "status", "--porcelain=v1"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(output.stdout.is_empty())
}

// ---------------------------------------------------------------------------
// Minimal `hyprctl clients -j` parser.
// ---------------------------------------------------------------------------
//
// We need six fields per client: address, class, initialClass, title, pid,
// workspace.name. A full JSON parser would add a dependency; instead this
// extracts top-level objects of the outer array and reads the needed fields
// with brace-aware scanning plus JSON string unescaping. Malformed input
// yields fewer clients, never a panic.

/// Split the outer JSON array into its top-level object substrings.
fn split_top_objects(json: &str) -> Vec<&str> {
    let bytes = json.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    let mut start = None;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 && start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(s) = start.take() {
                        out.push(&json[s..=i]);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Unescape a JSON string body (without surrounding quotes).
fn unescape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                if let Ok(code) = u32::from_str_radix(&hex, 16) {
                    out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Extract a top-level string field `"key": "value"` from an object slice.
/// Returns `None` when absent or not a JSON string.
fn field_string(obj: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut search = obj;
    loop {
        let pos = search.find(&needle)?;
        let mut rest = search[pos + needle.len()..].trim_start();
        if !rest.starts_with(':') {
            search = &search[pos + needle.len()..];
            continue;
        }
        rest = rest[1..].trim_start();
        if !rest.starts_with('"') {
            return None;
        }
        // Scan the quoted string honoring escapes.
        let mut escaped = false;
        for (i, c) in rest[1..].char_indices() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                return Some(unescape_json(&rest[1..1 + i]));
            }
        }
        return None;
    }
}

/// Extract a top-level integer field `"key": 123`.
fn field_int(obj: &str, key: &str) -> Option<i64> {
    let needle = format!("\"{key}\"");
    let mut search = obj;
    loop {
        let pos = search.find(&needle)?;
        let mut rest = search[pos + needle.len()..].trim_start();
        if !rest.starts_with(':') {
            search = &search[pos + needle.len()..];
            continue;
        }
        rest = rest[1..].trim_start();
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '-'))
            .unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        if let Ok(v) = rest[..end].parse::<i64>() {
            return Some(v);
        }
        search = &search[pos + needle.len()..];
    }
}

/// Extract the nested `"workspace": {… "name": "…"}` object slice.
fn nested_object<'a>(obj: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let mut search = obj;
    loop {
        let base = search.find(&needle)?;
        let abs_base = obj.len() - search.len() + base;
        let mut rest = search[base + needle.len()..].trim_start();
        if !rest.starts_with(':') {
            search = &search[base + needle.len()..];
            continue;
        }
        rest = rest[1..].trim_start();
        if !rest.starts_with('{') {
            return None;
        }
        let abs_open = obj.len() - rest.len();
        // Brace-match from the opening brace, string-aware.
        let bytes = obj.as_bytes();
        let mut depth = 0usize;
        let mut in_str = false;
        let mut escaped = false;
        for (i, &b) in bytes.iter().enumerate().skip(abs_open) {
            if in_str {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    in_str = false;
                }
                continue;
            }
            match b {
                b'"' => in_str = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&obj[abs_open..=i]);
                    }
                }
                _ => {}
            }
        }
        let _ = abs_base;
        return None;
    }
}

fn parse_one_client(obj: &str) -> Option<WindowInfo> {
    Some(WindowInfo {
        address: field_string(obj, "address")?,
        class: field_string(obj, "class").unwrap_or_default(),
        initial_class: field_string(obj, "initialClass").unwrap_or_default(),
        title: field_string(obj, "title").unwrap_or_default(),
        workspace: nested_object(obj, "workspace")
            .and_then(|w| field_string(w, "name"))
            .unwrap_or_default(),
        pid: field_int(obj, "pid").unwrap_or(-1).max(0) as u32,
    })
}

/// Parse `hyprctl clients -j` output. Best-effort: clients missing an
/// address are skipped; everything else degrades to empty/zero.
pub fn parse_hypr_clients(json: &str) -> Vec<WindowInfo> {
    split_top_objects(json)
        .into_iter()
        .filter_map(parse_one_client)
        .collect()
}

/// Test seam: classify helpers over injected maps (unit tests live here so
/// fixtures stay next to the parser).
#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashMap;

    pub fn parse_stat_for_test(line: &str) -> Option<(u32, char, i64)> {
        parse_stat_line(line).map(|f| (f.ppid, f.state_code, f.starttime_ticks))
    }

    pub fn clients_for_test(json: &str) -> Vec<HashMap<String, String>> {
        parse_hypr_clients(json)
            .into_iter()
            .map(|w| {
                HashMap::from([
                    ("address".to_string(), w.address),
                    ("class".to_string(), w.class),
                    ("title".to_string(), w.title),
                    ("workspace".to_string(), w.workspace),
                    ("pid".to_string(), w.pid.to_string()),
                ])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::parse_stat_for_test;
    use super::*;

    const FIXTURE_CLIENTS: &str = include_str!("../../tests/fixtures/hypr_clients.json");

    #[test]
    fn stat_line_parses_standard_fields() {
        // pid (comm) state ppid … utime stime … starttime
        let line = "328900 (opencode) R 328892 328900 34818 0 -1 4194304 100 0 0 0 25976 4459 0 0 20 0 9 0 12345 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0";
        assert_eq!(parse_stat_for_test(line), Some((328892, 'R', 12345)));
    }

    #[test]
    fn stat_line_handles_spacy_comm() {
        let line = "1 (my prog (x)) S 0 1 1 0 -1 4194560 100 0 0 0 0 0 0 0 20 0 1 0 50 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0";
        assert_eq!(parse_stat_for_test(line), Some((0, 'S', 50)));
    }

    #[test]
    fn stat_line_rejects_garbage() {
        assert_eq!(parse_stat_for_test(""), None);
        assert_eq!(parse_stat_for_test("123 (x) R"), None);
    }

    #[test]
    fn truncate_chars_respects_boundary() {
        assert_eq!(truncate_chars("abcdef", 6), "abcdef");
        assert_eq!(truncate_chars("abcdef", 3), "abc");
    }

    #[test]
    fn unescape_handles_common_sequences() {
        assert_eq!(unescape_json("a\\\"b\\\\c\\n"), "a\"b\\c\n");
        assert_eq!(unescape_json("\\u00e9"), "é");
    }

    #[test]
    fn hypr_fixture_parses_two_clients() {
        let clients = parse_hypr_clients(FIXTURE_CLIENTS);
        assert_eq!(clients.len(), 2);
        let agent = clients
            .iter()
            .find(|c| c.class == "org.omarchy.agent")
            .unwrap();
        assert_eq!(agent.address, "0xaaaafa4573b0");
        assert_eq!(agent.pid, 328892);
        assert_eq!(agent.workspace, "1");
        assert!(agent.title.starts_with("OC |"));
        let term = clients.iter().find(|c| c.class == "foot").unwrap();
        assert_eq!(term.pid, 351520);
    }

    #[test]
    fn hypr_parser_survives_malformed_input() {
        assert!(parse_hypr_clients("").is_empty());
        assert!(parse_hypr_clients("not json").is_empty());
        assert!(parse_hypr_clients("[{\"no_address\": 1}]").is_empty());
        // Missing optional fields degrade, address is the only hard requirement.
        let minimal = parse_hypr_clients("[{\"address\": \"0x1\"}]");
        assert_eq!(minimal.len(), 1);
        assert_eq!(minimal[0].pid, 0);
        assert_eq!(minimal[0].title, "");
    }

    #[test]
    fn git_info_detects_repo_branch_and_clean() {
        let dir = std::env::temp_dir().join("pitwall-m1-gitinfo-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let plat = LinuxPlatform;
        // Not a real repo (no objects) → branch known from HEAD file, clean unknown.
        let info = plat.git_info(&dir.to_string_lossy());
        assert!(info.is_repo);
        assert_eq!(info.branch.as_deref(), Some("main"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_info_rejects_non_repo() {
        let dir = std::env::temp_dir().join("pitwall-m1-notrepo-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let plat = LinuxPlatform;
        let info = plat.git_info(&dir.to_string_lossy());
        assert!(!info.is_repo);
        assert_eq!(info.branch, None);
        assert_eq!(info.clean, None);
        let _ = fs::remove_dir_all(&dir);
    }
}
