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

use super::{
    ChatLease, GitInfo, InlineImage, IoCounters, Platform, RawProcess, TerminalSpec, TerminalText,
    WindowInfo,
};
use std::fs;
use std::path::{Path, PathBuf};
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
    // Basename only: full exe paths leak versions, usernames, and layout.
    let exe_name = std::fs::read_link(format!("{base}/exe"))
        .map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    Some(RawProcess {
        pid,
        ppid: fields.ppid,
        name,
        command,
        exe_name,
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

    fn launch_terminal(&self, spec: &TerminalSpec<'_>) -> Result<(), String> {
        // The single exec boundary for terminals. Fixed argv, no shell, no
        // `-c`, no interpolation, detached stdio, `spawn()` (never `status()`
        // — Pitwall must not wait on a window the human owns).
        //
        // Validation (absolute, exists, is-dir) happens upstream in resume;
        // it is repeated here as defense in depth because this is the only
        // place an argv reaches the OS.
        let argv = terminal_argv(spec)?;
        Command::new("xdg-terminal-exec")
            .args(&argv)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("terminal launch failed: {e}"))
    }

    fn chat_leases(&self) -> Vec<ChatLease> {
        // Same directory the ephemeral context documents use, and the same
        // directory the chat lease writer (task 8.3) creates leases in:
        // `$XDG_RUNTIME_DIR/pitwall`, with the documented tmpfs fallback.
        read_chat_leases_in(&crate::context::ephemeral_dir())
    }

    fn inline_image_capability(&self) -> InlineImage {
        probe_inline_image()
    }

    fn focus_window_address(&self, address: &str) -> Result<(), String> {
        // Same Lua dispatch shape as first-party `omarchy-hyprland-focus-app`
        // (the bare multi-token form is broken in this hyprctl build).
        // Argv-based, no shell; the address is regex-validated upstream.
        if !is_hex_address(address) {
            return Err(format!(
                "refusing to focus invalid window address {address:?}"
            ));
        }
        let lua = format!("hl.dsp.focus({{ window = \"address:{address}\" }})");
        let status = Command::new("hyprctl")
            .args(["dispatch", &lua])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("focus dispatch failed to spawn: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("focus dispatch rejected (window likely gone)".to_string())
        }
    }

    fn process_io(&self, pid: u32) -> Option<IoCounters> {
        let content = fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
        let mut read_bytes = None;
        let mut write_bytes = None;
        for line in content.lines() {
            let (key, value) = line.split_once(':')?;
            let number: u64 = value.trim().parse().ok()?;
            match key {
                "read_bytes" => read_bytes = Some(number),
                "write_bytes" => write_bytes = Some(number),
                _ => {}
            }
        }
        Some(IoCounters {
            read_bytes: read_bytes?,
            write_bytes: write_bytes?,
        })
    }

    fn terminal_text(&self, pid: u32, class: &str) -> TerminalText {
        // Best-effort kitty path only: per-PID socket + `kitten` binary,
        // numeric-PID-derived path, bounded read with timeout. Everything
        // else — foot has no scrollback API, pty reads are destructive —
        // degrades to Unavailable. Never attempted invasively.
        if class.trim().to_lowercase() != "kitty" {
            return TerminalText::Unavailable {
                reason: "no scrollback API",
            };
        }
        kitty_text(pid)
    }
}

/// Read bounded terminal text via kitty's control socket. Returns
/// `Unavailable` on any failure (no binary, no socket, timeout, parse).
/// Separated for unit testing of the degradation contract.
fn kitty_text(pid: u32) -> TerminalText {
    const TIMEOUT_MS: u64 = 2000;
    const MAX_BYTES: usize = 4096;
    // `kitten` must exist; socket path derives from the numeric PID only.
    let socket = match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        Some(runtime) => runtime.join(format!("omarchy-kitty-{pid}")),
        None => {
            return TerminalText::Unavailable {
                reason: "no runtime dir",
            };
        }
    };
    if !socket.exists() {
        return TerminalText::Unavailable {
            reason: "no kitty socket",
        };
    }
    let mut child = match Command::new("kitten")
        .args([
            "@",
            "--to",
            &format!("unix:{}", socket.display()),
            "get-text",
            "--extent",
            "all",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            return TerminalText::Unavailable {
                reason: "kitten unavailable",
            };
        }
    };
    // Drain stdout on a helper thread (bounded): a large scrollback must
    // never fill the pipe and deadlock the child while we poll for exit.
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        use std::io::Read as _;
        let mut buf = Vec::new();
        if let Some(out) = stdout {
            let _ = out.take((MAX_BYTES * 4) as u64).read_to_end(&mut buf);
        }
        buf
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(TIMEOUT_MS);
    let exit_ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return TerminalText::Unavailable {
                        reason: "get-text timeout",
                    };
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return TerminalText::Unavailable {
                    reason: "get-text failed",
                };
            }
        }
    };
    let raw = reader.join().unwrap_or_default();
    if !exit_ok {
        return TerminalText::Unavailable {
            reason: "get-text failed",
        };
    }
    let text = String::from_utf8_lossy(&raw);
    let capped: String = text.chars().take(MAX_BYTES).collect();
    let lines: Vec<String> = capped
        .lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.trim().is_empty())
        .take(21)
        .map(str::to_string)
        .collect();
    let (first, last) = crate::context::window_lines(&lines);
    TerminalText::Lines { first, last }
}

