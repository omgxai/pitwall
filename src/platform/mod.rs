//! OS isolation boundary.
//!
//! The core ([`crate::collector`]) only ever talks to the [`Platform`]
//! trait. All syscalls, `/proc` parsing, `hyprctl` invocation, and `git`
//! subprocesses live in the OS-specific implementation (`linux` for now).
//! A future macOS/Windows port adds a new module implementing this trait —
//! the core stays untouched.

pub mod linux;

/// One observed OS process, in platform-neutral form.
///
/// `started_at_epoch` is seconds since the Unix epoch, or `-1` when the
/// platform could not determine it. `command` is truncated by the platform
/// (256 chars) to bound snapshot size and limit sensitive-data exposure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawProcess {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub command: String,
    pub cwd: String,
    /// Raw single-letter state from the OS (`R`, `S`, `D`, `T`, `Z`, …).
    /// Interpretation happens in the core, not the platform.
    pub state_code: char,
    /// Ticks since boot (Linux `stat.starttime`); `-1` when unknown.
    pub starttime_ticks: i64,
}

/// One compositor window that may host a terminal session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// Compositor window address (e.g. Hyprland `0x…`). Opaque to the core.
    pub address: String,
    /// Window class / app-id (e.g. `foot`, `org.omarchy.agent`).
    pub class: String,
    pub initial_class: String,
    pub title: String,
    pub workspace: String,
    /// PID of the window's root client process.
    pub pid: u32,
}

/// Git context for a directory, as observed (never inferred).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitInfo {
    pub is_repo: bool,
    pub branch: Option<String>,
    /// `None` when unknown (not a repo, or `git` unavailable/failed).
    pub clean: Option<bool>,
}

/// Capabilities the core needs from the host OS. Pure observation only:
/// no killing, no focusing, no configuration changes.
pub trait Platform {
    fn processes(&self) -> Vec<RawProcess>;
    fn windows(&self) -> Vec<WindowInfo>;
    fn git_info(&self, dir: &str) -> GitInfo;
    /// Seconds since Unix epoch at boot (`/proc/stat btime` on Linux),
    /// or `-1` when unknown.
    fn boot_epoch(&self) -> i64;
    /// Ticks per second for `starttime_ticks` conversion (Linux: 100).
    fn clock_ticks_per_sec(&self) -> i64;
    fn hostname(&self) -> String;
}
