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
/// `exe_name` is the basename of the executable (never the full path, which
/// may contain versions, usernames, or layout details); it disambiguates
/// cases where argv[0] is a shim or wrapper (e.g. mise shims).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawProcess {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub command: String,
    pub exe_name: String,
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

/// One terminal launch request.
///
/// This is the whole input to the single Terminal_Launch_Path
/// ([`Platform::launch_terminal`]). An empty `command` means "just open an
/// interactive terminal in `directory`" — exactly today's `resume`
/// behaviour. A non-empty `command` is a fixed argument vector whose first
/// element must be an absolute program path; it is never a shell string and
/// is never interpolated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSpec<'a> {
    /// Absolute, already-validated directory to open the terminal in.
    pub directory: &'a str,
    /// Program and arguments to run, or empty for an interactive shell.
    pub command: &'a [String],
}

/// One observed chat number lease.
///
/// A lease is a file the running `pitwall chat` process created with
/// `O_EXCL` under the runtime directory
/// (`$XDG_RUNTIME_DIR/pitwall/chat-NNN.lease`), containing its own pid. It
/// is *not* a counter: it remembers nothing about past numbers and vanishes
/// with the process (explicit release plus `Drop`, tmpfs as the last net).
///
/// Reading a lease says only "this file exists and names this pid". Whether
/// that pid is a live `pitwall chat` is a separate question, answered by
/// [`crate::context::is_live_pitwall_chat`] against an observed process
/// list — the platform never validates liveness itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChatLease {
    /// Chat number, `1..=999` (the file name's three digits).
    pub number: u16,
    /// Pid recorded in the lease body by its owner.
    pub pid: u32,
}

/// Inline raster image protocol support, detected at runtime.
///
/// These are *protocol* names, not emulator names: the variant says which
/// wire protocol the terminal answered a query with, never which program is
/// on the other end. Nothing in Pitwall reads a terminal's identity, and no
/// behaviour is derived from one (Requirement 28.11).
///
/// `None` is a first-class outcome, not a failure: every documented chat
/// function works without inline images (Requirement 28.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineImage {
    /// Terminal graphics protocol (APC `_G…`) acknowledged.
    Kitty,
    /// Sixel graphics advertised in a primary device-attributes reply.
    Sixel,
    /// No inline raster image protocol detected. Textual output only.
    None,
}

/// Capabilities the core needs from the host OS.
///
/// Observation (`processes`, `windows`, `git_info`, …) is side-effect free.
/// Actions (`launch_terminal`, `focus_window_address`) are explicit,
/// user-initiated, fixed-form operations — never arbitrary commands.
/// M4 safety levels: focus is Level 1, terminal launch is Level 2.
/// There is no Level 3+ (agent start, arbitrary execution) in this trait.
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
    /// Open a terminal at an already-validated absolute directory, detached,
    /// optionally running a fixed argument vector inside it.
    ///
    /// This is the *only* terminal-launch path: widened rather than
    /// siblinged so the two callers cannot drift apart and so no second
    /// launch implementation exists. Implementations must not invoke a
    /// shell, must not interpolate, and must reach the terminal only
    /// through the OS terminal abstraction — never a named emulator and
    /// never an emulator-specific application-identity or window-class
    /// argument.
    fn launch_terminal(&self, spec: &TerminalSpec<'_>) -> Result<(), String>;
    /// Chat number leases currently present in the runtime directory.
    ///
    /// Observation only: reads and parses `chat-NNN.lease` names and their
    /// pid bodies. It performs no liveness check, unlinks nothing, and
    /// returns an empty vec when the directory is absent or unreadable —
    /// "nothing observed", never an error. Callers decide which leases are
    /// stale (see [`crate::context::is_live_pitwall_chat`]).
    fn chat_leases(&self) -> Vec<ChatLease>;
    /// Runtime probe: will this terminal accept an inline raster image?
    ///
    /// Probes *protocol support* by asking the terminal and reading its
    /// answer. It must never read, infer, or branch on an emulator's
    /// identity, must never block (hard deadline, small byte cap), and must
    /// leave terminal settings exactly as it found them on every path —
    /// including error paths. Anything unexpected is
    /// [`InlineImage::None`], which is a fully supported outcome.
    fn inline_image_capability(&self) -> InlineImage;
    /// Focus a compositor window by its validated address (`0x…` hex).
    /// Best-effort: fails cleanly when the window is gone.
    fn focus_window_address(&self, address: &str) -> Result<(), String>;
    /// Byte counters from `/proc/PID/io` (read/write activity evidence).
    /// Read-only; `None` when unreadable. Never persisted by the core.
    fn process_io(&self, pid: u32) -> Option<IoCounters>;
    /// Bounded terminal text for one window's root PID. Best-effort and
    /// honest: terminals without a safe scrollback API (e.g. foot) return
    /// `TerminalText::Unavailable` — never attempted invasively.
    fn terminal_text(&self, pid: u32, class: &str) -> TerminalText;
}

/// Cumulative byte counters (Linux `/proc/PID/io` subset). Absolute values
/// are activity evidence ("has done I/O"); only in-memory deltas across two
/// close reads may suggest current liveness. Never semantic claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IoCounters {
    pub read_bytes: u64,
    pub write_bytes: u64,
}

/// Terminal text sampling outcome. `Lines` carries at most the first and
/// last 10 non-empty lines within a hard byte cap; `Unavailable` names the
/// reason so consumers (and the AI) know observation was impossible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalText {
    Lines {
        first: Vec<String>,
        last: Vec<String>,
    },
    Unavailable {
        reason: &'static str,
    },
}