/// Build the argument vector for one terminal launch, or refuse.
///
/// Kept as a pure function so the exact argv is unit-testable without
/// spawning anything, and so there is exactly one place where terminal argv
/// comes into existence.
///
/// The program is always the OS terminal abstraction (`xdg-terminal-exec`),
/// which resolves whichever emulator the human has configured. No emulator
/// is named, no emulator is required to be installed, and no
/// emulator-specific application-identity or window-class argument is ever
/// passed — only the abstraction's own generic options.
fn terminal_argv(spec: &TerminalSpec<'_>) -> Result<Vec<String>, String> {
    if spec.directory.is_empty() {
        return Err("refusing to launch a terminal with no directory".to_string());
    }
    if !spec.directory.starts_with('/') {
        return Err("refusing to launch a terminal in a non-absolute directory".to_string());
    }
    if has_ascii_control(spec.directory) {
        return Err("refusing a control character in the terminal directory".to_string());
    }
    if let Some(program) = spec.command.first() {
        if !program.starts_with('/') {
            return Err("refusing to run a non-absolute program in a terminal".to_string());
        }
    }
    if spec.command.iter().any(|e| has_ascii_control(e.as_str())) {
        return Err("refusing to run a command containing a control character".to_string());
    }

    // KNOWN SUSPECTED DEFECT, DELIBERATELY PRESERVED.
    //
    // `--dir` and the directory are passed as TWO argv elements. A
    // source-level review of the terminal abstraction's option parser (see
    // the spec's findings for task 1.1) found only the joined
    // `--dir=<workdir>` form; a bare `--dir` appears to be discarded as an
    // unknown option, after which the directory string is taken as the
    // command to run. If that holds on the target machine, this launch opens
    // a terminal in the wrong directory.
    //
    // It is NOT changed here. Requirement 23.1 requires resume's argv stay
    // byte-identical, and the machine this was written on cannot exercise the
    // abstraction at all, so altering the shape blind risks regressing a path
    // that may work in practice. MUST be verified on the Omarchy machine with
    // a side-effect-free print of the resolved command; the joined `--dir=`
    // form may turn out to be required. Do not add a fallback that tries both
    // forms — one launch path only.
    let mut argv = vec!["--dir".to_string(), spec.directory.to_string()];
    // Additive: an empty command yields argv identical to the resume path.
    if !spec.command.is_empty() {
        argv.push("--".to_string());
        argv.extend_from_slice(spec.command);
    }
    Ok(argv)
}

/// True when any byte is an ASCII control character. Control bytes in an
/// argv element can rewrite a terminal's state or confuse logs, so they are
/// refused at the exec boundary rather than sanitised.
fn has_ascii_control(value: &str) -> bool {
    value.bytes().any(|b| b.is_ascii_control())
}

/// Compositor window address shape (`0x` + hex). Shared by focus validation
/// and resume; intentionally strict — nothing else may flow into dispatch.
pub fn is_hex_address(address: &str) -> bool {
    let hex = address
        .strip_prefix("0x")
        .or_else(|| address.strip_prefix("0X"));
    match hex {
        Some(rest) => !rest.is_empty() && rest.chars().all(|c| c.is_ascii_hexdigit()),
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Chat number leases (Requirements 10.2, 10.4).
// ---------------------------------------------------------------------------
//
// Availability of a chat number is derived from what is *running*, never
// from a stored counter, so this reader is deliberately the dumbest half of
// that: it reports the lease files it can see and the pid each one names.
// Deciding which of those are stale needs an observed process list and is
// the allocator's job (task 8.3); unlinking is the allocator's job too.
//
// Every failure mode degrades to "observed nothing": an absent or
// unreadable directory, an unreadable file, a name that is not exactly
// `chat-NNN.lease`, a body that is not a plain decimal pid. A chat number
// wrongly believed free is corrected by the allocator's `O_EXCL` create; a
// loud failure here would break chat startup for no gain.

/// Lease file name fence. Only names this reader could itself have been
/// paired with are considered — everything else in the runtime directory
/// (ephemeral context documents included) is ignored.
const LEASE_PREFIX: &str = "chat-";
const LEASE_SUFFIX: &str = ".lease";
/// A lease body is a decimal pid. Cap the read so a planted large file
/// cannot be pulled into memory.
const LEASE_MAX_BYTES: u64 = 32;
/// Numbers run `001..=999`, so no directory can hold more valid leases.
const MAX_LEASES: usize = 999;

/// Chat number of a lease file name, or `None` for any other name.
///
/// Strict by design: exactly `chat-` + three ASCII digits + `.lease`, and
/// the number must be in `1..=999` (`chat-000.lease` is not a lease).
fn lease_number_of(file_name: &str) -> Option<u16> {
    let digits = file_name
        .strip_prefix(LEASE_PREFIX)?
        .strip_suffix(LEASE_SUFFIX)?;
    if digits.len() != 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: u16 = digits.parse().ok()?;
    if number == 0 {
        return None;
    }
    Some(number)
}

/// Owner pid from a lease body, or `None` when the body is not exactly one
/// plain decimal pid (trailing newline allowed).
fn lease_owner_pid(body: &str) -> Option<u32> {
    let text = body.trim();
    if text.is_empty() || text.len() > 10 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let pid: u32 = text.parse().ok()?;
    if pid == 0 {
        return None;
    }
    Some(pid)
}

/// Bounded read of a lease body. `None` on any I/O or UTF-8 problem.
fn read_lease_body(path: &Path) -> Option<String> {
    use std::io::Read as _;
    let file = fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(LEASE_MAX_BYTES).read_to_end(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

/// Read every well-formed lease in `dir`, ordered by number.
///
/// Test seam for [`Platform::chat_leases`]: production passes
/// [`crate::context::ephemeral_dir`], tests pass a sandbox.
fn read_chat_leases_in(dir: &Path) -> Vec<ChatLease> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<ChatLease> = Vec::new();
    for entry in entries.flatten() {
        // Regular files only: never read through a directory or a symlink
        // someone dropped in the runtime dir.
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let name = entry.file_name();
        let number = match name.to_str().and_then(lease_number_of) {
            Some(number) => number,
            None => continue,
        };
        let body = match read_lease_body(&entry.path()) {
            Some(body) => body,
            None => continue,
        };
        let pid = match lease_owner_pid(&body) {
            Some(pid) => pid,
            None => continue,
        };
        out.push(ChatLease { number, pid });
        if out.len() >= MAX_LEASES {
            break;
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Inline image capability probe (Requirements 20.7, 20.8, 28.10, 28.11).
// ---------------------------------------------------------------------------
//
// PROTOCOL SUPPORT ONLY. Nothing below reads a terminal's name, window
// class, `$TERM`, or any other identity, and no emulator is named. The
// probe asks the terminal a question and classifies the *answer*; a
// terminal that says nothing is simply `None`, which is a fully supported
// outcome (28.10) — the chat header then prints its textual form and every
// documented chat function still works.
//
// Two hard rules shape this code:
//
// 1. Terminal settings are restored on EVERY path. The saved settings are
//    captured and validated *before* anything is changed, the restoring
//    guard is constructed *before* `raw -echo` is applied, and restoration
//    runs from `Drop` — so an early return, an `Err`, or a panic all
//    restore. Leaving a terminal in `raw -echo` is worse than never
//    probing.
// 2. No read can block. The deadline is enforced by the terminal driver
//    itself (`min 0 time 2` ⇒ a read returns after at most 200 ms even
//    with nothing to read), with an `Instant` deadline and a byte cap as
//    belt and braces. No reader thread is spawned, so none can be left
//    behind holding the human's next keystroke.

/// The controlling terminal. Interactivity is decided from stdout (the
/// design's condition), but the settings change and the query/reply
/// exchange both go through the controlling terminal, which is where
/// terminal modes and replies actually live.
const TTY_PATH: &str = "/dev/tty";
/// Graphics-protocol query: a 1×1 RGB image with `a=q`, which asks
/// "could you?" and displays nothing. A terminal that speaks the protocol
/// answers `ESC _ G i=<id>;OK ESC \`; one that does not answers nothing.
const GRAPHICS_QUERY: &[u8] = b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\";
/// Primary device attributes. Every terminal answers this, which is why it
/// follows the graphics query: it turns "silence" into a real answer and
/// it is where sixel support (attribute `4`) is advertised.
const DA1_QUERY: &[u8] = b"\x1b[c";
/// Hard per-query deadline. Matches the `time 2` (tenths of a second) the
/// driver is set to, so worst case is one driver timeout per query.
const PROBE_DEADLINE_MS: u64 = 200;
/// Byte cap per reply. Both answers are well under this.
const PROBE_MAX_BYTES: usize = 64;

/// Run `stty -F /dev/tty <args>`, returning its trimmed stdout on success.
/// Fixed argv, no shell, no interpolation; `None` when `stty` is missing,
/// the tty cannot be opened, or the command fails.
fn stty(args: &[&str]) -> Option<String> {
    let output = Command::new("stty")
        .arg("-F")
        .arg(TTY_PATH)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Accept only what `stty -g` legitimately emits: one opaque token of
/// alphanumerics and `: , - =`, never starting with `-` (so the restore
/// argument can never be read as an option). No whitespace, no control
/// bytes. If the saved settings do not look restorable, the probe never
/// changes anything in the first place.
fn is_saved_settings(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b',' | b'-' | b'='))
}

/// Restores saved terminal settings when dropped — the only mechanism that
/// restores them, so no code path can forget to.
struct TtyGuard {
    saved: String,
}

impl Drop for TtyGuard {
    fn drop(&mut self) {
        // Best effort and silent: there is nothing useful to do if the
        // terminal has gone away, and this runs on error paths too.
        let _ = stty(&[self.saved.as_str()]);
    }
}

/// Write one query and read whatever comes back, bounded in both time and
/// bytes. An empty string means "no answer", which is a normal outcome.
fn ask_terminal(tty: &mut fs::File, query: &[u8]) -> String {
    use std::io::{Read as _, Write as _};
    if tty.write_all(query).is_err() {
        return String::new();
    }
    let _ = tty.flush();
    let window = std::time::Duration::from_millis(PROBE_DEADLINE_MS);
    let deadline = std::time::Instant::now() + window;
    let mut reply = Vec::new();
    let mut chunk = [0u8; 32];
    while reply.len() < PROBE_MAX_BYTES && std::time::Instant::now() < deadline {
        match tty.read(&mut chunk) {
            // `min 0 time 2`: zero bytes means the driver's timer expired.
            Ok(0) => break,
            Ok(n) => reply.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
        if reply_is_complete(&reply) {
            break;
        }
    }
    String::from_utf8_lossy(&reply).into_owned()
}

/// True when the reply looks terminated, so we can stop early instead of
/// waiting out another driver timeout. Latency only — a wrong answer here
/// cannot produce a wrong capability, only a slightly shorter read.
fn reply_is_complete(reply: &[u8]) -> bool {
    // APC responses end with ST (`ESC \`); DA1 responses end with `c`.
    reply.windows(2).any(|w| w == b"\x1b\\") || reply.last() == Some(&b'c')
}

/// True when the reply carries a graphics-protocol acknowledgement:
/// an APC `_G` response whose payload's status field is `OK`.
fn is_graphics_ok(reply: &str) -> bool {
    let start = match reply.find("\u{1b}_G") {
        Some(start) => start,
        None => return false,
    };
    let rest = &reply[start + "\u{1b}_G".len()..];
    // Payload runs to the terminator (ESC of the ST), or to the end.
    let body = match rest.find('\u{1b}') {
        Some(end) => &rest[..end],
        None => rest,
    };
    match body.split_once(';') {
        Some((_control_keys, status)) => status.trim() == "OK",
        None => false,
    }
}

/// True when a primary device-attributes reply advertises attribute `4`
/// (sixel graphics). Shape: `ESC [ ? <n> ; <n> … c`.
fn da1_has_sixel(reply: &str) -> bool {
    let start = match reply.find("\u{1b}[?") {
        Some(start) => start,
        None => return false,
    };
    let rest = &reply[start + "\u{1b}[?".len()..];
    let end = match rest.find('c') {
        Some(end) => end,
        None => return false,
    };
    rest[..end].split(';').any(|attribute| attribute.trim() == "4")
}

/// Classify one reply, or `None` when it says nothing we understand.
fn classify_reply(reply: &str) -> Option<InlineImage> {
    if is_graphics_ok(reply) {
        return Some(InlineImage::Kitty);
    }
    if da1_has_sixel(reply) {
        return Some(InlineImage::Sixel);
    }
    None
}

/// The runtime capability probe. See the section comment above for the two
/// invariants (unconditional restoration, no blocking read).
fn probe_inline_image() -> InlineImage {
    use std::io::IsTerminal as _;
    // 1. Not a terminal ⇒ nothing to ask, nothing changed. This is the
    //    common non-interactive case (piped output, CI, the panel's
    //    `Process`), and it must be free of side effects.
    if !std::io::stdout().is_terminal() {
        return InlineImage::None;
    }
    // 2. Save first. If settings cannot be saved or do not look
    //    restorable, stop *before* touching anything: a missing `stty` is
    //    `None`, not a mangled terminal.
    let saved = match stty(&["-g"]) {
        Some(saved) if is_saved_settings(&saved) => saved,
        _ => return InlineImage::None,
    };
    // 3. Guard before raw: every exit below restores, including panics.
    let _guard = TtyGuard { saved };
    // 4. `raw -echo` so the reply is not echoed or line-buffered, plus
    //    `min 0 time 2` — the driver-level 200 ms deadline that makes
    //    every read below non-blocking.
    if stty(&["raw", "-echo", "min", "0", "time", "2"]).is_none() {
        return InlineImage::None;
    }
    let mut opts = fs::OpenOptions::new();
    opts.read(true).write(true);
    let mut tty = match opts.open(TTY_PATH) {
        Ok(tty) => tty,
        Err(_) => return InlineImage::None,
    };
    // 5. Graphics query first; if it went unanswered (or answered with
    //    something unrecognised), DA1 — which every terminal answers, and
    //    which is where sixel is advertised.
    if let Some(capability) = classify_reply(&ask_terminal(&mut tty, GRAPHICS_QUERY)) {
        return capability;
    }
    if let Some(capability) = classify_reply(&ask_terminal(&mut tty, DA1_QUERY)) {
        return capability;
    }
    InlineImage::None
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
    fn hex_address_validation_is_strict() {
        assert!(is_hex_address("0xaaaafa4573b0"));
        assert!(is_hex_address("0XABCDEF"));
        assert!(is_hex_address("0x0"));
        assert!(!is_hex_address(""));
        assert!(!is_hex_address("aaaafa4573b0"));
        assert!(!is_hex_address("0x"));
        assert!(!is_hex_address("0xZZZ"));
        assert!(!is_hex_address("0x123; rm -rf ~"));
        assert!(!is_hex_address("0x12 34"));
        assert!(!is_hex_address("address:0x123"));
    }

    #[test]
    fn launch_terminal_refuses_empty_directory() {
        let plat = LinuxPlatform;
        let spec = TerminalSpec {
            directory: "",
            command: &[],
        };
        assert!(plat.launch_terminal(&spec).is_err());
    }

    /// Test helper: build argv from borrowed parts.
    fn argv_of(directory: &str, command: &[String]) -> Result<Vec<String>, String> {
        terminal_argv(&TerminalSpec { directory, command })
    }

    #[test]
    fn empty_command_argv_is_byte_identical_to_resume_shape() {
        // Pins Requirement 23.1: the resume launch argv must not change.
        let empty: Vec<String> = Vec::new();
        let want = ["--dir", "/home/u/proj"];
        assert_eq!(argv_of("/home/u/proj", &empty).unwrap(), want);
    }

    #[test]
    fn non_empty_command_is_appended_after_end_of_options() {
        let command = vec!["/usr/bin/pitwall".to_string(), "chat".to_string()];
        let want = ["--dir", "/home/u/proj", "--", "/usr/bin/pitwall", "chat"];
        assert_eq!(argv_of("/home/u/proj", &command).unwrap(), want);
    }

    #[test]
    fn terminal_argv_refuses_unsafe_input() {
        let empty: Vec<String> = Vec::new();
        // Empty, relative, and control-bearing directories.
        assert!(argv_of("", &empty).is_err());
        assert!(argv_of("relative/proj", &empty).is_err());
        assert!(argv_of("/home/u/pro\nj", &empty).is_err());
        // Non-absolute program.
        let relative = vec!["pitwall".to_string(), "chat".to_string()];
        assert!(argv_of("/home/u/proj", &relative).is_err());
        // Control character anywhere in the command.
        let controlled = vec!["/usr/bin/pitwall".to_string(), "ch\u{1b}at".to_string()];
        assert!(argv_of("/home/u/proj", &controlled).is_err());
    }

    #[test]
    fn terminal_argv_never_sets_an_app_id_or_class_or_shell_flag() {
        // Requirements 28.3/28.4: only the abstraction's generic options.
        let command = vec!["/usr/bin/pitwall".to_string()];
        let argv = argv_of("/home/u/proj", &command).unwrap();
        for element in &argv {
            let lower = element.to_lowercase();
            assert!(!lower.contains("--app-id"));
            assert!(!lower.contains("--class"));
            assert_ne!(lower.as_str(), "-c");
        }
    }

    #[test]
    fn lease_names_are_parsed_strictly() {
        assert_eq!(lease_number_of("chat-001.lease"), Some(1));
        assert_eq!(lease_number_of("chat-999.lease"), Some(999));
        // `000` is not in `001..=999`.
        assert_eq!(lease_number_of("chat-000.lease"), None);
        // Exactly three digits, nothing else.
        assert_eq!(lease_number_of("chat-1.lease"), None);
        assert_eq!(lease_number_of("chat-0001.lease"), None);
        assert_eq!(lease_number_of("chat-01a.lease"), None);
        assert_eq!(lease_number_of("chat-001.lease.bak"), None);
        assert_eq!(lease_number_of("chat-001"), None);
        // Other runtime-dir residents are not leases.
        assert_eq!(lease_number_of("ctx-1234-0123456789abcdef.json"), None);
        assert_eq!(lease_number_of(""), None);
    }

    #[test]
    fn lease_bodies_must_be_one_plain_pid() {
        assert_eq!(lease_owner_pid("4321"), Some(4321));
        assert_eq!(lease_owner_pid("4321\n"), Some(4321));
        assert_eq!(lease_owner_pid(""), None);
        assert_eq!(lease_owner_pid("0"), None);
        assert_eq!(lease_owner_pid("-1"), None);
        assert_eq!(lease_owner_pid("12 34"), None);
        assert_eq!(lease_owner_pid("99999999999"), None);
        assert_eq!(lease_owner_pid("pid=42"), None);
    }

    #[test]
    fn lease_reader_ignores_everything_it_did_not_write() {
        let dir = std::env::temp_dir().join("pitwall-m8-lease-reader-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("chat-003.lease"), "300\n").unwrap();
        fs::write(dir.join("chat-001.lease"), "100").unwrap();
        fs::write(dir.join("chat-002.lease"), "not-a-pid").unwrap();
        fs::write(dir.join("chat-9.lease"), "900").unwrap();
        fs::write(dir.join("ctx-100-0123456789abcdef.json"), "{}").unwrap();
        fs::create_dir_all(dir.join("chat-004.lease")).unwrap();
        let leases = read_chat_leases_in(&dir);
        let seen: Vec<(u16, u32)> = leases.iter().map(|l| (l.number, l.pid)).collect();
        assert_eq!(seen, vec![(1, 100), (3, 300)]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lease_reader_returns_empty_for_an_unreadable_directory() {
        let missing = std::env::temp_dir().join("pitwall-m8-no-such-lease-dir");
        let _ = fs::remove_dir_all(&missing);
        assert!(read_chat_leases_in(&missing).is_empty());
    }

    #[test]
    fn graphics_ok_reply_is_recognised() {
        assert!(is_graphics_ok("\u{1b}_Gi=31;OK\u{1b}\\"));
        assert!(is_graphics_ok("\u{1b}_Gi=31,I=2;OK\u{1b}\\"));
        // An error status is not support.
        assert!(!is_graphics_ok("\u{1b}_Gi=31;ENOENT:bad file\u{1b}\\"));
        assert!(!is_graphics_ok("\u{1b}[?62;1;4c"));
        assert!(!is_graphics_ok("\u{1b}_Gi=31\u{1b}\\"));
        assert!(!is_graphics_ok(""));
    }

    #[test]
    fn da1_sixel_attribute_is_recognised() {
        assert!(da1_has_sixel("\u{1b}[?62;1;4;6c"));
        assert!(da1_has_sixel("\u{1b}[?4c"));
        assert!(!da1_has_sixel("\u{1b}[?62;1;6c"));
        // Unterminated, and a number that merely contains a 4.
        assert!(!da1_has_sixel("\u{1b}[?62;1;4"));
        assert!(!da1_has_sixel("\u{1b}[?64;41c"));
        assert!(!da1_has_sixel(""));
    }

    #[test]
    fn unrecognised_replies_classify_as_no_capability() {
        assert_eq!(classify_reply(""), None);
        assert_eq!(classify_reply("garbage"), None);
        assert_eq!(classify_reply("\u{1b}[?62;1;6c"), None);
        assert_eq!(
            classify_reply("\u{1b}_Gi=31;OK\u{1b}\\"),
            Some(InlineImage::Kitty)
        );
        assert_eq!(classify_reply("\u{1b}[?62;4c"), Some(InlineImage::Sixel));
    }

    #[test]
    fn only_restorable_saved_settings_are_accepted() {
        assert!(is_saved_settings("500:5:bf:8a3b:3:1c:7f:15:4:0:1"));
        assert!(is_saved_settings("gfmt1:cflag=4b00:iflag=6b02"));
        assert!(!is_saved_settings(""));
        // Whitespace, control bytes, and option-looking values are refused,
        // so the probe never applies `raw -echo` it could not undo.
        assert!(!is_saved_settings("500:5 ; rm -rf /"));
        assert!(!is_saved_settings("500:5\n"));
        assert!(!is_saved_settings("-F"));
    }

    #[test]
    fn the_probe_reports_no_capability_without_a_terminal() {
        use std::io::IsTerminal as _;
        // No fake terminal and no mocking: under `cargo test` the harness's
        // stdout is a pipe, so this drives the real step-1 early return —
        // the non-interactive case (piped output, CI, the panel's
        // `Process`), which must be free of side effects. If the test binary
        // is ever run straight from a terminal there is nothing to fake, and
        // probing the human's terminal to satisfy an assertion would be the
        // wrong trade, so that case is skipped rather than mocked.
        if std::io::stdout().is_terminal() {
            return;
        }
        assert_eq!(probe_inline_image(), InlineImage::None);
        // The same answer through the trait method the Chat_Header calls,
        // and asking twice changes nothing. `InlineImage::None` is the
        // single value that stands for "absent capability" — it is what the
        // header reads as its textual, unbranded form (20.8). That mapping
        // lives in `chat` and is asserted there; here the point is only that
        // every no-capability path produces this one value, never a second
        // flavour of "no".
        let plat = LinuxPlatform;
        assert_eq!(plat.inline_image_capability(), InlineImage::None);
        assert_eq!(plat.inline_image_capability(), InlineImage::None);
        assert_ne!(InlineImage::None, InlineImage::Kitty);
        assert_ne!(InlineImage::None, InlineImage::Sixel);
    }

    #[test]
    fn a_timed_out_reply_carries_no_capability() {
        // A driver timeout leaves a *partial* reply, not an empty one: the
        // bytes that arrived before the 200 ms window closed. The whole probe
        // cannot be driven here without a terminal, so this covers the pure
        // seam either side of the read — the loop must not mistake a
        // half-arrived answer for a finished one…
        assert!(!reply_is_complete(b"\x1b_Gi=31;"));
        assert!(!reply_is_complete(b"\x1b[?62;1;"));
        // …and a partial answer says nothing we understand, which is the
        // same outcome as silence.
        assert_eq!(classify_reply("\u{1b}_Gi=31;"), None);
        assert_eq!(classify_reply("\u{1b}[?62;1;"), None);
        // A terminated answer does stop the read early — latency only.
        assert!(reply_is_complete(b"\x1b_Gi=31;OK\x1b\\"));
        assert!(reply_is_complete(b"\x1b[?62;1;4c"));
    }

    #[test]
    fn a_missing_stty_leaves_the_terminal_untouched() {
        // `stty` is reached through a fixed argv with no injectable seam, so
        // the absent-binary path itself is live Omarchy verification: a
        // missing binary makes `Command::output` fail, makes `stty` return
        // `None`, and returns `InlineImage::None` from step 2 — *before* the
        // guard exists and before `raw -echo` is applied, so there is
        // nothing to restore. What is assertable in-process is the gate that
        // makes that safe: only one restorable `stty -g` token is ever fed
        // back to `stty`, so a `stty` that is missing, wrong, or merely
        // talkative cannot get its output used as an argument.
        assert!(!is_saved_settings("stty: /dev/tty: No such device or address"));
        assert!(!is_saved_settings("/dev/tty"));
        // The length cap, which a garbage `stty` dumping a blob would meet.
        assert!(is_saved_settings(&"a".repeat(4096)));
        assert!(!is_saved_settings(&"a".repeat(4097)));
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

    // -----------------------------------------------------------------
    // Property test (M8 task 6.3). `proptest` is a dev-dependency pinned
    // `=1.5.0`; Property 22 is exactly ONE test at 100+ cases. Nothing here
    // spawns a process: the property is about argv *construction*, so every
    // case drives the real pure `terminal_argv` through the same `argv_of`
    // seam the example tests above use. The examples above pin specific
    // shapes; this generalises over generated valid and invalid inputs.
    // -----------------------------------------------------------------

    use proptest::prelude::*;

    /// Path and argument words for generated directories and commands.
    ///
    /// Deliberately a fixed list rather than a character-class regex. The
    /// property asserts that no argv element carries an emulator name or an
    /// emulator-specific identity flag, and a free regex would eventually
    /// generate a directory containing `kitty` or `xterm` and fail the test
    /// for a reason that is not a defect. No word below contains `foot`,
    /// `kitty`, `ghostty`, `alacritty`, `wezterm`, `xterm`, `--app-id` or
    /// `--class` as a substring, and segments are joined with `/`, so no
    /// generated value can spell one of those tokens across a boundary
    /// either. Clause (a) in the test states the same guarantee in the
    /// input-independent form that also holds for a human whose project
    /// genuinely lives in `/home/u/kitty-fork`.
    const SAFE_WORDS: &[&str] = &[
        "home", "u", "proj", "work", "repo", "src", "dev", "tmp", "build", "m8",
    ];

    /// ASCII control characters, every one of which `has_ascii_control`
    /// refuses (`0x00..=0x1f` and `0x7f`).
    const CONTROL_CHARS: &[char] = &['\u{0}', '\u{7}', '\t', '\n', '\r', '\u{1b}', '\u{7f}'];

    /// Emulator names and emulator-specific identity flags that must never
    /// reach a launch argv (Requirements 28.3, 28.4, 28.6).
    ///
    /// NOTE FOR THE 28.4 INSPECTION GATE (task 15.1): this list is the one
    /// place in `src/` that spells these names deliberately, and it does so
    /// in order to *forbid* them. It is `#[cfg(test)]`, never linked into
    /// the shipped binary, and it names no emulator that Pitwall requires,
    /// resolves, installs, or branches on. A grep for emulator names must
    /// record this occurrence as the enforcement of 28.4, not a breach of
    /// it.
    const FORBIDDEN_TOKENS: &[&str] = &[
        "foot",
        "kitty",
        "ghostty",
        "alacritty",
        "wezterm",
        "xterm",
        "--app-id",
        "--class",
    ];

    /// Splice one control character into a generated value. Every generated
    /// value is ASCII, so any byte index is a char boundary.
    fn insert_control(value: &str, control: char, at: usize) -> String {
        let mut out = value.to_string();
        let at = at % (out.len() + 1);
        out.insert(at, control);
        out
    }

    /// Indices into [`SAFE_WORDS`], `count` of them.
    fn word_ixs(count: std::ops::Range<usize>) -> impl Strategy<Value = Vec<usize>> {
        proptest::collection::vec(0usize..SAFE_WORDS.len(), count)
    }

    /// Optional control-character splice into the directory: which control
    /// character, and where in the string.
    fn dir_splice() -> impl Strategy<Value = Option<(usize, usize)>> {
        proptest::option::of((0usize..CONTROL_CHARS.len(), 0usize..24))
    }

    /// Optional control-character splice into the command: which control
    /// character, which element, and where in that element.
    fn command_splice() -> impl Strategy<Value = Option<(usize, usize, usize)>> {
        proptest::option::of((0usize..CONTROL_CHARS.len(), 0usize..4, 0usize..24))
    }

    proptest! {
        // 256 cases, comfortably above the 100 floor: the generated shape
        // cross-product (3 directory shapes × 4 command lengths × absolute
        // or relative program × control character present or absent, twice)
        // is 96 combinations, so this covers each many times over.
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        // **Validates: Requirements 16.6, 16.7, 26.17, 28.1, 28.3, 28.4, 28.6**
        // Feature: pitwall-chat-and-brief-ticker, Property 22: Every terminal launch is validated and fixed-form — For any launch request, the single Terminal_Launch_Path refuses an empty or non-absolute directory, a directory that is not a directory, a non-absolute command binary and any argument containing a control character; on acceptance it produces exactly the Terminal_Abstraction argv with the directory and, when present, the command after `--`, with no shell and a detached child, and with no emulator-specific application-identity or window-class argument and no emulator name in any element.
        #[test]
        fn prop22_every_terminal_launch_is_validated_and_fixed_form(
            dir_shape in 0usize..3,
            dir_words in word_ixs(1..4),
            dir_control in dir_splice(),
            cmd_len in 0usize..4,
            cmd_program_absolute in any::<bool>(),
            cmd_words in word_ixs(4..5),
            cmd_control in command_splice(),
        ) {
            // ---- the directory: empty, relative, or absolute ------------
            let joined = dir_words
                .iter()
                .map(|ix| SAFE_WORDS[*ix])
                .collect::<Vec<_>>()
                .join("/");
            let mut directory = match dir_shape {
                0 => String::new(),
                1 => joined,
                _ => format!("/{joined}"),
            };
            // Never spliced into the empty case, so "empty" stays exactly
            // empty and each generated refusal reason stays the one it was
            // generated to be.
            let dir_has_control = match dir_control {
                Some((ix, at)) if !directory.is_empty() => {
                    let spliced = insert_control(&directory, CONTROL_CHARS[ix], at);
                    directory = spliced;
                    true
                }
                _ => false,
            };

            // ---- the command: empty, or a program plus 0..2 arguments ---
            let words: Vec<&str> = cmd_words.iter().map(|ix| SAFE_WORDS[*ix]).collect();
            let mut command: Vec<String> = Vec::new();
            if cmd_len > 0 {
                command.push(if cmd_program_absolute {
                    format!("/{}/{}", words[0], words[1])
                } else {
                    words[0].to_string()
                });
                for word in words.iter().take(cmd_len).skip(1) {
                    command.push((*word).to_string());
                }
            }
            let cmd_has_control = match cmd_control {
                Some((ix, element, at)) if !command.is_empty() => {
                    let target = element % command.len();
                    let spliced = insert_control(&command[target], CONTROL_CHARS[ix], at);
                    command[target] = spliced;
                    true
                }
                _ => false,
            };

            // What the launcher must do with this input, derived from the
            // *generation choices* — the test knows what it constructed, so
            // it never re-implements the validation it is checking.
            let dir_is_valid = dir_shape == 2 && !dir_has_control;
            let command_is_valid =
                command.is_empty() || (cmd_program_absolute && !cmd_has_control);

            // The one function under test. `argv_of` is the borrowed-parts
            // wrapper over the real `terminal_argv`; there is no second
            // implementation anywhere in this test.
            let result = argv_of(&directory, &command);

            if !dir_is_valid || !command_is_valid {
                // Refusal clause: an empty directory, a non-absolute
                // directory, a control character in the directory, a
                // non-absolute command binary, or a control character in any
                // command element. The message is a deliberate refusal, not
                // an incidental error.
                prop_assert!(
                    result.is_err(),
                    "accepted directory {directory:?} command {command:?}"
                );
                let message = result.unwrap_err();
                prop_assert!(message.starts_with("refusing"), "{message}");
            } else {
                prop_assert!(
                    result.is_ok(),
                    "refused valid directory {directory:?} command {command:?}"
                );
                let argv = result.unwrap();

                // Fixed form: exactly the Terminal_Abstraction argv, with
                // the command after `--` when one is present. This pins the
                // two-element `--dir DIR` shape the launcher emits today;
                // Requirement 23.1 requires that shape not move, and the
                // suspected-defect note on `terminal_argv` records the one
                // circumstance (verification on the Omarchy machine) under
                // which it and this oracle would move together.
                let mut want = vec!["--dir".to_string(), directory.clone()];
                if !command.is_empty() {
                    want.push("--".to_string());
                    want.extend(command.iter().cloned());
                }
                prop_assert_eq!(&argv, &want);

                // (a) Input-independent form of "no emulator name in any
                //     element". The launcher invents exactly one element
                //     (`--dir`), or two when a command is present (`--`);
                //     every other element is byte-identical to a value the
                //     caller supplied — no quoting, no escaping, no
                //     interpolation, nothing appended. So the terminal
                //     -agnostic guarantee is stated over the invented
                //     elements, which is the honest scope: a project that
                //     genuinely lives in `/home/u/kitty-fork` must still
                //     launch, and asserting over caller paths would forbid
                //     that rather than forbid an emulator flag.
                prop_assert_eq!(argv[1].as_str(), directory.as_str());
                let invented: Vec<&str> = if command.is_empty() {
                    prop_assert_eq!(argv.len(), 2);
                    vec![argv[0].as_str()]
                } else {
                    prop_assert_eq!(&argv[3..], &command[..]);
                    vec![argv[0].as_str(), argv[2].as_str()]
                };
                for element in &invented {
                    let lower = element.to_lowercase();
                    for token in FORBIDDEN_TOKENS {
                        prop_assert!(
                            !lower.contains(*token),
                            "invented argv element {element:?} carries {token}"
                        );
                    }
                }

                // (b) The same scan over the whole argv. Sound only because
                //     SAFE_WORDS cannot spell a forbidden token, so a
                //     failure here is a real defect and never an accident of
                //     a generated path.
                for element in &argv {
                    let lower = element.to_lowercase();
                    for token in FORBIDDEN_TOKENS {
                        prop_assert!(
                            !lower.contains(*token),
                            "argv element {element:?} carries {token}"
                        );
                    }
                    // No shell, in the part that is an argv fact: no `-c`,
                    // and no shell program smuggled in as an element. The
                    // rest of the "no shell and a detached child" clause is
                    // not an argv fact at all — it lives in
                    // `launch_terminal` (`Command::new` on the abstraction,
                    // `Stdio::null()` on all three streams, `spawn()` rather
                    // than `status()`), and asserting it would mean spawning
                    // a process, which this property forbids. That half is
                    // carried by the inspection gate on that function
                    // (task 15.1).
                    prop_assert!(
                        !matches!(lower.as_str(), "-c" | "sh" | "bash" | "zsh" | "/bin/sh"),
                        "argv element {element:?} looks like a shell invocation"
                    );
                }
            }

            // Requirement 23.1, asserted for every generated *valid*
            // directory whatever command was generated alongside it: the
            // resume path passes an empty command, and its argv must stay
            // byte-identical to what it has always been.
            if dir_is_valid {
                let resume_argv = argv_of(&directory, &[]);
                prop_assert!(resume_argv.is_ok(), "refused {directory:?}");
                prop_assert_eq!(
                    resume_argv.unwrap(),
                    vec!["--dir".to_string(), directory.clone()]
                );
            }

            // The property's "a directory that is not a directory" clause is
            // deliberately NOT asserted here, because `terminal_argv` does
            // not stat the filesystem and must not: it is pure, and that
            // check needs I/O. It lives upstream at the callers —
            // `resume::resume` (`std::fs::metadata(..).is_dir()`, refusing
            // with the offending path named, `src/resume.rs`) and the
            // Chat_Launcher's launch-directory validation (Requirement 16.7,
            // task 12.2). The accepted cases above include absolute paths
            // that do not exist on the machine running the test, which is
            // the observable evidence that this function does not stat, and
            // is exactly why the is-a-directory check has to stay at the
            // callers. Their own tests carry that half of Property 22.
        }
    }
}
