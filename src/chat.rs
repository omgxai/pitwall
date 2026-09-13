//! Pitwall Chat: the native chat surface (M8, Part B).
//!
//! The whole module is organised around one invariant: a chat's identity is
//! captured **once**, at startup, and can never change afterwards
//! (Requirement 11). [`ChatDescriptor`] is therefore a plain value with
//! private fields, read-only accessors, no setters, no `&mut self` method
//! and no interior mutability. There is also **no config read anywhere in
//! this module**: the harness and model arrive as arguments from the
//! `pitwall chat` startup path, so a later `pitwall config set` cannot
//! reach a running chat (11.3, 11.4) while the next chat picks up the
//! changed values at its own startup (11.5).
//!
//! Immutability is enforced structurally rather than by convention, so
//! several concurrent chats provably cannot influence each other's
//! configuration (21.3).
//!
//! Validation is delegated, never duplicated: model shape comes from
//! [`crate::summary::valid_model`], the harness id set from
//! [`crate::agents::KNOWN`], and the context session id shape from
//! [`crate::resume::is_session_id`].

/// Maximum characters accepted from one chat question. Bounds what may
/// travel into the single argv element carrying the question.
pub const MAX_CHAT_QUESTION_CHARS: usize = 2000;

/// Maximum characters of a context label (project name or [`WORKSPACE_LABEL`]).
pub const MAX_CONTEXT_LABEL_CHARS: usize = 48;

/// Maximum characters of the whole window title (Chat_Title_Grammar).
pub const MAX_CHAT_TITLE_CHARS: usize = 200;

/// Harness timeout for chat, deliberately the *same* value the explicit
/// summary path already uses: one configured agent, one waiting budget.
/// Referencing [`crate::summary::DEFAULT_TIMEOUT_SECS`] instead of copying
/// 120 keeps the two from drifting apart.
pub const CHAT_TIMEOUT_SECS: u64 = crate::summary::DEFAULT_TIMEOUT_SECS;

/// Rendered stand-in for an empty model: the harness uses its own default
/// and the human is told so, in the header and in the window title (16.3).
pub const AGENT_DEFAULT_LABEL: &str = "agent default";

/// Context label used when a chat is scoped to the whole observed
/// workspace rather than to one session's project (16.4).
pub const WORKSPACE_LABEL: &str = "Workspace";

/// The interpunct that separates Chat_Title_Grammar fields. A context
/// label may never contain it, otherwise the title would stop parsing.
const TITLE_SEPARATOR_CHAR: char = '\u{00B7}';

/// Characters a context label may carry: anything that neither breaks a
/// single presented line (controls) nor collides with the title separator.
fn is_label_char(c: char) -> bool {
    !c.is_control() && c != TITLE_SEPARATOR_CHAR
}

/// A chat's identity, captured once at startup and immutable for the
/// process lifetime.
///
/// Fields are private and exposed only through borrowing accessors: there
/// is no setter, no `&mut self` method, and no `Cell`/`RefCell`/`Mutex`
/// anywhere in the type, so the values recorded at capture time are the
/// values every later stage (header, title, responder, resume bridge,
/// state artifact) observes (11.3, 11.4, 11.8, 21.3).
///
/// `Clone` is derived because a descriptor is a value that may be handed
/// to several read-only consumers; cloning still yields something nobody
/// can mutate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatDescriptor {
    /// `1..=999`, rendered as three digits by [`ChatDescriptor::number_text`].
    number: u16,
    /// A canonical [`crate::agents::KNOWN`] id (`opencode`/`claude`/`codex`).
    harness: String,
    /// A [`crate::summary::valid_model`] id, or empty for the agent default.
    model: String,
    /// Project name, or [`WORKSPACE_LABEL`] when there is no context session.
    context_label: String,
    /// Pitwall-local opaque session id (`sess_` + 16 hex) when the chat is
    /// scoped to one observed session. Never a pid and never a window
    /// address: this is the same id `state.json` already exposes under
    /// `resumable`.
    context_session_id: Option<String>,
    /// Unix seconds at capture time.
    started_at_epoch: i64,
    /// Absolute, existing directory the chat runs against.
    project_dir: String,
}

impl ChatDescriptor {
    /// Capture a chat's identity once.
    ///
    /// Every value is validated here so no later stage has to re-check and
    /// no invalid descriptor can exist:
    ///
    /// - `number` within `1..=999` (10.1);
    /// - `harness` a known [`crate::agents::KNOWN`] id — *installed* is a
    ///   separate, environment-dependent check the startup path performs
    ///   before calling this (11.6);
    /// - `model` empty (agent default) or [`crate::summary::valid_model`] (11.7);
    /// - `context_label` non-empty, free of control characters and of the
    ///   title separator, and at most [`MAX_CONTEXT_LABEL_CHARS`] characters
    ///   — collapsing and truncating a raw project name into that shape is
    ///   the sanitiser's job (§4.4), this is the guard that the result was
    ///   applied;
    /// - `context_session_id` shaped by [`crate::resume::is_session_id`] (9.4);
    /// - `started_at_epoch` not negative, so the header and the state
    ///   artifact never present a placeholder clock value;
    /// - `project_dir` absolute and an existing directory, validated the
    ///   same way [`crate::resume::resume`] validates a resume target.
    ///
    /// The harness and model arrive as arguments precisely so that this
    /// module never reads the config file (11.1, 11.4).
    pub fn capture(
        number: u16,
        harness: &str,
        model: &str,
        context_label: &str,
        context_session_id: Option<&str>,
        started_at_epoch: i64,
        project_dir: &str,
    ) -> Result<ChatDescriptor, String> {
        if !(1..=999).contains(&number) {
            return Err(format!("chat number {number} is outside 001..999"));
        }
        let known = crate::agents::KNOWN
            .iter()
            .find(|a| a.id == harness)
            .ok_or_else(|| format!("unknown harness '{harness}'"))?;
        if !model.is_empty() && !crate::summary::valid_model(model) {
            return Err(format!("refusing malformed model id ({model:?})"));
        }
        if context_label.is_empty() {
            return Err("refusing empty context label".to_string());
        }
        if context_label.chars().count() > MAX_CONTEXT_LABEL_CHARS {
            return Err(format!(
                "context label exceeds {MAX_CONTEXT_LABEL_CHARS} characters (refusing)"
            ));
        }
        if !context_label.chars().all(is_label_char) {
            return Err("refusing unsanitised context label".to_string());
        }
        if let Some(id) = context_session_id {
            if !crate::resume::is_session_id(id) {
                return Err("malformed session ID (refusing)".to_string());
            }
        }
        if started_at_epoch < 0 {
            return Err("refusing negative start time".to_string());
        }
        if !project_dir.starts_with('/') {
            return Err("refusing non-absolute project dir".to_string());
        }
        match std::fs::metadata(project_dir) {
            Ok(md) if md.is_dir() => {}
            Ok(_) => return Err(format!("project path is not a directory: {project_dir}")),
            Err(_) => return Err(format!("project directory unavailable: {project_dir}")),
        }
        Ok(ChatDescriptor {
            number,
            harness: known.id.to_string(),
            model: model.to_string(),
            context_label: context_label.to_string(),
            context_session_id: context_session_id.map(str::to_string),
            started_at_epoch,
            project_dir: project_dir.to_string(),
        })
    }

    /// The allocated chat number, `1..=999`.
    pub fn number(&self) -> u16 {
        self.number
    }

    /// Zero-padded three-digit form used by the header, the window title
    /// and the lease file name.
    pub fn number_text(&self) -> String {
        format!("{:03}", self.number)
    }

    /// The captured harness id.
    pub fn harness(&self) -> &str {
        &self.harness
    }

    /// The captured model id; empty means the harness default.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The model as presented to the human: the captured id, or
    /// [`AGENT_DEFAULT_LABEL`] when none was configured (16.3).
    pub fn model_label(&self) -> &str {
        if self.model.is_empty() {
            AGENT_DEFAULT_LABEL
        } else {
            &self.model
        }
    }

    /// The captured context label.
    pub fn context_label(&self) -> &str {
        &self.context_label
    }

    /// The captured Pitwall-local context session id, when the chat is
    /// scoped to one session.
    pub fn context_session_id(&self) -> Option<&str> {
        self.context_session_id.as_deref()
    }

    /// Unix seconds at capture time.
    pub fn started_at_epoch(&self) -> i64 {
        self.started_at_epoch
    }

    /// The validated absolute project directory.
    pub fn project_dir(&self) -> &str {
        &self.project_dir
    }
}

// ---------------------------------------------------------------------------
// Chat_Title_Grammar — task 8.2 (design §4.4)
//
// The window title is not decoration: together with the Chat_Process_Identity
// it *is* how Pitwall recognises one of its own chat windows (16.5, 17.2), so
// the parser below is deliberately unforgiving. Everything in this section
// reads the descriptor through the accessors above and adds no mutable state;
// the only function that touches the outside world is the OSC emitter at the
// end, and it is separated from the pure formatter precisely so the grammar
// stays testable without a terminal.
// ---------------------------------------------------------------------------

/// The literal that opens every chat title, and the same literal the panel
/// presents on a chat entry (16.2, 18.2).
pub const CHAT_LABEL: &str = "Pitwall Chat";

/// The field separator: space, U+00B7, space. Three characters, **four
/// bytes** — every length decision in this module counts characters, never
/// bytes, so the multi-byte separator cannot skew a cap or split a char.
const TITLE_SEPARATOR: &str = " \u{00B7} ";

/// Chat_Title_Grammar:
///
/// ```text
/// Pitwall Chat NNN · <harness> · <model|agent default> · <context label>
/// ```
///
/// Compose one title from already-validated parts. Private because the only
/// legitimate sources of those parts are a [`ChatDescriptor`] (via
/// [`format_title`]) and a parsed [`ChatTitle`] (via [`ChatTitle::render`]).
fn compose_title(
    number_text: &str,
    harness: &str,
    model_label: &str,
    context_label: &str,
) -> String {
    let sep = TITLE_SEPARATOR;
    format!("{CHAT_LABEL} {number_text}{sep}{harness}{sep}{model_label}{sep}{context_label}")
}

/// Truncate to at most `max` **characters** (never bytes).
fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// The context label exactly as [`format_title`] will emit it.
///
/// Normally this is the descriptor's label verbatim. It differs only when the
/// full title would exceed [`MAX_CHAT_TITLE_CHARS`], in which case the label
/// — the one field that carries no identity, unlike the number, harness and
/// model — gives up its tail so the cap holds.
///
/// The label can never be squeezed out entirely. Worst case: 13 characters of
/// `"Pitwall Chat "`, 3 digits, three 3-character separators (9), the longest
/// harness id (`opencode`, 8) and the longest model
/// [`crate::summary::valid_model`] accepts (128 — its charset is ASCII only,
/// so its byte bound is also a character bound) sum to 161, leaving 39
/// characters of label budget out of 200. Any label of 39 characters or fewer
/// is therefore always emitted whole, and only the 40..=48 range can be
/// trimmed, and only behind an unusually long model id.
pub fn title_context_label(d: &ChatDescriptor) -> String {
    let head = compose_title(&d.number_text(), d.harness(), d.model_label(), "");
    let room = MAX_CHAT_TITLE_CHARS.saturating_sub(head.chars().count());
    let budget = room.min(MAX_CONTEXT_LABEL_CHARS).max(1);
    debug_assert!(room >= 39, "title head grew beyond the documented worst case");
    truncate_chars(d.context_label(), budget)
}

/// Render a descriptor as its Chat_Title_Grammar window title (16.2).
///
/// Pure: no I/O, no clock, no config. `model_label()` supplies the
/// [`AGENT_DEFAULT_LABEL`] literal for an empty model (16.3), and the
/// descriptor already carries [`WORKSPACE_LABEL`] when the chat has no
/// context session (16.4), so this function invents nothing.
pub fn format_title(d: &ChatDescriptor) -> String {
    let label = title_context_label(d);
    let title = compose_title(&d.number_text(), d.harness(), d.model_label(), &label);
    debug_assert!(title.chars().count() <= MAX_CHAT_TITLE_CHARS);
    title
}

/// The facts a well-formed chat window title carries.
///
/// Produced only by [`parse_title`], so its existence is itself the proof
/// that a title matched the grammar. `harness` is a `&'static str` borrowed
/// from [`crate::agents::KNOWN`]: an unknown harness cannot be represented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatTitle {
    number: u16,
    harness: &'static str,
    /// Empty when the title carried [`AGENT_DEFAULT_LABEL`].
    model: String,
    context_label: String,
}

impl ChatTitle {
    /// The chat number, `1..=999`.
    pub fn number(&self) -> u16 {
        self.number
    }

    /// The three-digit form, byte-identical to the digits in the title —
    /// which is also the lease file name the corroborating process signal is
    /// looked up under (17.2).
    pub fn number_text(&self) -> String {
        format!("{:03}", self.number)
    }

    /// The harness id, guaranteed to be one of [`crate::agents::KNOWN`].
    pub fn harness(&self) -> &'static str {
        self.harness
    }

    /// The model id; empty means the title said [`AGENT_DEFAULT_LABEL`].
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The model as presented, mirroring [`ChatDescriptor::model_label`].
    pub fn model_label(&self) -> &str {
        if self.model.is_empty() {
            AGENT_DEFAULT_LABEL
        } else {
            &self.model
        }
    }

    /// The context label, exactly as it appeared in the title.
    pub fn context_label(&self) -> &str {
        &self.context_label
    }

    /// Rebuild the title these facts came from.
    ///
    /// `render(parse(t)) == t` for every well-formed `t`: nothing is trimmed
    /// or normalised on the way in, and a parsed title is already within both
    /// caps, so nothing is trimmed on the way out either.
    pub fn render(&self) -> String {
        let n = self.number_text();
        compose_title(&n, self.harness, self.model_label(), &self.context_label)
    }
}

/// Parse a window title, strictly.
///
/// `None` for anything that is not *exactly* a Chat_Title_Grammar title. The
/// collector calls this on every observed window title, so looseness here
/// would let an ordinary terminal borrow half of a chat's identity; the other
/// half (a live `pitwall chat` lease pid inside that window's process tree)
/// is checked separately and cannot be forged by printing text (17.2, 17.3).
///
/// Rejected, in order:
///
/// 1. more than [`MAX_CHAT_TITLE_CHARS`] characters;
/// 2. any control character anywhere (a title that could re-enter an escape
///    sequence is not a title we will ever emit);
/// 3. a field count other than four — the title is split on the separator, so
///    an extra separator *anywhere*, including inside the label, fails here;
/// 4. a first field that is not `Pitwall Chat ` followed by exactly three
///    ASCII digits (`1`, `0001`, `12a`, ` 12` and non-ASCII digits all fail);
/// 5. the number `000`, i.e. outside `001..=999`;
/// 6. a harness that is not a [`crate::agents::KNOWN`] id, matched exactly;
/// 7. a model that is neither the [`AGENT_DEFAULT_LABEL`] literal nor
///    [`crate::summary::valid_model`] (which also rejects an empty field);
/// 8. a context label that is empty, longer than
///    [`MAX_CONTEXT_LABEL_CHARS`], or carries a control character or the
///    separator character — the same `is_label_char` predicate
///    [`ChatDescriptor::capture`] guards with.
///
/// **Why the split is unambiguous.** The context label is the last field and
/// the only one that may contain a space: a harness id has none, a
/// `valid_model` id has none (its charset is alphanumerics and `/-_.:`), and
/// the one model stand-in with a space, `agent default`, contains no `·`.
/// Because no field may contain `·` at all, the separator occurs exactly
/// three times in a well-formed title, so splitting from the left and
/// demanding four fields recovers the original fields verbatim — including a
/// label with spaces, leading spaces or trailing spaces, none of which are
/// touched.
pub fn parse_title(title: &str) -> Option<ChatTitle> {
    if title.chars().count() > MAX_CHAT_TITLE_CHARS {
        return None;
    }
    if title.chars().any(char::is_control) {
        return None;
    }

    let fields: Vec<&str> = title.split(TITLE_SEPARATOR).collect();
    if fields.len() != 4 {
        return None;
    }

    // Field 0: the literal, one space, three digits.
    let after_label = fields[0].strip_prefix(CHAT_LABEL)?;
    let digits = after_label.strip_prefix(' ')?;
    // `len()` is bytes, but a 3-byte run of ASCII digits is also 3 characters,
    // so the two agree here and nowhere else in this module.
    if digits.len() != 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: u16 = digits.parse().ok()?;
    if !(1..=999).contains(&number) {
        return None;
    }

    // Field 1: a known harness id, exactly.
    let harness = crate::agents::KNOWN.iter().find(|a| a.id == fields[1])?.id;

    // Field 2: the agent-default literal, or a valid model id.
    let model = if fields[2] == AGENT_DEFAULT_LABEL {
        String::new()
    } else if crate::summary::valid_model(fields[2]) {
        fields[2].to_string()
    } else {
        return None;
    };

    // Field 3: a sanitised, bounded, non-empty label.
    let context_label = fields[3];
    if context_label.is_empty()
        || context_label.chars().count() > MAX_CONTEXT_LABEL_CHARS
        || !context_label.chars().all(is_label_char)
    {
        return None;
    }

    Some(ChatTitle {
        number,
        harness,
        model,
        context_label: context_label.to_string(),
    })
}

/// Reduce a raw project name to something the title grammar and a single
/// presented line can both carry (design §4.4).
///
/// - whitespace runs — including newlines and tabs — collapse to one space,
///   and leading or trailing whitespace disappears entirely;
/// - every other control character is dropped rather than replaced, so a
///   stray `NUL` or `ESC` cannot widen or split the name;
/// - the separator character is removed, because a label containing it would
///   stop the title parsing;
/// - the result is capped at [`MAX_CONTEXT_LABEL_CHARS`] characters, counted
///   as characters so a multi-byte name is never cut mid-character.
///
/// Returns an empty string when nothing presentable survives (a name made
/// only of controls, spaces and separators). [`ChatDescriptor::capture`]
/// refuses an empty label, so callers should route raw names through
/// [`context_label_for`], which supplies [`WORKSPACE_LABEL`] in that case.
pub fn sanitize_context_label(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = 0usize;
    let mut pending_space = false;
    for c in raw.chars() {
        if chars >= MAX_CONTEXT_LABEL_CHARS {
            break;
        }
        // Whitespace first: `\n`, `\t`, `\r` are *both* whitespace and
        // control, and reading them as whitespace keeps "line1\nline2"
        // legible as "line1 line2" instead of welding it into "line1line2".
        if c.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if c.is_control() || c == TITLE_SEPARATOR_CHAR {
            continue;
        }
        if pending_space {
            // The space costs one of the budgeted characters; if it would be
            // the last one, stop instead of ending on a trailing space.
            if chars + 1 >= MAX_CONTEXT_LABEL_CHARS {
                break;
            }
            out.push(' ');
            chars += 1;
            pending_space = false;
        }
        out.push(c);
        chars += 1;
    }
    out
}

/// The Context_Label for a chat: the sanitised project name when there is a
/// presentable one, else [`WORKSPACE_LABEL`] (16.4).
///
/// One place decides this, so the title, the header and the state artifact
/// cannot disagree about how a chat is scoped.
pub fn context_label_for(project_name: Option<&str>) -> String {
    match project_name.map(sanitize_context_label) {
        Some(label) if !label.is_empty() => label,
        _ => WORKSPACE_LABEL.to_string(),
    }
}

// --- OSC window-title emission -------------------------------------------
//
// The chat titles *itself* with OSC 2. No `--title` or `--app-id` launch flag
// is passed: those are not portably honoured across the terminals Omarchy
// offers, naming an emulator would break the terminal-agnostic constraint
// (28.4), and a harness or shell would overwrite an externally set title
// anyway. Emitting from inside the process also means the title can be
// re-asserted after every answer, so nothing can quietly take the identity
// away (design §4.4).
//
// The sequence builder is pure and the writer is generic, so both are
// testable without a terminal; only `assert_window_title` touches stdout.

/// Build `ESC ] 2 ; <title> BEL`.
///
/// `None` when `title` carries a control character: such a title could
/// terminate the sequence early or start another one, and no title
/// [`format_title`] produces can contain one — this is a guard against a
/// title that did not come from there.
pub fn window_title_sequence(title: &str) -> Option<String> {
    if title.is_empty() || title.chars().any(char::is_control) {
        return None;
    }
    Some(format!("\u{1b}]2;{title}\u{07}"))
}

/// Write the OSC 2 sequence for `title` to `out`, flushing it.
///
/// Writes nothing at all when [`window_title_sequence`] refuses the title.
/// Generic over the sink so tests can assert the exact bytes.
pub fn write_window_title<W: std::io::Write>(out: &mut W, title: &str) -> std::io::Result<()> {
    match window_title_sequence(title) {
        Some(seq) => {
            out.write_all(seq.as_bytes())?;
            out.flush()
        }
        None => Ok(()),
    }
}

/// Assert this chat's window identity on the terminal: call once at startup
/// and again after each answer (16.2, design §4.4).
///
/// A no-op when stdout is not a TTY — piped output must not collect escape
/// bytes — and best-effort otherwise: a chat that cannot write its title is
/// still a working chat, so a write error is not surfaced as a failure. The
/// argument is a [`ChatDescriptor`] rather than a string so the only title
/// that can ever be emitted is the one the grammar produces.
pub fn assert_window_title(d: &ChatDescriptor) {
    use std::io::IsTerminal;
    let mut out = std::io::stdout();
    if !out.is_terminal() {
        return;
    }
    let _ = write_window_title(&mut out, &format_title(d));
}

// ---------------------------------------------------------------------------
// Chat_Identity_Allocator and the O_EXCL lease — task 8.3 (design §4.3)
//
// Number derivation from observed chat windows plus live leases, atomic
// claim, stale-lease reclamation, and release on `/exit` and by `Drop`.
// Numbers are never persisted as a counter.
//
// **There is no counter, and there is nowhere one could hide** (10.4, design
// §9.4). Nothing in this section reads or writes a stored number: a chat
// number is *available* when nothing observable holds it, and the two
// observations are
//
//   (a) the chat-classified windows of the current `collector::collect`
//       (their numbers arrive as `observed_numbers`), and
//   (b) the `chat-NNN.lease` files in the runtime directory whose recorded
//       pid is a live `pitwall chat` process.
//
// The lease file exists for exactly one reason a pure computation cannot
// cover: two chats starting inside one observation gap would otherwise both
// compute the same lowest-available number. `O_EXCL` makes the claim atomic,
// so the kernel — not a guess — decides which of them wins. The lease
// remembers nothing (its whole content is its owner's pid), cannot drift,
// and lives on tmpfs, so it is a mutual-exclusion token and never a counter:
// delete every lease while no chat runs and the next chat still allocates
// `001`.
//
// The lease is also the corroborating half of window identity (design §4.8):
// the collector takes the number out of a window title and requires
// `chat-NNN.lease` to name a pid inside that window's process tree. That is
// why the body is the owner pid and why the number in the file name must be
// the number in the title — a chat window can be *claimed* by printing text,
// but not *proved* without the lease.
// ---------------------------------------------------------------------------

/// Lowest chat number (10.1, 10.2).
pub const MIN_CHAT_NUMBER: u16 = 1;

/// Highest chat number (10.1, 10.2, 10.5).
pub const MAX_CHAT_NUMBER: u16 = 999;

/// Lease file name fence, byte-identical to the one the platform's lease
/// reader parses (`src/platform/linux.rs`). The two must agree exactly: the
/// name this module writes is the name that reader recognises and the
/// collector looks a window title's number up under (17.2).
const LEASE_PREFIX: &str = "chat-";
const LEASE_SUFFIX: &str = ".lease";

/// A lease body is one decimal pid. Cap the read so a planted large file
/// cannot be pulled into memory (same bound as the platform reader).
const LEASE_MAX_BYTES: u64 = 32;

/// The lease file name for a chat number: `chat-NNN.lease`, three digits,
/// matching [`ChatDescriptor::number_text`] and the digits in the window
/// title.
pub fn lease_file_name(number: u16) -> String {
    format!("{LEASE_PREFIX}{number:03}{LEASE_SUFFIX}")
}

/// Owner pid recorded in a lease file, or `None` when it cannot be read as
/// exactly one plain decimal pid.
///
/// `None` is a deliberate dead end for reclamation: a lease whose owner
/// cannot be determined is *never* unlinked. Mirrors the platform reader's
/// strictness (bounded read, trailing newline allowed, pid `0` rejected) so
/// the two cannot disagree about what a lease says.
fn read_lease_pid(path: &std::path::Path) -> Option<u32> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(LEASE_MAX_BYTES).read_to_end(&mut buf).ok()?;
    let body = String::from_utf8(buf).ok()?;
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

/// Does an existing lease count as holding its number?
///
/// Yes unless it is *demonstrably* dead. Deliberately conservative in the
/// same way [`crate::context::sweep_orphans_in`] is:
///
/// - an empty `processes` slice means observation failed, not that nothing
///   is running, so nothing is reclaimed and every lease counts as held;
/// - otherwise the question is delegated to
///   [`crate::context::is_live_pitwall_chat`] — the one liveness check in
///   the tree — which requires the pid to be present *and* to be a
///   `pitwall chat` invocation.
///
/// Erring this way costs at most a higher number for one chat; erring the
/// other way would hand a running chat's number to a second chat, breaking
/// the distinctness invariant of 21.2.
fn lease_counts_as_held(pid: u32, processes: &[crate::platform::RawProcess]) -> bool {
    if processes.is_empty() {
        return true;
    }
    crate::context::is_live_pitwall_chat(pid, processes)
}

/// Create the runtime directory and secure it `0700`, exactly as
/// [`crate::context::EphemeralContext`] does before writing there.
fn secure_runtime_dir(dir: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create runtime dir: {e}"))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("cannot secure runtime dir: {e}"))
}

/// An owned chat number: the number plus the `chat-NNN.lease` file that
/// proves this process holds it.
///
/// Release is guaranteed by construction, following
/// [`crate::context::EphemeralContext`] exactly: the path lives in an
/// `Option`, [`ChatNumberLease::release`] `take`s it (so a second call —
/// including the one `Drop` makes after an explicit release at `/exit` —
/// finds `None` and unlinks nothing), and `Drop` is the net for every other
/// exit path. Holding the path rather than reconstructing it also means the
/// guard can only ever remove the file it created.
///
/// Not `Clone` and not `Copy` on purpose: two values must never claim the
/// authority to unlink one lease.
pub struct ChatNumberLease {
    number: u16,
    /// `Some` while the lease file is ours to remove; `None` afterwards.
    path: Option<std::path::PathBuf>,
}

impl ChatNumberLease {
    /// The held chat number, `1..=999`.
    pub fn number(&self) -> u16 {
        self.number
    }

    /// The three-digit form, identical to the digits in the lease file name,
    /// the window title, and [`ChatDescriptor::number_text`].
    pub fn number_text(&self) -> String {
        format!("{:03}", self.number)
    }

    /// The lease file, while it is still held.
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    /// Release the number explicitly (called at `/exit`). Idempotent, and
    /// best-effort: a lease that cannot be unlinked is reclaimed by the next
    /// allocator that sees its pid is gone, and by tmpfs at logout.
    pub fn release(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for ChatNumberLease {
    fn drop(&mut self) {
        self.release();
    }
}

/// Try to claim `number` atomically.
///
/// `Ok(Some(lease))` — created; `Ok(None)` — `EEXIST`, somebody else holds
/// the name (or a stale file does); `Err` — the directory refused the
/// create. The guard is constructed the instant the file exists, before the
/// first fallible step, so a failed write removes the file it just made.
fn claim_lease(dir: &std::path::Path, number: u16) -> Result<Option<ChatNumberLease>, String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;
    let path = dir.join(lease_file_name(number));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true).mode(0o600);
    match opts.open(&path) {
        Ok(mut file) => {
            let owned = ChatNumberLease {
                number,
                path: Some(path),
            };
            file.write_all(format!("{}\n", std::process::id()).as_bytes())
                .map_err(|e| format!("cannot write chat lease: {e}"))?;
            Ok(Some(owned))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(format!("cannot create chat lease: {e}")),
    }
}

/// Allocate the lowest available chat number and claim it.
///
/// Production entry point: leases live under
/// [`crate::context::ephemeral_dir`]. See [`allocate_chat_number_in`] for
/// the algorithm; the arguments are the caller's already-collected
/// observation, so this function performs no `/proc` reading and no
/// compositor query of its own.
///
/// - `observed_numbers` — numbers held by chat-classified windows in the
///   current observation (`WindowRole::Chat`, task 11.1). Passing an empty
///   slice is legitimate — the leases alone then decide — but passing what
///   was observed is what makes 10.2 hold across an observation gap.
/// - `leases` — [`crate::platform::Platform::chat_leases`] output.
/// - `processes` — [`crate::platform::Platform::processes`] output.
///
/// Note what is *not* a parameter: a number. There is no way for a human to
/// ask for one, and no CLI option feeds one in (10.6).
pub fn allocate_chat_number(
    observed_numbers: &[u16],
    leases: &[crate::platform::ChatLease],
    processes: &[crate::platform::RawProcess],
) -> Result<ChatNumberLease, String> {
    allocate_chat_number_in(
        &crate::context::ephemeral_dir(),
        observed_numbers,
        leases,
        processes,
    )
}

/// Same, under an explicit directory (test seam, mirroring
/// [`crate::context::EphemeralContext::create_in`] and
/// [`crate::context::sweep_orphans_in`], so a test never touches the real
/// runtime directory).
///
/// The algorithm, in the order design §4.3 states it:
///
/// 1. **in use** = the observed chat windows' numbers, plus every lease that
///    [`lease_counts_as_held`] cannot disprove;
/// 2. **stale** = every lease that it *can* disprove — unlinked immediately,
///    which is also what makes 10.3 fall out: a number a finished chat left
///    behind is simply available again, with no released-number bookkeeping
///    anywhere;
/// 3. **candidate** = the lowest number in `1..=999` that is not in use
///    (10.2);
/// 4. **claim** = create `chat-NNN.lease` `O_EXCL` mode `0600` containing
///    this process's pid;
/// 5. **`EEXIST`** = a lease appeared inside the observation gap. Re-validate
///    *that file*: still held ⇒ advance to the next candidate; demonstrably
///    dead ⇒ unlink and retry the same number once; unreadable owner ⇒ leave
///    it alone and advance.
///
/// **Race resolution.** Two chats that start inside one observation gap
/// compute the same candidate and both attempt step 4; exactly one `O_EXCL`
/// create succeeds. The loser takes the `EEXIST` branch, finds the winner's
/// pid live, and advances. Which chat gets `001` is decided by kernel-atomic
/// creation order rather than by a guess; the invariant that matters — the
/// set of live numbers is always distinct (21.2) — holds either way.
///
/// **Why step 2 cannot cut a live chat's legs off.** Steps 1 and 2 look at
/// the same lease from two sides, so a number could in principle be held by
/// an observed window while its lease is unlinked as stale — which would
/// strip that window of the corroborating half of its identity (§4.8). It
/// cannot happen: a window is chat-classified *because* its number's lease
/// names a pid in its process tree, so if the window is live its lease pid is
/// a live `pitwall chat` and step 2 leaves the file alone. The only way step
/// 2 disproves it is that the chat process died since the observation — and
/// then the window died with it. Either way the number stays in `in_use` for
/// this allocation, because step 1 already put it there.
///
/// Returns `Err` when all 999 numbers are in use. Classifying that as
/// process exit code 1 (10.5) belongs to the CLI (task 12.1); the allocator
/// only describes what happened.
pub fn allocate_chat_number_in(
    dir: &std::path::Path,
    observed_numbers: &[u16],
    leases: &[crate::platform::ChatLease],
    processes: &[crate::platform::RawProcess],
) -> Result<ChatNumberLease, String> {
    secure_runtime_dir(dir)?;

    let valid = MIN_CHAT_NUMBER..=MAX_CHAT_NUMBER;
    let mut in_use: std::collections::BTreeSet<u16> = observed_numbers
        .iter()
        .copied()
        .filter(|n| valid.contains(n))
        .collect();

    for lease in leases {
        if !valid.contains(&lease.number) {
            continue;
        }
        if lease_counts_as_held(lease.pid, processes) {
            in_use.insert(lease.number);
        } else {
            // Demonstrably not a live `pitwall chat`: reclaim now, so the
            // number is free for this very allocation (10.3).
            let _ = std::fs::remove_file(dir.join(lease_file_name(lease.number)));
        }
    }

    for number in MIN_CHAT_NUMBER..=MAX_CHAT_NUMBER {
        if in_use.contains(&number) {
            continue;
        }
        if let Some(lease) = claim_lease(dir, number)? {
            return Ok(lease);
        }
        // EEXIST: a lease exists that the observation did not carry.
        let path = dir.join(lease_file_name(number));
        let held = match read_lease_pid(&path) {
            Some(pid) => lease_counts_as_held(pid, processes),
            // Unreadable or malformed body: no owner to disprove, so this
            // file is never unlinked here.
            None => true,
        };
        if held || std::fs::remove_file(&path).is_err() {
            continue;
        }
        // Reclaimed a stale lease: retry this number exactly once. A second
        // EEXIST means another chat claimed it in between — advance.
        if let Some(lease) = claim_lease(dir, number)? {
            return Ok(lease);
        }
    }

    Err(format!(
        "all chat numbers {MIN_CHAT_NUMBER:03}..{MAX_CHAT_NUMBER:03} are in use (refusing)"
    ))
}

// ---------------------------------------------------------------------------
// Closed-vocabulary input classifier — task 9.1 (design §4.6, §5.4)
//
// This is the safety boundary of the whole chat surface. It is the single
// place that decides whether an input line *can* act, so it is modelled as a
// **total function**: [`classify`] takes a `&str` and returns exactly one
// [`ChatInput`] variant for every possible input. There is no fallthrough, no
// `Option`, no "probably a command", and no second opinion later in the loop.
//
// Three structural decisions carry the requirements:
//
// 1. **Acting is one variant.** Every workspace-changing input is
//    [`ChatInput::Act`], wrapping [`ActionCommand`]. A question is
//    [`ChatInput::Question`] and *cannot* be an `Act`, so "asking" is
//    incapable of acting by construction rather than by discipline (14.5,
//    14.10, 26.3, 26.4, 27.1, 27.2). Adding a future acting command means
//    adding an `ActionCommand` variant, which every `match` on `Act` sees;
//    it cannot sneak in beside `Question`.
// 2. **The vocabulary is closed and is data.** [`VOCABULARY`] lists the six
//    entries with their help text and their [`Effect`], and it is the *only*
//    source of the read-only vs acting marks that [`render_vocabulary`]
//    prints (27.8) and that [`ChatInput::effect`] reports. Matching and
//    listing therefore cannot drift apart.
// 3. **Nothing input-derived becomes a command line.** A classified input
//    carries at most two bounded strings — the sanitised question text and
//    the raw resume argument — and neither is ever a binary, a flag, or a
//    shell word. Each vocabulary entry is executed by *dispatching on the
//    enum* to a fixed argument vector built by later tasks (9.2 for the
//    harness, 9.4 for `resume::resume`), so there is no caller-composed
//    command line anywhere on this path and no shell to interpret one
//    (14.1, 14.9).
//
// Matching is exact on the command word and tolerant only of surrounding
// whitespace. It is deliberately **not** case-insensitive and does **not**
// prefix-match: `/Resume`, `/resumes` and `/res` are unknown commands, not
// near-misses that act. A loose match is a way to change the workspace by
// accident.
//
// The whole section is pure — no I/O, no `Platform`, no database, no clock —
// which is what makes the zero-action property (Property 19) mechanically
// testable.
// ---------------------------------------------------------------------------

/// Maximum characters of an unknown command word echoed back in the refusal
/// line. The word came from a human, so it is bounded before it is printed;
/// the refusal never needs more than a recognisable stub.
const MAX_UNKNOWN_COMMAND_CHARS: usize = 32;

/// Maximum characters of the resume argument carried out of classification.
///
/// Presentation hygiene, **not** validation: task 9.4 shape-checks the target
/// with [`crate::resume::is_session_id`], which accepts exactly 21 characters
/// (`sess_` + 16 hex). The cap sits far above that, so trimming here can
/// never turn a target the shape check would refuse into one it accepts — it
/// only bounds what a refusal line may have to print.
const MAX_RESUME_TARGET_CHARS: usize = 64;

/// Whether a vocabulary entry can change workspace state (27.8).
///
/// Two values, not three: `/exit` ends the chat but changes nothing in the
/// workspace, so it is read-only like the informational entries. Exactly one
/// entry in [`VOCABULARY`] is [`Effect::ChangesWorkspaceState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Displays information the chat already holds; invokes nothing.
    ReadOnly,
    /// May change workspace state. Only `/resume` is this (26.1, 26.2).
    ChangesWorkspaceState,
}

impl Effect {
    /// Does this effect change workspace state?
    pub fn changes_workspace_state(self) -> bool {
        matches!(self, Effect::ChangesWorkspaceState)
    }

    /// The mark [`render_vocabulary`] prints beside an entry (27.8).
    pub fn marker(self) -> &'static str {
        match self {
            Effect::ReadOnly => "read-only",
            Effect::ChangesWorkspaceState => "CHANGES WORKSPACE STATE",
        }
    }
}

/// One entry of the closed In_Chat_Command_Vocabulary (14.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VocabularyEntry {
    /// The command word, matched exactly and case-sensitively.
    pub name: &'static str,
    /// The argument form, when the entry takes one. `/resume` is the only
    /// entry that does, and its argument is optional.
    pub argument: Option<&'static str>,
    /// One line of help.
    pub help: &'static str,
    /// Read-only, or changing workspace state (27.8).
    pub effect: Effect,
}

impl VocabularyEntry {
    /// The form a human types, e.g. `/resume [session-id]`.
    pub fn usage(&self) -> String {
        match self.argument {
            Some(arg) => format!("{} {arg}", self.name),
            None => self.name.to_string(),
        }
    }
}

/// The closed vocabulary: four informational entries, one acting entry, one
/// that ends the chat (14.7, design §5.4).
///
/// Anything else beginning with `/` is an unknown command: the listing is
/// printed and nothing is invoked (14.8).
pub const VOCABULARY: &[VocabularyEntry] = &[
    VocabularyEntry {
        name: "/help",
        argument: None,
        help: "list these commands",
        effect: Effect::ReadOnly,
    },
    VocabularyEntry {
        name: "/context",
        argument: None,
        help: "show the bounded context this chat can see",
        effect: Effect::ReadOnly,
    },
    VocabularyEntry {
        name: "/sessions",
        argument: None,
        help: "show the observed sessions in that context",
        effect: Effect::ReadOnly,
    },
    VocabularyEntry {
        name: "/clear",
        argument: None,
        help: "clear the conversation shown here (nothing was stored)",
        effect: Effect::ReadOnly,
    },
    VocabularyEntry {
        name: "/resume",
        argument: Some("[session-id]"),
        help: "resume a session: focus its window, or open one terminal",
        effect: Effect::ChangesWorkspaceState,
    },
    VocabularyEntry {
        name: "/exit",
        argument: None,
        help: "end this chat",
        effect: Effect::ReadOnly,
    },
];

/// The informational entries: they print what the chat already holds and
/// invoke no harness and no action (14.7, design §5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoCommand {
    /// `/help`
    Help,
    /// `/context`
    Context,
    /// `/sessions`
    Sessions,
    /// `/clear`
    Clear,
}

impl InfoCommand {
    /// The command word, byte-identical to its [`VOCABULARY`] name.
    pub fn name(self) -> &'static str {
        match self {
            InfoCommand::Help => "/help",
            InfoCommand::Context => "/context",
            InfoCommand::Sessions => "/sessions",
            InfoCommand::Clear => "/clear",
        }
    }
}

/// A command that may change workspace state.
///
/// Deliberately its own type with its own variant in [`ChatInput`]: "can this
/// input act?" is answered by one pattern match on [`ChatInput::Act`], and a
/// later acting command has to be added *here*, where every existing match
/// arm on `Act` already covers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionCommand {
    /// `/resume [session-id]` — the only workspace-changing input in M8
    /// (26.1, 26.2).
    ///
    /// `target` is the argument **exactly as entered**, control-stripped and
    /// bounded for safe printing but otherwise untouched. It is *not*
    /// validated here: resolving `None` against the descriptor's context
    /// session id (26.7), shape-checking with
    /// [`crate::resume::is_session_id`] (26.11) and executing through
    /// [`crate::resume::resume`] (26.10) all belong to the Resume_Action
    /// bridge (task 9.4). Carrying the raw argument keeps this function pure
    /// and keeps every refusal message in one place.
    Resume { target: Option<String> },
}

/// A chat question that has passed every input rule (14.2, 15.5).
///
/// Only [`classify`] constructs one, and the text field is private, so a
/// `ChatQuestion` existing *is* the proof that its text was trimmed,
/// control-stripped via [`crate::context::strip_controls`], scrubbed via
/// [`crate::context::scrub_string`], non-empty, and at most
/// [`MAX_CHAT_QUESTION_CHARS`] characters. Task 9.2 places exactly this
/// string in exactly one argv element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatQuestion {
    text: String,
}

impl ChatQuestion {
    /// The sanitised question text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Its length in characters, always `1..=MAX_CHAT_QUESTION_CHARS`.
    pub fn chars(&self) -> usize {
        self.text.chars().count()
    }
}

/// The classification of one input line. Total: every possible input maps to
/// exactly one variant (Property 19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatInput {
    /// Empty, whitespace-only, or nothing but control characters: neither a
    /// command nor a question. The loop skips it and re-prompts.
    Blank,
    /// An informational entry: read-only, no harness, no action.
    Info(InfoCommand),
    /// The one class of input that may change workspace state.
    Act(ActionCommand),
    /// `/exit`: end the chat with code 0.
    End,
    /// A `/`-prefixed input outside the vocabulary. Prints the listing and
    /// invokes nothing (14.8). `entered` is the control-stripped, bounded
    /// command word, kept only so the refusal can name it.
    Unknown { entered: String },
    /// Anything not beginning with `/` (27.5): one harness answer over the
    /// bounded context, no action.
    Question(ChatQuestion),
    /// A question over [`MAX_CHAT_QUESTION_CHARS`] characters.
    ///
    /// Still the Chat_Question class — it is not in the vocabulary, it
    /// invokes no action, and it changes nothing — but it is **refused rather
    /// than truncated**, following the precedent
    /// [`crate::assign::MAX_ASSIGN_PROMPT_CHARS`] sets: cutting a question
    /// changes what was asked, and an answer to a silently shortened
    /// question is worse than a request to trim and retry.
    QuestionTooLong { chars: usize },
}

impl ChatInput {
    /// The [`VOCABULARY`] entry this input names, when it names one.
    ///
    /// `None` for [`ChatInput::Blank`], [`ChatInput::Unknown`] and both
    /// question variants: they are not vocabulary entries.
    pub fn vocabulary_entry(&self) -> Option<&'static VocabularyEntry> {
        let name = match self {
            ChatInput::Info(cmd) => cmd.name(),
            ChatInput::Act(ActionCommand::Resume { .. }) => "/resume",
            ChatInput::End => "/exit",
            ChatInput::Blank
            | ChatInput::Unknown { .. }
            | ChatInput::Question(_)
            | ChatInput::QuestionTooLong { .. } => return None,
        };
        VOCABULARY.iter().find(|entry| entry.name == name)
    }

    /// The effect the listing marks this input with (27.8), taken from
    /// [`VOCABULARY`] so the mark and the behaviour cannot disagree.
    pub fn effect(&self) -> Option<Effect> {
        self.vocabulary_entry().map(|entry| entry.effect)
    }

    /// Can this input change workspace state?
    ///
    /// True for [`ChatInput::Act`] and for nothing else — the read-only vs
    /// acting distinction is a property of the variant, not of a flag that
    /// could be set wrongly (14.10, 26.3, 27.2).
    pub fn changes_workspace_state(&self) -> bool {
        matches!(self, ChatInput::Act(_))
    }

    /// Does this input invoke the harness?
    ///
    /// True only for an accepted question: every vocabulary entry, every
    /// unknown command, every blank line and every refused question invokes
    /// nothing (14.8, Property 14).
    pub fn invokes_harness(&self) -> bool {
        matches!(self, ChatInput::Question(_))
    }
}

/// Classify one input line. Total, pure, and the only decision point for
/// whether an input can act (14.7, 14.8, 26.1, 27.5).
///
/// Order, and why:
///
/// 1. trim surrounding whitespace; empty ⇒ [`ChatInput::Blank`];
/// 2. the prefix test runs on the **trimmed raw** line, before any control
///    stripping. `"\u{1b}/exit"` therefore does not begin with `/` and is a
///    question, not the end command: stripping first would let a line
///    carrying an escape sequence turn into a command;
/// 3. no `/` prefix ⇒ the question path (27.5);
/// 4. a `/` prefix ⇒ split on whitespace and match the first word
///    **exactly**, then require the argument count the entry allows. `/help
///    now`, `/resume a b` and `/exit please` are unknown commands rather than
///    guesses about what was meant;
/// 5. no match ⇒ [`ChatInput::Unknown`], which prints the listing and invokes
///    nothing (14.8).
pub fn classify(line: &str) -> ChatInput {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return ChatInput::Blank;
    }
    if !trimmed.starts_with('/') {
        return classify_question(trimmed);
    }

    let mut words = trimmed.split_whitespace();
    // `trimmed` is non-empty and starts with `/`, so a first word exists.
    let word = words.next().unwrap_or_default();
    let args: Vec<&str> = words.collect();

    // The whole closed vocabulary, in one exhaustive expression. Every arm is
    // an exact, case-sensitive word plus an exact argument count; the catch-all
    // is the only other outcome, and it acts on nothing.
    match (word, args.as_slice()) {
        ("/help", []) => ChatInput::Info(InfoCommand::Help),
        ("/context", []) => ChatInput::Info(InfoCommand::Context),
        ("/sessions", []) => ChatInput::Info(InfoCommand::Sessions),
        ("/clear", []) => ChatInput::Info(InfoCommand::Clear),
        ("/exit", []) => ChatInput::End,
        ("/resume", []) => ChatInput::Act(ActionCommand::Resume { target: None }),
        ("/resume", [target]) => ChatInput::Act(ActionCommand::Resume {
            target: Some(resume_target_text(target)),
        }),
        _ => ChatInput::Unknown { entered: unknown_command_text(word) },
    }
}

/// The question path: sanitise, bound, refuse over-cap.
///
/// Sanitisation is entirely delegated — [`crate::context::strip_controls`]
/// then [`crate::context::scrub_string`], the same two functions
/// [`crate::assign::prepare`] applies to a human prompt, in the same order.
/// No new sanitiser exists in this module.
///
/// The cap is checked twice, on the control-stripped text and again on the
/// scrubbed text, because redaction *replaces* a secret with `[redacted]` and
/// can therefore lengthen the string. What travels into argv is the scrubbed
/// text, so the scrubbed text is what must fit (14.2).
fn classify_question(trimmed: &str) -> ChatInput {
    // `strip_controls` keeps `\n` and `\t` (they are content, not escapes);
    // trimming again removes whitespace that stripping exposed at the ends.
    let clean = crate::context::strip_controls(trimmed).trim().to_string();
    if clean.is_empty() {
        // Nothing but control characters: not a question, and asking the
        // harness about it would be asking about nothing.
        return ChatInput::Blank;
    }
    let clean_chars = clean.chars().count();
    if clean_chars > MAX_CHAT_QUESTION_CHARS {
        return ChatInput::QuestionTooLong { chars: clean_chars };
    }
    let scrubbed = crate::context::scrub_string(&clean);
    let scrubbed_chars = scrubbed.chars().count();
    if scrubbed_chars > MAX_CHAT_QUESTION_CHARS {
        return ChatInput::QuestionTooLong { chars: scrubbed_chars };
    }
    ChatInput::Question(ChatQuestion { text: scrubbed })
}

/// Bound an unknown command word for printing: controls stripped, capped at
/// [`MAX_UNKNOWN_COMMAND_CHARS`] characters.
fn unknown_command_text(word: &str) -> String {
    let clean = crate::context::strip_controls(word);
    truncate_chars(&clean, MAX_UNKNOWN_COMMAND_CHARS)
}

/// Bound a resume argument for carrying and printing. See
/// [`MAX_RESUME_TARGET_CHARS`]: hygiene, never validation.
fn resume_target_text(target: &str) -> String {
    let clean = crate::context::strip_controls(target);
    truncate_chars(&clean, MAX_RESUME_TARGET_CHARS)
}

/// Render the closed vocabulary, marking each entry read-only or as changing
/// workspace state (27.8, 14.8).
///
/// Pure: returns the text, prints nothing. The marks come from
/// [`VOCABULARY`], so this listing always describes the behaviour
/// [`classify`] actually produces.
pub fn render_vocabulary() -> String {
    let usage: Vec<String> = VOCABULARY.iter().map(VocabularyEntry::usage).collect();
    let usage_width = usage.iter().map(|u| u.chars().count()).max().unwrap_or(0);
    let mark_width = VOCABULARY
        .iter()
        .map(|e| e.effect.marker().chars().count())
        .max()
        .unwrap_or(0);

    let mut out = String::from("In-chat commands (this list is the whole vocabulary):\n");
    for (entry, use_text) in VOCABULARY.iter().zip(usage.iter()) {
        out.push_str(&format!(
            "  {:<uw$}  {:<mw$}  {}\n",
            use_text,
            entry.effect.marker(),
            entry.help,
            uw = usage_width,
            mw = mark_width
        ));
    }
    out.push_str(
        "Anything that does not start with '/' is a question: it is answered \
         from the observed context and changes nothing.\n",
    );
    out
}

/// The refusal for an unknown `/` command: name it, say nothing ran, print
/// the vocabulary (14.8).
///
/// `entered` should be the [`ChatInput::Unknown`] payload, which is already
/// control-stripped and bounded.
pub fn unknown_command_message(entered: &str) -> String {
    format!(
        "'{entered}' is not an in-chat command; nothing was run.\n{}",
        render_vocabulary()
    )
}

// ---------------------------------------------------------------------------
// Chat_Responder — task 9.2 (design §4.6)
//
// Fresh bounded `SummaryContext` per question, one fixed-argv harness run
// with `CHAT_TIMEOUT_SECS`, text-only extraction. Takes no `Platform`, no
// database path and no resume handle, so the question path structurally
// cannot reach an action.
//
// It must call `assert_window_title` after printing each answer, and the
// startup path (task 12.1) must call it once before the header, so a harness
// that rewrites the title cannot take the chat's identity with it (16.2).
//
// **The observe/respond split, and why it exists.** Answering a question needs
// two capabilities that must not sit in the same function:
//
//   * *observing* — `collector::collect`, the degradable continuity-store
//     reads, and rendering the bounded document — needs a `&dyn Platform`
//     (which can focus windows and launch terminals) and a data directory;
//   * *answering* — staging the `0600` context file, building the fixed argv,
//     running the harness once, and extracting text — needs neither.
//
// So observing happens in [`observe`], which takes the `Platform` and returns
// an inert [`ChatObservation`] (a bounded `SummaryContext` plus the already
// rendered document — data, no capability), and answering happens in
// [`respond`], whose parameters are `(&ChatDescriptor, &ChatObservation,
// &ChatQuestion)`. **`respond` receives no `&dyn Platform`, no database path
// and no resume handle**, so the question path cannot reach an action even if
// the harness answers "I will resume it": there is nothing in scope to act
// with (14.5, 27.1, 27.2, 27.3, 27.10). The only caller of the resume bridge
// is the `/resume` arm of [`classify`], which a question can never produce.
//
// (Design §4.6 writes the second parameter as `&SummaryContext`. It is
// `&ChatObservation` here for one mechanical reason: rendering the document
// from a `SummaryContext` goes through `context::build_context_from_summary`,
// which needs the `Platform` to sample bounded terminal text and IO counters.
// Keeping the render on the observing side is what lets `respond` stay
// capability-free; the observation carries the same facts plus the rendered
// form of them, and still carries no way to act.)
// ---------------------------------------------------------------------------

/// The fixed chat instruction, the exact analogue of
/// [`crate::summary::INSTRUCTION`]: one immutable string, no interpolation,
/// no caller-supplied fragment, so what the harness is told cannot vary with
/// the question.
///
/// It states, in order, the four rules Requirement 13 sets — answer only from
/// the attached bounded context (13.1), say the information is unavailable
/// when the context lacks it (13.2), present process and IO activity as
/// activity evidence rather than task progress (13.3), and report agent
/// identity with the recorded confidence (13.4) — and closes with the
/// observe-only boundary (14.5, 27.4). The final sentence is deliberate: a
/// question that *asks* for an action is answered by naming the command the
/// human would have to enter, which is the only way wording can lead to an
/// action, and it leads to a human keystroke rather than to an executor
/// (27.3).
pub const CHAT_INSTRUCTION: &str = "Act as Pitwall's race engineer answering one question about the human's workspace. Answer only from the attached, bounded Pitwall workspace context: it is the whole of what you may use. When that context does not carry the evidence the question needs, say plainly that the information is unavailable, and never close the gap with a guess, an assumption, or general knowledge. Treat process activity and input-output counters as activity evidence only, never as proof of task progress or completion: say that a session is active, not that its work advanced. Name an agent only with the confidence the context records for it, and say the agent is unconfirmed when that confidence is low. Prefer plain language such as 'OpenCode is idle in Work' or 'One session stopped; another remains active'. Do not mention internal diagnostics such as process counts, confidence mechanics, missing scrollback, or file paths unless they are themselves the answer. Do not invent work, completion, blockers, or attention items. Answer in 1-4 short sentences. You are observing only: do not execute tasks, do not modify files, do not start, resume, or delegate anything, and do not continue the work. If the question asks for an action, say which in-chat command the human would enter to perform it, and take no action yourself.";

/// Derived-event cap for a chat context, the same 20 `cmd_summarize` passes
/// to [`crate::context::derive_events`] (12.2).
pub const MAX_CHAT_EVENTS: usize = 20;

/// Checkpoint cap for a chat context, the same newest-10 `cmd_summarize`
/// assembles (12.2).
pub const MAX_CHAT_CHECKPOINTS: usize = 10;

/// Unread-notification cap for a chat context, the same 20 the state
/// artifact's inbox view uses.
pub const MAX_CHAT_NOTIFICATIONS: usize = 20;

/// How the bounded context document reaches one harness.
///
/// Both variants keep the document out of argv, which is the point: argv is
/// world-readable through `/proc`, while the `0600` file and a pipe are not
/// (14.4, 15.1-15.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextDelivery {
    /// A path in argv, content in the `0600` file (`opencode -f CTX`).
    File,
    /// Content on the child's stdin, path nowhere (`claude -p`, `codex exec`).
    Stdin,
}

/// The verified-delivery table (design §4.6, §9.3).
///
/// One row per harness, and the row *is* the permission to run it: a harness
/// absent from this table has no channel that keeps the document out of a
/// command line, so [`delivery_for`] refuses it rather than degrading privacy
/// (see [`ChatError::DeliveryUnverified`]). Adding a harness means adding a
/// row here after its private channel has actually been observed — never by
/// widening argv.
const DELIVERY: &[(&str, ContextDelivery)] = &[
    // `opencode run -f CTX` — the surface `summary::build_argv` already uses.
    ("opencode", ContextDelivery::File),
    // `claude -p MSG` with the document on stdin (gate 1.2).
    ("claude", ContextDelivery::Stdin),
    // `codex exec MSG` with the document on stdin. Documented surface; if it
    // turns out not to consume stdin alongside a prompt argument, the fix is
    // to delete this row — chat then refuses `codex` with
    // [`ChatError::DeliveryUnverified`] — and never to move the document into
    // argv (§9.3).
    ("codex", ContextDelivery::Stdin),
];

/// The recorded private delivery channel for a harness, or `None` when there
/// is none.
pub fn delivery_for(harness: &str) -> Option<ContextDelivery> {
    DELIVERY
        .iter()
        .find(|(id, _)| *id == harness)
        .map(|(_, delivery)| *delivery)
}

/// Why one question could not be answered.
///
/// Every variant renders to a **fixed, short line** through
/// [`ChatError::class`] and [`ChatError::message`]. Harness internals, the
/// argument vector, the ephemeral path and the runner's own error text never
/// reach the human: the only variable parts are the harness id (a
/// [`crate::agents::KNOWN`] id from the descriptor) and the timeout the human
/// can already read off the clock. Every variant leaves the chat ready for the
/// next input (13.5, 13.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatError {
    /// This harness has no recorded private channel for the bounded context,
    /// so it is refused (design §9.3). Never a reason to put the document in
    /// argv.
    DeliveryUnverified { harness: String },
    /// The configured harness is not installed here.
    HarnessNotInstalled { harness: String },
    /// The bounded context could not be staged as an owner-only file.
    /// Nothing was invoked.
    ContextUnavailable,
    /// The harness could not be started (spawn failure).
    HarnessUnavailable,
    /// The harness ran and failed, or its context could not be delivered.
    HarnessFailed,
    /// The harness exceeded [`CHAT_TIMEOUT_SECS`] and was killed (13.6).
    TimedOut { secs: u64 },
    /// The harness returned nothing usable after text extraction (13.5).
    NoAnswer,
}

impl ChatError {
    /// The short failure class, stable enough to assert on and free of any
    /// harness detail. Mirrors the fixed classes `cmd_summarize` prints.
    pub fn class(&self) -> &'static str {
        match self {
            ChatError::DeliveryUnverified { .. } => "harness refused",
            ChatError::HarnessNotInstalled { .. } => "harness not installed",
            ChatError::ContextUnavailable => "context unavailable",
            ChatError::HarnessUnavailable => "harness unavailable",
            ChatError::HarnessFailed => "harness error",
            ChatError::TimedOut { .. } => "timeout",
            ChatError::NoAnswer => "no answer",
        }
    }

    /// The one line the chat prints. Always ends by saying the chat is still
    /// ready, because it always is (13.5, 13.6).
    pub fn message(&self) -> String {
        let body = match self {
            ChatError::DeliveryUnverified { harness } => format!(
                "harness '{harness}' has no verified private channel for the bounded context. \
                 Pitwall will not place workspace context in a command line, so this harness \
                 cannot answer questions; start a chat with a harness that has one (opencode)"
            ),
            ChatError::HarnessNotInstalled { harness } => {
                format!("harness '{harness}' is not installed here")
            }
            ChatError::ContextUnavailable => {
                "the bounded context could not be prepared; nothing was sent to the harness"
                    .to_string()
            }
            ChatError::HarnessUnavailable => "the harness could not be started".to_string(),
            ChatError::HarnessFailed => "the harness could not answer this question".to_string(),
            ChatError::TimedOut { secs } => {
                format!("the harness did not answer within {secs}s and was stopped")
            }
            ChatError::NoAnswer => "the harness returned no usable text".to_string(),
        };
        format!("{body}; the chat is ready for the next input.")
    }
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

/// Workspace facts narrowed to a chat's scope, before the bounded context is
/// assembled. Produced only by [`scope_to_context`].
///
/// Fields are private and read through accessors, following
/// [`crate::context::SummaryContext`]: the scoping decision is made once and
/// cannot be widened afterwards by a later stage reaching into the struct.
pub struct ChatScope {
    snapshot: crate::collector::WorkspaceSnapshot,
    prev: Vec<crate::store::PrevSession>,
    /// Checkpoints created since the previous retained observation — the
    /// event-derivation input, not context content.
    recent: Vec<crate::store::Checkpoint>,
    /// Checkpoints presented as context content (newest per project).
    checkpoints: Vec<crate::store::Checkpoint>,
    notifications: Vec<crate::store::Notification>,
}

impl ChatScope {
    /// The scoped observation.
    pub fn snapshot(&self) -> &crate::collector::WorkspaceSnapshot {
        &self.snapshot
    }

    /// The scoped previous-observation sessions (event derivation input).
    pub fn prev_sessions(&self) -> &[crate::store::PrevSession] {
        &self.prev
    }

    /// The scoped checkpoints presented as context content.
    pub fn checkpoints(&self) -> &[crate::store::Checkpoint] {
        &self.checkpoints
    }

    /// The scoped unread notifications.
    pub fn notifications(&self) -> &[crate::store::Notification] {
        &self.notifications
    }

    /// Assemble the bounded [`crate::context::SummaryContext`] (12.1, 12.2).
    ///
    /// Events are derived here, from the *scoped* previous sessions against
    /// the *scoped* snapshot, so an out-of-scope session cannot contribute an
    /// event; the caps are the same numbers `cmd_summarize` passes, and the
    /// ≤6-session and ≤16 KB document bounds are enforced downstream by
    /// [`crate::context::build_context_from_summary`], untouched.
    pub fn into_summary_context(mut self) -> crate::context::SummaryContext {
        let events = scoped_events(&self);
        self.checkpoints.truncate(MAX_CHAT_CHECKPOINTS);
        self.notifications.truncate(MAX_CHAT_NOTIFICATIONS);
        crate::context::SummaryContext::new(
            self.snapshot,
            events,
            self.checkpoints,
            self.notifications,
        )
    }
}

/// Derived events for one scoped observation, capped at [`MAX_CHAT_EVENTS`].
///
/// The inputs are the scoped previous sessions, the scoped snapshot and the
/// checkpoints created since that previous observation — exactly the three
/// inputs `cmd_summarize` passes, on the narrowed set.
fn scoped_events(scope: &ChatScope) -> Vec<crate::context::Event> {
    let prev = &scope.prev;
    let recent = &scope.recent;
    crate::context::derive_events(prev, &scope.snapshot, recent, MAX_CHAT_EVENTS)
}

/// Narrow observed facts to a chat's scope (12.3, 12.4). Pure: no `Platform`,
/// no store, no clock.
///
/// - **No context session id** ⇒ the whole observed workspace, unfiltered
///   (12.4).
/// - **A context session id that is currently observed** ⇒ that session and
///   its project: every session sharing the project id, and the checkpoints,
///   notifications and previous-observation rows belonging to that session or
///   that project (12.3).
/// - **A context session id that is *not* observed** (the session ended since
///   the chat started) ⇒ the rows that still name that session and nothing
///   else. The scope's subject is gone, so the context honestly holds no live
///   session; the previous-observation row is kept deliberately, because it is
///   what lets [`crate::context::derive_events`] report that the session
///   ended, and its checkpoint is what makes it resumable. Widening back to
///   the whole workspace here would silently answer about sessions the human
///   did not scope the chat to.
pub fn scope_to_context(
    context_session_id: Option<&str>,
    mut snapshot: crate::collector::WorkspaceSnapshot,
    mut prev: Vec<crate::store::PrevSession>,
    mut recent: Vec<crate::store::Checkpoint>,
    mut checkpoints: Vec<crate::store::Checkpoint>,
    mut notifications: Vec<crate::store::Notification>,
) -> ChatScope {
    let Some(session_id) = context_session_id else {
        return ChatScope {
            snapshot,
            prev,
            recent,
            checkpoints,
            notifications,
        };
    };

    // The project the scoped session belongs to, when it is still observed.
    let project_id: Option<String> = snapshot
        .sessions
        .iter()
        .find(|s| s.id == session_id)
        .and_then(|s| s.project.as_ref().map(|p| p.id.clone()));
    let scoped = project_id.as_deref();

    snapshot.sessions.retain(|s| {
        s.id == session_id || s.project.as_ref().is_some_and(|p| same_project(scoped, &p.id))
    });
    prev.retain(|p| {
        // `PrevSession::project_id` is optional (a session observed without a
        // project). Absent means "not in the scoped project", never "keep".
        let project = p.project_id.as_deref().unwrap_or("");
        p.session_id == session_id || same_project(scoped, project)
    });
    recent.retain(|c| c.session_id == session_id || same_project(scoped, &c.project_id));
    checkpoints.retain(|c| c.session_id == session_id || same_project(scoped, &c.project_id));
    notifications.retain(|n| n.session_id == session_id || same_project(scoped, &n.project_id));

    ChatScope {
        snapshot,
        prev,
        recent,
        checkpoints,
        notifications,
    }
}

/// Is `candidate` the scoped project id?
///
/// `None` scoped (the context session is not observed, or carries no project)
/// and an empty candidate both answer `false`: absence of a project is never a
/// reason to widen the scope.
fn same_project(scoped: Option<&str>, candidate: &str) -> bool {
    scoped.is_some_and(|s| !candidate.is_empty() && candidate == s)
}

/// One question's worth of inert workspace facts: the bounded
/// [`crate::context::SummaryContext`] and the document rendered from it.
///
/// "Inert" is the whole point. This is the value that crosses from the
/// observing side (which holds a `Platform`) to [`respond`] (which must not),
/// and it carries data only: two strings and a bounded context. There is no
/// `Platform`, no `Store`, no path and no callback in it, so possessing one
/// grants no ability to change anything.
pub struct ChatObservation {
    context: crate::context::SummaryContext,
    document: String,
    truncated_sessions: usize,
}

impl ChatObservation {
    /// The bounded context these facts came from.
    pub fn context(&self) -> &crate::context::SummaryContext {
        &self.context
    }

    /// The rendered bounded document — what the harness is given, verbatim.
    pub fn document(&self) -> &str {
        &self.document
    }

    /// How many sessions the document had to omit to stay inside the 16 KB
    /// bound. Reported honestly by the renderer; presented by the header, not
    /// invented here.
    pub fn truncated_sessions(&self) -> usize {
        self.truncated_sessions
    }
}

/// Observe the workspace for one question (12.5): fresh observation, bounded
/// context, rendered document.
///
/// This is the *only* function on the question path that takes a
/// `&dyn Platform` and a data directory, and it neither invokes a harness nor
/// changes any workspace state: it collects, reads the continuity store, and
/// renders (27.10). Its result is handed to [`respond`], which cannot act.
///
/// Store reads degrade exactly as `cmd_summarize`'s do — previous observation,
/// recent checkpoints, newest-per-project checkpoints and unread notifications
/// are each optional, and an unavailable store yields an event-free context
/// built from the live observation alone rather than a failure. Chat opens no
/// database that does not already exist, and writes nothing (15.6).
pub fn observe(
    platform: &dyn crate::platform::Platform,
    d: &ChatDescriptor,
    data_dir: &std::path::Path,
) -> ChatObservation {
    let snapshot = crate::collector::collect(platform);
    let facts = read_store_facts(data_dir);
    let context = scope_to_context(
        d.context_session_id(),
        snapshot,
        facts.prev,
        facts.recent,
        facts.checkpoints,
        facts.notifications,
    )
    .into_summary_context();
    let (document, truncated_sessions) =
        crate::context::build_context_from_summary(platform, &context);
    ChatObservation {
        context,
        document,
        truncated_sessions,
    }
}

/// The continuity-store half of an observation. Every field is empty when the
/// store is unavailable.
struct StoreFacts {
    prev: Vec<crate::store::PrevSession>,
    recent: Vec<crate::store::Checkpoint>,
    checkpoints: Vec<crate::store::Checkpoint>,
    notifications: Vec<crate::store::Notification>,
}

impl StoreFacts {
    fn empty() -> StoreFacts {
        StoreFacts {
            prev: Vec::new(),
            recent: Vec::new(),
            checkpoints: Vec::new(),
            notifications: Vec::new(),
        }
    }
}

/// Read the degradable store facts, in the same order and with the same caps
/// as `cmd_summarize`.
///
/// Each step is independently optional: a missing database, a failed open, a
/// missing previous observation or a failed query costs exactly the facts that
/// step would have added. The note printed on an unavailable store names no
/// path — the human did not supply one to a running chat.
fn read_store_facts(data_dir: &std::path::Path) -> StoreFacts {
    let db = data_dir.join(crate::store::DB_FILENAME);
    if !db.exists() {
        // No continuity history yet. Not a failure and not worth a note: the
        // live observation is the whole truth here.
        return StoreFacts::empty();
    }
    let Ok(store) = crate::store::Store::open(&db) else {
        eprintln!("pitwall chat: note: continuity store unavailable; using live observation");
        return StoreFacts::empty();
    };

    // Previous observation + checkpoints created since it: the event
    // derivation inputs. Either missing means no derived events, not an error.
    let (prev, recent) = (|| {
        let (obs_id, collected_at) = store.latest_observation().ok()??;
        let prev = store.observation_sessions(obs_id).ok()?;
        let recent = store.checkpoints_since(collected_at, MAX_CHAT_EVENTS as i64).ok()?;
        Some((prev, recent))
    })()
    .unwrap_or((Vec::new(), Vec::new()));

    // Context checkpoints: newest per project, capped. Same shape as
    // `cmd_summarize` (scan the newest 50, keep the first per project).
    let mut seen_projects = std::collections::HashSet::new();
    let mut checkpoints: Vec<crate::store::Checkpoint> = Vec::new();
    if let Ok(all) = store.latest_checkpoints(50) {
        for cp in all {
            if seen_projects.insert(cp.project_id.clone()) {
                checkpoints.push(cp);
            }
            if checkpoints.len() >= MAX_CHAT_CHECKPOINTS {
                break;
            }
        }
    }

    let notifications = store
        .unread_notifications(MAX_CHAT_NOTIFICATIONS as i64)
        .unwrap_or_default();

    StoreFacts {
        prev,
        recent,
        checkpoints,
        notifications,
    }
}

/// One harness invocation: the fixed argument vector, plus whether the
/// bounded document travels on stdin.
///
/// Produced only by [`build_chat_call`], so a `HarnessCall` existing is the
/// proof that its argv came from the design's table and that the document is
/// not in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessCall {
    argv: Vec<String>,
    context_on_stdin: bool,
}

impl HarnessCall {
    /// The fixed argument vector; `argv[0]` is the harness binary.
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// Does the bounded document travel on the child's stdin?
    pub fn context_on_stdin(&self) -> bool {
        self.context_on_stdin
    }
}

/// The single argv element that carries the question (14.2).
///
/// The chat instruction and the question are joined into **one** string. The
/// question arrives from [`ChatQuestion`], which already proves it is trimmed,
/// control-stripped, scrubbed and at most [`MAX_CHAT_QUESTION_CHARS`]
/// characters; nothing here re-sanitises it and nothing here can turn it into
/// a flag, a path, or a second element.
pub fn chat_message(question: &ChatQuestion) -> String {
    debug_assert!(question.chars() <= MAX_CHAT_QUESTION_CHARS);
    format!("{CHAT_INSTRUCTION}\n\nQuestion: {}", question.text())
}

/// Build the fixed per-harness call from design §4.6's table:
///
/// | Harness | argv | context |
/// |---|---|---|
/// | `opencode` | `<bin> run --format json --dir DIR -f CTX [-m MODEL] <MSG>` | `-f` file |
/// | `claude` | `<bin> -p <MSG>` | stdin |
/// | `codex` | `<bin> exec <MSG>` | stdin |
///
/// Fixed means fixed: the binary is a discovered absolute path, every flag is
/// a literal in this function, `DIR` and `MODEL` come from the immutable
/// descriptor (already validated at capture), and `<MSG>` is the single
/// trailing element [`chat_message`] produced. Nothing the human typed becomes
/// a flag or a path (14.1, 14.4), and there is no shell anywhere on the path.
///
/// `claude` and `codex` get no `-m`: neither exposes a verified model surface
/// ([`crate::agents::ModelDiscovery::AgentDefault`]), so they run on their own
/// configured model — which is exactly what the descriptor's
/// [`AGENT_DEFAULT_LABEL`] presents in the header and title (16.3).
///
/// `context_path` is used **only** for the [`ContextDelivery::File`] harness;
/// for the stdin harnesses the path appears nowhere in argv, and the
/// `debug_assert` below states that as a checked invariant.
pub fn build_chat_call(
    d: &ChatDescriptor,
    binary: &std::path::Path,
    context_path: &std::path::Path,
    message: &str,
) -> Result<HarnessCall, ChatError> {
    let delivery = delivery_for(d.harness()).ok_or_else(|| ChatError::DeliveryUnverified {
        harness: d.harness().to_string(),
    })?;
    let bin = binary.to_string_lossy().into_owned();
    let argv = match delivery {
        ContextDelivery::File => {
            let mut argv = vec![
                bin,
                "run".to_string(),
                "--format".to_string(),
                "json".to_string(),
                "--dir".to_string(),
                d.project_dir().to_string(),
                "-f".to_string(),
                context_path.to_string_lossy().into_owned(),
            ];
            if !d.model().is_empty() {
                argv.push("-m".to_string());
                argv.push(d.model().to_string());
            }
            argv.push(message.to_string());
            argv
        }
        ContextDelivery::Stdin => {
            let flag = match d.harness() {
                "claude" => "-p",
                "codex" => "exec",
                // Unreachable: `DELIVERY` and this match are the same closed
                // set, and `delivery_for` already refused everything else.
                other => {
                    debug_assert!(false, "no argv shape recorded for harness {other:?}");
                    return Err(ChatError::DeliveryUnverified {
                        harness: other.to_string(),
                    });
                }
            };
            vec![bin, flag.to_string(), message.to_string()]
        }
    };

    let context_on_stdin = matches!(delivery, ContextDelivery::Stdin);
    let staged = context_path.to_string_lossy().into_owned();
    let path_in_argv = argv.iter().any(|a| a.contains(&staged));
    debug_assert!(
        !context_on_stdin || !path_in_argv,
        "a stdin-delivery harness must not carry the context path in argv"
    );
    let message_elements = argv.iter().filter(|a| a.as_str() == message).count();
    debug_assert_eq!(message_elements, 1, "the question is one argv element");
    Ok(HarnessCall {
        argv,
        context_on_stdin,
    })
}

/// Answer one question. **No `&dyn Platform`, no database path, no resume
/// handle** — see the section banner: this signature is the structural
/// no-action guarantee (14.5, 27.1, 27.2, 27.3).
///
/// Production entry point: the ephemeral file lands in
/// [`crate::context::ephemeral_dir`], the harness binary is discovered on
/// `PATH`, and the budget is [`CHAT_TIMEOUT_SECS`]. See [`respond_in`].
pub fn respond(
    d: &ChatDescriptor,
    observation: &ChatObservation,
    question: &ChatQuestion,
) -> Result<String, ChatError> {
    respond_in(
        d,
        observation,
        question,
        &crate::context::ephemeral_dir(),
        &crate::agents::path_dirs(),
        std::time::Duration::from_secs(CHAT_TIMEOUT_SECS),
    )
}

/// Same, with the runtime directory, the binary search path and the timeout
/// supplied (test seam, mirroring
/// [`crate::context::EphemeralContext::create_in`] and
/// [`allocate_chat_number_in`], so a test never touches the real runtime
/// directory and never runs a real harness).
///
/// The order is deliberate and each step refuses before the next becomes
/// possible:
///
/// 1. **delivery** — a harness with no recorded private channel for the
///    document is refused here, before anything is staged or spawned (§9.3);
/// 2. **binary** — discovered by [`crate::agents::discover_in`], so what runs
///    is an absolute path to a known name, never a caller-supplied string;
/// 3. **stage** — [`crate::context::EphemeralContext::create_owned_in`] writes
///    the document `0600` under a pid-tagged name (12.8, 15.7);
/// 4. **argv** — [`build_chat_call`], one element for the message;
/// 5. **run** — one [`crate::summary::run_agent`] call, the same single
///    runner, with the timeout that kills the child (13.6). Exactly one
///    harness invocation per question, and none for anything else (12.6,
///    12.7);
/// 6. **extract** — [`crate::summary::extract_summary_text`], so tool-call and
///    metadata events are discarded rather than presented (13.7);
/// 7. **present** — [`present_answer`] strips controls and scrubs (15.5).
///
/// **Cleanup on every path.** The context file is removed by the explicit
/// `close()` below on success, on refusal, on spawn failure, on non-zero exit,
/// on empty text and on timeout, and by `EphemeralContext`'s `Drop` on any
/// path that does not reach it (panic included) (12.8, 15.7).
///
/// **Title re-assertion on every path.** [`assert_window_title`] runs after
/// the harness has finished whether it answered or not: a harness that emitted
/// its own OSC 2 sequence before failing must not keep the window (16.2). It
/// is a no-op unless stdout is a TTY.
pub fn respond_in(
    d: &ChatDescriptor,
    observation: &ChatObservation,
    question: &ChatQuestion,
    runtime_dir: &std::path::Path,
    bin_dirs: &[std::path::PathBuf],
    timeout: std::time::Duration,
) -> Result<String, ChatError> {
    // 1. Private delivery channel, or refuse — before anything is staged or
    //    spawned. `build_chat_call` re-checks and is the single source of the
    //    channel actually used below.
    if delivery_for(d.harness()).is_none() {
        return Err(ChatError::DeliveryUnverified {
            harness: d.harness().to_string(),
        });
    }

    // 2. The binary: a known name, found as an absolute path.
    let binary = crate::agents::discover_in(bin_dirs)
        .into_iter()
        .find(|a| a.id == d.harness())
        .and_then(|a| a.path)
        .ok_or_else(|| ChatError::HarnessNotInstalled {
            harness: d.harness().to_string(),
        })?;

    // 3. Stage the bounded document owner-only.
    let document = observation.document();
    let mut ctx = crate::context::EphemeralContext::create_owned_in(runtime_dir, document)
        .map_err(|_| ChatError::ContextUnavailable)?;
    let staged = ctx.path().map(|p| p.to_path_buf());

    let outcome = (|| -> Result<String, ChatError> {
        let path = staged.as_deref().ok_or(ChatError::ContextUnavailable)?;
        // 4. Fixed argv, one message element.
        let message = chat_message(question);
        let call = build_chat_call(d, &binary, path, &message)?;
        // The document travels by file or by pipe, never in argv. The call
        // decides which, so argv and payload cannot disagree.
        let payload = if call.context_on_stdin() {
            Some(document)
        } else {
            None
        };
        // 5. One run, one timeout, one kill.
        let raw = crate::summary::run_agent(call.argv(), payload, timeout)
            .map_err(|e| classify_run_failure(&e, timeout))?;
        // 6. Text only.
        let text = crate::summary::extract_summary_text(&raw);
        // 7. Present.
        let answer = present_answer(&text);
        if answer.is_empty() {
            return Err(ChatError::NoAnswer);
        }
        Ok(answer)
    })();

    ctx.close();
    debug_assert!(staged.is_some_and(|p| !p.exists()));
    assert_window_title(d);
    outcome
}

/// Map [`crate::summary::run_agent`]'s error text onto a fixed failure class.
///
/// The runner reports failures as prose for the CLI to print; a chat must not
/// print it (it can name the binary, the argv and the exit status), so the
/// text is *classified and dropped* here. Matching on the runner's fixed
/// prefixes is the same technique `cmd_summarize` uses for its short classes;
/// the fallback is [`ChatError::HarnessFailed`], so a wording change in the
/// runner can at worst blur two failure classes together and can never leak
/// the text or lose the "still ready" guarantee.
fn classify_run_failure(error: &str, timeout: std::time::Duration) -> ChatError {
    if error.contains("timed out") {
        ChatError::TimedOut {
            secs: timeout.as_secs(),
        }
    } else if error.contains("spawn failed") {
        ChatError::HarnessUnavailable
    } else {
        ChatError::HarnessFailed
    }
}

/// Make extracted harness text safe to print (15.5).
///
/// Controls are stripped first — a harness answer is untrusted text, and an
/// answer carrying `ESC ] 2 ;` would otherwise retitle the very window whose
/// title is half of this chat's identity — then
/// [`crate::context::scrub_string`] redacts anything secret-shaped, in the
/// same order [`classify_question`] applies to the human's side of the
/// conversation. Empty output stays empty, which [`respond_in`] turns into
/// [`ChatError::NoAnswer`] (13.5).
pub fn present_answer(text: &str) -> String {
    let clean = crate::context::strip_controls(text);
    crate::context::scrub_string(clean.trim()).trim().to_string()
}

// ---------------------------------------------------------------------------
// Chat_Header and branding — task 9.3 (design §4.5)
//
// Everything a human sees at startup, and the shape of every conversation
// line afterwards. Three rules govern this whole section:
//
// 1. **Text is the product; the picture is decoration.** Every builder below
//    returns a `String` and touches nothing outside itself. The branding
//    asset is planned, never required: `InlineImage::None`, a missing asset
//    and an asset in a form we cannot emit are one and the same outcome —
//    the textual header, printed in full, with no apology, no warning and no
//    substitute graphics of any kind (20.8, 28.10).
// 2. **Pure builders, thin printer.** `render_header`, `render_turn` and the
//    line builders are pure functions of a `ChatDescriptor`, a `Turn` and a
//    `Palette`. Only `print_*`/`show_header` touch a sink, so the whole
//    layout is testable without a terminal — the same split the OSC title
//    emitter uses in §4.4.
// 3. **Plain SGR, nothing else.** Colour is four constant SGR strings
//    (bright cyan accent, bold white, dim, reset). No cursor addressing, no
//    alternate screen, no line drawing: a divider is a run of ASCII `-`,
//    because a frame around the conversation would be exactly the simulated
//    widget 20.11 forbids. `Palette::detect` degrades to plain text when
//    stdout is not a TTY or `NO_COLOR` is set.
//
// The conversation itself is a `Vec<Turn>` in [`Conversation`] and nowhere
// else: no file, no database, no log, no `Drop` that writes anything. The
// process exiting is the whole of its lifetime (19.4, 19.5, 15.6).
// ---------------------------------------------------------------------------

/// SGR: bright cyan. The Pitwall accent, used for the wordmark tagline, role
/// labels and the words a human is meant to type.
const SGR_ACCENT: &str = "\u{1b}[96m";
/// SGR: bold bright white. Identity and field values on a dark terminal.
const SGR_STRONG: &str = "\u{1b}[1;97m";
/// SGR: dim. Labels, dividers and timestamps — present but not competing.
const SGR_DIM: &str = "\u{1b}[2m";
/// SGR: reset all attributes.
const SGR_RESET: &str = "\u{1b}[0m";

/// Whether presentation may use colour, decided once and passed down.
///
/// A value rather than an ambient check, so every builder is pure and both
/// presentations are testable: [`Palette::plain`] emits no escape byte at
/// all, which is also what a redirected stdout gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    colour: bool,
}

impl Palette {
    /// Plain text: no SGR sequences anywhere.
    pub fn plain() -> Palette {
        Palette { colour: false }
    }

    /// The Pitwall cyan/white treatment (20.9).
    pub fn coloured() -> Palette {
        Palette { colour: true }
    }

    /// The palette for this process's stdout.
    ///
    /// Colour only when stdout is a terminal — a redirected header must stay
    /// greppable — and never when `NO_COLOR` is set to a non-empty value, the
    /// convention every other tool on the machine already follows.
    pub fn detect() -> Palette {
        use std::io::IsTerminal;
        if !std::io::stdout().is_terminal() {
            return Palette::plain();
        }
        match std::env::var("NO_COLOR") {
            Ok(v) if !v.is_empty() => Palette::plain(),
            _ => Palette::coloured(),
        }
    }

    /// Does this palette emit SGR sequences?
    pub fn is_coloured(self) -> bool {
        self.colour
    }

    fn wrap(self, sgr: &str, text: &str) -> String {
        if self.colour && !text.is_empty() {
            format!("{sgr}{text}{SGR_RESET}")
        } else {
            text.to_string()
        }
    }

    /// Cyan.
    pub fn accent(self, text: &str) -> String {
        self.wrap(SGR_ACCENT, text)
    }

    /// Bold white.
    pub fn strong(self, text: &str) -> String {
        self.wrap(SGR_STRONG, text)
    }

    /// Dim.
    pub fn dim(self, text: &str) -> String {
        self.wrap(SGR_DIM, text)
    }
}

// --- wall-clock rendering --------------------------------------------------
//
// Pitwall carries no date library and no libc binding, so there is no time
// zone database to consult: these two functions convert Unix seconds to a
// **UTC** civil date with integer arithmetic only (Howard Hinnant's
// `civil_from_days`). The header therefore prints its start time with an
// explicit `UTC` suffix and each turn's clock does the same, because a bare
// `12:24` that is not local time would be a quietly wrong fact — and Pitwall
// never presents one. Local-time rendering would need a dependency, which is
// a decision for the human, not for this task.

/// Year, month, day for a count of days since 1970-01-01 (proleptic
/// Gregorian, UTC). Exact integer arithmetic, valid for negative days too.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    // Shift the epoch to 0000-03-01 so leap days fall at the end of the
    // 400-year era, which is what makes the rest of this division-only.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // day of era, [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // March-based month, [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (y + i64::from(m <= 2), m, d)
}

/// `YYYY-MM-DD HH:MM:SS UTC` for Unix seconds. Used for the header's
/// `Started` field (20.2).
pub fn format_epoch_utc(epoch: i64) -> String {
    let days = epoch.div_euclid(86_400);
    let secs = epoch.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02} UTC")
}

/// `HH:MM UTC` for Unix seconds. Used for conversation-turn timestamps
/// (20.10), where the date is the header's job and would only be noise.
pub fn format_clock_utc(epoch: i64) -> String {
    let secs = epoch.rem_euclid(86_400);
    let (hh, mm) = (secs / 3600, (secs % 3600) / 60);
    format!("{hh:02}:{mm:02} UTC")
}

// --- the header ------------------------------------------------------------

/// The wordmark, exactly as the branding artwork sets it (20.1).
pub const WORDMARK: &str = "PITWALL";

/// The wordmark's line under it, from the same artwork.
pub const WORDMARK_TAGLINE: &str = "YOUR AI PIT CREW";

/// The one-line purpose, from the reference layout.
pub const HEADER_PURPOSE: &str = "Workspace-aware AI assistant for your development sessions";

/// Width of the divider rule (20.4).
pub const HEADER_WIDTH: usize = 72;

/// The input prompt. Two characters, no widgets, no cursor addressing.
pub const PROMPT: &str = "> ";

/// The four-verb column the reference sets beside the field block. Presented,
/// never acted on: it is the product statement, not a menu.
const HEADER_VERBS: &[&str] = &["OBSERVE", "UNDERSTAND", "RESUME", "CONTROL", "WITH AI"];

/// Column the verb list starts in, so the block keeps its shape for short
/// field values as well as long ones.
const HEADER_LEFT_WIDTH: usize = 46;

/// The divider rule: ASCII hyphens.
///
/// Deliberately not a box-drawing rule. 20.11 forbids bordered bubbles,
/// panes and simulated widgets, and the cheapest way to guarantee Pitwall
/// never grows one by accident is to keep every box-drawing codepoint out of
/// the chat surface entirely — which the tests assert.
pub fn divider() -> String {
    "-".repeat(HEADER_WIDTH)
}

/// `Pitwall Chat NNN` (20.1). Same literal and same three digits the window
/// title and the panel entry use.
pub fn header_title(d: &ChatDescriptor) -> String {
    format!("{CHAT_LABEL} {}", d.number_text())
}

/// The header's labelled fields, in reference order (20.2, 11.8).
///
/// Every value comes from the immutable descriptor: `Workspace` is the
/// Context_Label, `Model` is [`ChatDescriptor::model_label`] so an empty
/// configured model presents as `agent default` (11.8, 16.3), and
/// `Session ID` is this chat's own three-digit number — the Pitwall-local
/// context session id, when there is one, is named in the coverage block
/// below, where it can be described rather than just printed.
pub fn header_fields(d: &ChatDescriptor) -> Vec<(&'static str, String)> {
    vec![
        ("Workspace", d.context_label().to_string()),
        ("Harness", d.harness().to_string()),
        ("Model", d.model_label().to_string()),
        ("Session ID", d.number_text()),
        ("Started", format_epoch_utc(d.started_at_epoch())),
    ]
}

/// The field block plus the verb column, aligned.
///
/// Padding is computed from the *plain* widths (labels and values, never the
/// SGR bytes), so a coloured block and a plain block line up identically.
fn render_field_block(d: &ChatDescriptor, p: Palette) -> String {
    let fields = header_fields(d);
    let label_width = fields
        .iter()
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0);
    let left_width = fields
        .iter()
        .map(|(_, v)| 2 + label_width + 3 + v.chars().count())
        .max()
        .unwrap_or(0)
        .max(HEADER_LEFT_WIDTH);

    let mut out = String::new();
    for (i, (label, value)) in fields.iter().enumerate() {
        let padded_label = format!("{label:<w$}", w = label_width);
        let visible = 2 + label_width + 3 + value.chars().count();
        let mut line = format!("  {} : {}", p.dim(&padded_label), p.strong(value));
        if let Some(verb) = HEADER_VERBS.get(i).copied() {
            line.push_str(&" ".repeat(left_width.saturating_sub(visible) + 4));
            line.push_str(&p.accent(verb));
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// The "connected to Pitwall context" block, naming what the context covers
/// (20.3).
///
/// The coverage sentence is derived from the descriptor alone — the presence
/// of a context session id and the Context_Label — so it can never overstate:
/// a workspace-scoped chat says it sees the whole observed workspace, and a
/// session-scoped chat names the session and its project and claims nothing
/// wider. The bounds themselves (≤6 sessions, ≤20 events, ≤10 checkpoints,
/// ≤16 KB) belong to `/context`, which reports the observation it actually
/// built; the header describes the *scope*, not one observation's contents.
pub fn context_block(d: &ChatDescriptor) -> Vec<String> {
    let mut lines = vec!["Connected to Pitwall context.".to_string()];
    match d.context_session_id() {
        Some(id) => {
            lines.push(format!(
                "Coverage: session {id} and its project {}.",
                d.context_label()
            ));
            lines.push(
                "That session's observed state, its recent derived events, and its \
                 recent checkpoints."
                    .to_string(),
            );
        }
        None => {
            lines.push("Coverage: the whole observed workspace.".to_string());
            lines.push(
                "Every observed session and its state, recent derived events, and \
                 recent checkpoints."
                    .to_string(),
            );
        }
    }
    lines.push(
        "Nothing outside that context is visible here, and nothing here is stored.".to_string(),
    );
    lines
}

/// The prompt line's own lines: the observe-vs-act statement (27.9) and the
/// instruction for ending the chat (20.5).
///
/// Short on purpose. This is a safety statement, and a safety statement that
/// takes a paragraph is not read.
pub fn prompt_banner() -> Vec<String> {
    vec![
        "Asking a question observes only: it is answered from that context and \
         changes nothing."
            .to_string(),
        "Acting takes a command you enter yourself; /resume is the only one that \
         changes workspace state."
            .to_string(),
        "Type /help for the command list, /exit to end this chat.".to_string(),
    ]
}

/// The whole textual Chat_Header, ready to print.
///
/// Pure: no clock, no environment, no terminal. This is the *complete*
/// header — the branding image, when there is one, is emitted before it and
/// adds nothing to it (20.8).
pub fn render_header(d: &ChatDescriptor, p: Palette) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} \u{00b7} {}\n",
        p.strong(WORDMARK),
        p.accent(WORDMARK_TAGLINE)
    ));
    out.push('\n');
    out.push_str(&format!("{}\n", p.strong(&header_title(d))));
    out.push_str(&format!("{}\n", p.dim(HEADER_PURPOSE)));
    out.push('\n');
    out.push_str(&render_field_block(d, p));
    out.push_str(&format!("{}\n", p.dim(&divider())));
    for (i, line) in context_block(d).iter().enumerate() {
        if i == 0 {
            out.push_str(&format!("{}\n", p.accent(line)));
        } else {
            out.push_str(&format!("{line}\n"));
        }
    }
    out.push_str(&format!("{}\n", p.dim(&divider())));
    for line in prompt_banner() {
        out.push_str(&format!("{}\n", p.dim(&line)));
    }
    out
}

// --- branding --------------------------------------------------------------
//
// **Where the asset is at runtime.** The repository holds it at
// `assets/pitwallpixelart.jpeg`, but an installed `pitwall` is a single
// binary in `~/.local/bin`: `packaging/install.sh` installs the binary, the
// whole `plugin/dev.pitwall` directory (which is how the panel's `flag.svg`
// reaches the system), and the user units — and **no binary-side asset
// directory at all**. There is therefore no existing convention for shipping
// an asset to the CLI, and inventing an install step is not this task's
// call. The lookup below searches the places the existing conventions would
// put it (the XDG data directory, the installed plugin directory, next to
// the binary, the repository checkout) and finding nothing is simply the
// textual header — the same outcome as a terminal without inline images.
//
// **Why the image is not emitted today.** The Kitty graphics protocol
// accepts RGB, RGBA and PNG payloads only, and Sixel needs pixel data; both
// therefore need the JPEG decoded, which needs a dependency this task may
// not add. Scaling is also cell-based in the protocol, so the only form that
// lands at exactly 256×256 pixels *without resampling the artwork* (20.6
// requires it unchanged) is a 256×256 PNG. [`renderable_asset`] states that
// gate; an asset that does not meet it is `Branding::TextOnly`, silently.

/// The Branding_Asset's file name (20.6). One asset, one name, everywhere.
pub const BRANDING_ASSET_FILE: &str = "pitwallpixelart.jpeg";

/// Display size for the Branding_Asset, in pixels (20.7).
pub const BRANDING_DISPLAY_PX: u32 = 256;

/// What the branding stage will do this run.
///
/// There is no third variant, and in particular no "capability present but
/// unusable" variant: everything that is not an emittable image is
/// [`Branding::TextOnly`], which prints the header and says nothing about a
/// picture (20.8, 28.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Branding {
    /// Emit this file inline before the header.
    Inline {
        /// Absolute path to an asset [`renderable_asset`] accepted.
        path: std::path::PathBuf,
    },
    /// Print the textual header only. Not a failure and not reported.
    TextOnly,
}

/// The Omarchy plugin directory the installer writes into — the only place
/// Pitwall already ships a runtime asset (`flag.svg`).
fn plugin_asset_dir() -> std::path::PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return std::path::PathBuf::from(xdg).join("omarchy/plugins/dev.pitwall");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(home).join(".config/omarchy/plugins/dev.pitwall")
}

/// Every place the Branding_Asset could legitimately be, in search order.
///
/// Pure, so the order is testable without touching a filesystem:
///
/// 1. `<data dir>/assets/` — the XDG data directory Pitwall already owns;
/// 2. the installed Omarchy plugin directory — the `flag.svg` precedent;
/// 3. `<binary dir>/assets/` — a self-contained install layout;
/// 4. `<binary dir>/../../assets/` — a `target/release` build in the repo;
/// 5. `<cwd>/assets/` — running from the checkout.
pub fn branding_candidates(
    exe: Option<&std::path::Path>,
    data_dir: &std::path::Path,
    plugin_dir: &std::path::Path,
    cwd: Option<&std::path::Path>,
) -> Vec<std::path::PathBuf> {
    let mut out = vec![
        data_dir.join("assets").join(BRANDING_ASSET_FILE),
        plugin_dir.join(BRANDING_ASSET_FILE),
    ];
    if let Some(dir) = exe.and_then(std::path::Path::parent) {
        out.push(dir.join("assets").join(BRANDING_ASSET_FILE));
        if let Some(repo) = dir.ancestors().nth(2) {
            out.push(repo.join("assets").join(BRANDING_ASSET_FILE));
        }
    }
    if let Some(cwd) = cwd {
        out.push(cwd.join("assets").join(BRANDING_ASSET_FILE));
    }
    out
}

/// The first candidate that exists, or `None`.
///
/// `None` is unremarkable: it means the textual header, with no message.
pub fn branding_asset_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok();
    let cwd = std::env::current_dir().ok();
    branding_candidates(
        exe.as_deref(),
        &crate::store::default_data_dir(),
        &plugin_asset_dir(),
        cwd.as_deref(),
    )
    .into_iter()
    .find(|c| c.is_file())
}

/// PNG pixel dimensions from a file's leading bytes, read straight out of the
/// IHDR chunk. `None` for anything that is not a PNG.
///
/// Signature-based, never extension-based: what a file is called says nothing
/// about what a terminal can decode.
pub fn png_pixel_size(head: &[u8]) -> Option<(u32, u32)> {
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if head.len() < 24 || head[..8] != PNG_MAGIC || head[12..16] != *b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
    let h = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
    Some((w, h))
}

/// The asset at `path`, if it can be emitted unchanged at exactly
/// [`BRANDING_DISPLAY_PX`] square: a PNG of that pixel size.
///
/// Anything else — a JPEG (no in-tree decoder), a differently sized PNG
/// (the protocol scales by cells, and resampling would alter the artwork
/// 20.6 requires unchanged), an unreadable file — yields `None`, which is the
/// textual header.
pub fn renderable_asset(path: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 32];
    let mut filled = 0usize;
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => return None,
        }
    }
    let (w, h) = png_pixel_size(&head[..filled])?;
    (w == BRANDING_DISPLAY_PX && h == BRANDING_DISPLAY_PX).then(|| path.to_path_buf())
}

/// Decide the branding for this run from the runtime probe and the located
/// asset (20.7, 20.8, 28.10).
///
/// Pure: the probe result and the path arrive as arguments, so every
/// combination is testable. `Sixel` is `TextOnly` because emitting sixels
/// needs an encoder Pitwall does not carry.
pub fn plan_branding(
    capability: crate::platform::InlineImage,
    asset: Option<&std::path::Path>,
) -> Branding {
    match capability {
        crate::platform::InlineImage::None | crate::platform::InlineImage::Sixel => {
            Branding::TextOnly
        }
        crate::platform::InlineImage::Kitty => match asset.and_then(renderable_asset) {
            Some(path) => Branding::Inline { path },
            None => Branding::TextOnly,
        },
    }
}

/// Base64, standard alphabet with padding. Small, exact, dependency-free —
/// the graphics protocol requires its payloads base64-encoded.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(ALPHABET[(n >> 18) as usize & 63]));
        out.push(char::from(ALPHABET[(n >> 12) as usize & 63]));
        if chunk.len() > 1 {
            out.push(char::from(ALPHABET[(n >> 6) as usize & 63]));
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(char::from(ALPHABET[n as usize & 63]));
        } else {
            out.push('=');
        }
    }
    out
}

/// The graphics-protocol escape sequence that displays `path` at its natural
/// size: `ESC _ G a=T,f=100,t=f ; <base64 path> ESC \`.
///
/// File medium (`t=f`) rather than an inline payload: the terminal reads the
/// file itself, so a 238 KB asset never crosses the pty, and `t=f` (unlike
/// `t=t`) leaves the file in place. No `c`/`r` keys are sent because the
/// protocol scales by *cells*: the accepted asset is already
/// [`BRANDING_DISPLAY_PX`] square, so natural size *is* 256×256 (20.7).
///
/// `None` for a path that is not absolute or carries a control character —
/// such a path could terminate the sequence early.
pub fn kitty_file_image_sequence(path: &std::path::Path) -> Option<String> {
    let text = path.to_str()?;
    if !text.starts_with('/') || text.chars().any(char::is_control) {
        return None;
    }
    let encoded = base64_encode(text.as_bytes());
    Some(format!("\u{1b}_Ga=T,f=100,t=f;{encoded}\u{1b}\\"))
}

// --- conversation turns ----------------------------------------------------

/// Who said a line.
///
/// Two speakers, because a chat has two: the human and Pitwall. Pitwall's
/// own operational lines (refusals, command output, resume results) are
/// Pitwall turns, so nothing needs a third label or a different shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The human at the keyboard.
    You,
    /// Pitwall, answering or reporting.
    Pitwall,
}

impl Role {
    /// The label that opens a rendered turn (20.10).
    pub fn label(self) -> &'static str {
        match self {
            Role::You => "You",
            Role::Pitwall => "Pitwall",
        }
    }
}

/// One conversation turn: who, when, what.
///
/// Text is control-stripped on the way in (newlines and tabs survive, so a
/// multi-line answer stays multi-line) because a turn is untrusted text that
/// must never carry an escape sequence into the terminal. Secret scrubbing has
/// already happened upstream — [`present_answer`] for answers,
/// [`classify`] for questions — and is not repeated here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    role: Role,
    at_epoch: i64,
    text: String,
}

impl Turn {
    /// Record a turn. `at_epoch` is clamped at zero so no turn can present a
    /// pre-epoch clock.
    pub fn new(role: Role, at_epoch: i64, text: &str) -> Turn {
        Turn {
            role,
            at_epoch: at_epoch.max(0),
            text: crate::context::strip_controls(text).trim_end().to_string(),
        }
    }

    /// Who spoke.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Unix seconds when the turn was recorded.
    pub fn at_epoch(&self) -> i64 {
        self.at_epoch
    }

    /// The turn's text, control-stripped.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// The whole conversation: a `Vec<Turn>` in process memory (19.4).
///
/// This type is the entire storage story. It has no path, no handle and no
/// `Drop` implementation, nothing writes it to the continuity database, to
/// `state.json`, to a log or to the repository (15.6, 19.4), `/clear` empties
/// it ([`Conversation::clear`]), and the process exiting discards it (19.5).
/// A second chat has its own, so two chats share no conversation state
/// (21.7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conversation {
    turns: Vec<Turn>,
}

impl Conversation {
    /// A new, empty conversation.
    pub fn new() -> Conversation {
        Conversation { turns: Vec::new() }
    }

    /// Append a turn, in the order it happened.
    pub fn record(&mut self, role: Role, at_epoch: i64, text: &str) {
        self.turns.push(Turn::new(role, at_epoch, text));
    }

    /// The turns so far.
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    /// How many turns are held.
    pub fn len(&self) -> usize {
        self.turns.len()
    }

    /// Is nothing held?
    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// Discard every turn (`/clear`). Nothing was stored, so nothing else
    /// has to be undone.
    pub fn clear(&mut self) {
        self.turns.clear();
    }
}

/// Unix seconds now, for timestamping a turn. `0` if the clock is before the
/// epoch, which cannot happen on a sane system and is not worth a failure.
pub fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Render one turn as plain labelled lines with a timestamp (20.10).
///
/// The shape is `<role>  <clock>` followed by the text indented two spaces —
/// no bubble, no frame, no pane, no box-drawing character anywhere (20.11).
/// Indentation is the only structure, because indentation is what a terminal
/// already has.
pub fn render_turn(turn: &Turn, p: Palette) -> String {
    let mut out = format!(
        "{}  {}\n",
        p.accent(turn.role().label()),
        p.dim(&format_clock_utc(turn.at_epoch()))
    );
    for line in turn.text().lines() {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

// --- the thin printing layer ----------------------------------------------

/// Emit the branding image, when there is one to emit.
///
/// Writes the escape sequence and one newline so the header starts on a fresh
/// row below the picture. Horizontal placement beside the header would need
/// the terminal's cell pixel size, which Pitwall does not query.
/// [`Branding::TextOnly`] writes nothing at all — not a note, not a blank
/// line (20.8).
pub fn print_branding<W: std::io::Write>(out: &mut W, branding: &Branding) -> std::io::Result<()> {
    if let Branding::Inline { path } = branding {
        if let Some(seq) = kitty_file_image_sequence(path) {
            out.write_all(seq.as_bytes())?;
            out.write_all(b"\n")?;
        }
    }
    Ok(())
}

/// Print the branding (if any) and the complete textual header.
pub fn print_header<W: std::io::Write>(
    out: &mut W,
    d: &ChatDescriptor,
    branding: &Branding,
    p: Palette,
) -> std::io::Result<()> {
    print_branding(out, branding)?;
    out.write_all(render_header(d, p).as_bytes())?;
    out.flush()
}

/// Print one conversation turn.
pub fn print_turn<W: std::io::Write>(out: &mut W, turn: &Turn, p: Palette) -> std::io::Result<()> {
    out.write_all(render_turn(turn, p).as_bytes())?;
    out.flush()
}

/// Print the input prompt and flush, so the cursor sits after it.
pub fn print_prompt<W: std::io::Write>(out: &mut W, p: Palette) -> std::io::Result<()> {
    out.write_all(p.accent(PROMPT).as_bytes())?;
    out.flush()
}

/// Show the header on stdout: probe once, locate the asset once, print.
///
/// The only environment-facing function in this section, and the one the
/// startup path calls. Best-effort by construction: a probe that says
/// `None`, a missing asset and a write error all leave a working chat, and
/// none of them is reported as a problem (28.10).
pub fn show_header(platform: &dyn crate::platform::Platform, d: &ChatDescriptor) {
    let asset = branding_asset_path();
    let branding = plan_branding(platform.inline_image_capability(), asset.as_deref());
    let mut out = std::io::stdout();
    let _ = print_header(&mut out, d, &branding, Palette::detect());
}

// ---------------------------------------------------------------------------
// Resume_Action bridge — task 9.4 (design §4.7)
//
// The only workspace-changing input, executed solely through
// `crate::resume::resume`.
//
// **One caller, and it is the `Act` arm** (26.3–26.6). [`resume_action`] is
// reachable from exactly one place: the `ChatInput::Act(ActionCommand::Resume
// { .. })` arm of the input loop. Three separate facts keep it that way and
// none of them is a convention someone has to remember:
//
// 1. [`classify`] can only produce `Act` for a line whose first word is
//    exactly `/resume` — a question can never reach that variant, whatever it
//    says (27.3, 27.5);
// 2. `resume_action` needs a `&dyn Platform` and a database path, and
//    [`respond`] deliberately has neither (§4.6). The question path therefore
//    cannot construct a call to this function, and neither can the header, the
//    startup path or [`observe`] — none of them is handed a resume handle;
// 3. every refusal, every executor call and every presented line for a resume
//    lives *here*, so a second acting path would have to be written from
//    scratch rather than assembled from parts lying around.
//
// **What this bridge owns, and what the executor owns.** Two refusals belong to
// the bridge because they are about the *request*, and both happen before
// [`crate::resume::resume`] is called at all — no lookup, no collect, no store
// open, no launch (26.9, 26.12):
//
// | refusal | owner |
// |---|---|
// | no argument and no context session id | bridge ([`ResumeRefused::NoTarget`]) |
// | target fails `sess_[0-9a-f]{16}` | bridge ([`ResumeRefused::MalformedTarget`]) |
// | neither live nor checkpointed (26.15) | executor (`unknown session …`) |
// | project dir not absolute (26.16) | executor |
// | project dir missing or not a directory (26.16) | executor |
// | focus failed, launch failed, store unavailable | executor |
//
// The executor's refusals are *carried*, never re-derived: duplicating
// "is it live?" or "does that directory exist?" here would be a second
// implementation of Requirement 26's matrix, free to disagree with the one
// that actually acts. Level 1 focus, Level 2 one-terminal, and the rule that
// a failed focus never falls back to a terminal all likewise stay inside
// [`crate::resume::resume`] (26.13, 26.14, 28.7).
//
// **No agent, ever** (26.20). This section calls no runner: it has no binary
// search path, no timeout and no argv builder, so [`crate::summary::run_agent`]
// cannot be invoked from it, and `resume::resume` has no agent path either —
// its only two outcomes are "focused" and "opened a terminal", and the terminal
// it opens carries an empty command. Requirement 26.19's validation order
// therefore has nothing to govern here: no invocation of this bridge runs an
// agent binary.
//
// **Facts used** (26.24): the descriptor, the entered argument, and the
// executor's result. Nothing else — this section reads no config, opens no
// store of its own and consults no [`ChatObservation`].
//
// **Privacy** (26.25, Requirement 15). Every presented line goes through
// [`one_line`], which strips controls, flattens newlines and tabs, applies
// [`crate::context::scrub_string`] and bounds the length — the same treatment
// [`present_answer`] gives harness text, for the same reason: a checkpointed
// path is stored data, not a literal this code wrote.
//
// The one deliberate decision is the project directory in the
// `OpenedTerminal` line. It is presented, and that is correct:
//
// - Requirement 26.16 *requires* naming the directory in a refusal, so the
//   path is already a presentable value by the spec's own reckoning;
//   suppressing it on success while printing it on failure would be
//   incoherent;
// - it is not a value Requirement 15 excludes: 15.4 excludes arbitrary
//   filesystem *contents*, not the project directory the human pinned or
//   scoped this chat to, which `state.json` already carries and which
//   `pitwall resume` already prints on the CLI;
// - the human needs it. "Opened a terminal" without saying where is a fact
//   they then have to go and verify.
//
// What is *not* presented, on any path: pids and compositor window addresses
// (15.2). The window address never leaves the executor —
// `ResumeOutcome::FocusedLive` carries only the session id — and no pid is in
// reach of this section at all.
// ---------------------------------------------------------------------------

/// Maximum characters of executor-supplied text carried into a presented
/// line: a refusal reason, or the directory a terminal was opened at.
///
/// Sized so a *real* value is never clipped and a planted one cannot flood
/// the terminal. Linux caps a path at 4096 bytes including the terminator, so
/// 4095 characters is the widest legitimate directory; the rest is room for
/// the executor's own prose around it. Requirement 26.16 asks a refusal to
/// name the directory, so quietly shortening one would be the wrong kind of
/// bound here.
const MAX_RESUME_TEXT_CHARS: usize = 4200;

/// The clause every Resume_Action line ends with, because the chat always is
/// ready afterwards (26.23). Same wording [`ChatError::message`] uses, so
/// "what happens next" reads identically whatever happened.
const RESUME_READY: &str = "the chat is ready for the next input.";

/// The refusal body for a `/resume` that resolves to no target at all (26.9).
///
/// It names what is missing — a session id, and a chat that records no context
/// session — and says how to supply one, because the human's next input is the
/// only thing that can fix it.
const NO_TARGET_BODY: &str = "cannot resume: no session id was entered and this chat \
     records no context session, so there is no target; nothing was resumed. Enter \
     '/resume sess_<16 hex digits>' to name one";

/// Reduce executor-supplied text to one presentable, bounded line (26.25).
///
/// [`crate::context::strip_controls`] deliberately keeps `\n` and `\t` — they
/// are content in a harness answer — but a Resume_Action presents *one line*,
/// so they are flattened to spaces here rather than kept. Then
/// [`crate::context::scrub_string`] redacts anything secret-shaped and the
/// result is capped in characters.
///
/// Scrubbing does not change what the reason says (26.22): it replaces a
/// secret-shaped run with `[redacted]` and leaves everything else verbatim,
/// which is exactly the boundary 26.25 asks to keep unchanged.
fn one_line(text: &str, max: usize) -> String {
    let stripped = crate::context::strip_controls(text);
    let flat: String = stripped
        .chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .collect();
    let scrubbed = crate::context::scrub_string(flat.trim());
    truncate_chars(scrubbed.trim(), max)
}

/// What a completed Resume_Action did.
///
/// One variant per operation [`crate::resume::resume`] can perform, so the
/// success line can name *which* operation happened rather than saying
/// something vague like "resumed" (26.21). There is no third variant, because
/// there is no third operation: this type is the whole surface of what a chat
/// can do to a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeDone {
    /// Level 1: the target was live, so its window was focused and no
    /// terminal was opened (26.13, 28.7).
    FocusedLive { session_id: String },
    /// Level 2: the target was not live but held a checkpoint whose project
    /// directory validated, so exactly one terminal was opened there (26.14).
    OpenedTerminal {
        session_id: String,
        /// The checkpointed project directory, presented — see the section
        /// banner for why this is inside Requirement 15's boundary.
        directory: String,
    },
}

impl ResumeDone {
    /// The target session, as named in the presented line.
    pub fn session_id(&self) -> &str {
        match self {
            ResumeDone::FocusedLive { session_id }
            | ResumeDone::OpenedTerminal { session_id, .. } => session_id,
        }
    }

    /// The operation that was performed (26.21). Short, and never a guess:
    /// it is the executor's own outcome variant, renamed for a human.
    pub fn operation(&self) -> &'static str {
        match self {
            ResumeDone::FocusedLive { .. } => "focused its live window",
            ResumeDone::OpenedTerminal { .. } => "opened one terminal",
        }
    }

    /// The one success line: target, operation, and where applicable the
    /// directory (26.21).
    pub fn line(&self) -> String {
        let op = self.operation();
        let body = match self {
            ResumeDone::FocusedLive { session_id } => {
                format!("resumed {session_id}: {op} and opened no new terminal")
            }
            ResumeDone::OpenedTerminal {
                session_id,
                directory,
            } => format!("resumed {session_id}: {op} at {directory}"),
        };
        format!("{body}; {RESUME_READY}")
    }
}

impl std::fmt::Display for ResumeDone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.line())
    }
}

/// Why a Resume_Action did not resume anything.
///
/// The first two variants are the bridge's own refusals and are produced
/// *before* the executor is reached, so their existence is the proof that no
/// executor call happened — see [`ResumeRefused::reached_executor`]. The
/// third carries the executor's reason verbatim (26.22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeRefused {
    /// `/resume` with no argument, from a chat whose descriptor records no
    /// context session id: there is no target to resolve (26.9).
    NoTarget,
    /// The resolved target is not shaped `sess_[0-9a-f]{16}` (26.11, 26.12).
    /// `entered` is bounded and control-stripped, so naming it is safe.
    MalformedTarget { entered: String },
    /// The executor was called and refused. `reason` is its own text, carried
    /// rather than reinterpreted (26.15, 26.16, 26.22).
    Executor { target: String, reason: String },
}

impl ResumeRefused {
    /// The short refusal class, stable enough to assert on. Mirrors
    /// [`ChatError::class`].
    pub fn class(&self) -> &'static str {
        match self {
            ResumeRefused::NoTarget => "no target",
            ResumeRefused::MalformedTarget { .. } => "malformed target",
            ResumeRefused::Executor { .. } => "resume refused",
        }
    }

    /// Did this refusal happen *after* the executor was called?
    ///
    /// `false` for both of the bridge's own refusals, which is what 26.9 and
    /// 26.12 require, and what the tests assert against a recording platform.
    pub fn reached_executor(&self) -> bool {
        matches!(self, ResumeRefused::Executor { .. })
    }

    /// The one failure line (26.9, 26.12, 26.22).
    pub fn line(&self) -> String {
        let body = match self {
            ResumeRefused::NoTarget => NO_TARGET_BODY.to_string(),
            ResumeRefused::MalformedTarget { entered } => format!(
                "cannot resume '{entered}': a session id is 'sess_' followed by 16 \
                 hexadecimal digits; nothing was looked up and nothing was resumed"
            ),
            ResumeRefused::Executor { target, reason } => {
                format!("could not resume {target}: {reason}; nothing else changed")
            }
        };
        format!("{body}; {RESUME_READY}")
    }
}

impl std::fmt::Display for ResumeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.line())
    }
}

/// The outcome of one Resume_Action attempt: what it did, or why it did not.
pub type ResumeReport = Result<ResumeDone, ResumeRefused>;

/// The one line a Resume_Action presents, whatever happened.
///
/// The loop records this as a [`Role::Pitwall`] turn — a resume result is
/// Pitwall reporting, not a third kind of speaker (see [`Role`]).
pub fn resume_line(report: &ResumeReport) -> String {
    match report {
        Ok(done) => done.line(),
        Err(refused) => refused.line(),
    }
}

/// Resolve and shape-validate a Resume_Action target. Pure: no platform, no
/// store, no filesystem (26.7, 26.8, 26.9, 26.11, 26.12).
///
/// Order, and why it is this order:
///
/// 1. an entered argument wins (26.8) — the human naming a session is the
///    most specific thing they can do, and it must not be second-guessed by
///    the chat's own scope;
/// 2. otherwise the descriptor's context session id (26.7), which is
///    immutable and was already shaped by
///    [`crate::resume::is_session_id`] at capture;
/// 3. otherwise [`ResumeRefused::NoTarget`] — a workspace-scoped chat has no
///    implicit target, and *guessing* one from the observed workspace would
///    be exactly the "never guesses among sessions" line `resume` holds
///    (26.9);
/// 4. the resolved candidate is shape-checked before returning, so no caller
///    can perform a lookup on an unvalidated target (26.11).
///
/// The argument is passed through the same hygiene [`classify`] applies, so
/// this is safe to call with a raw string too; the cap is far above the 21
/// characters [`crate::resume::is_session_id`] accepts, so trimming can never
/// manufacture a valid target from an invalid one.
///
/// An empty or whitespace-only argument is a [`ResumeRefused::MalformedTarget`]
/// rather than a fall-back to the descriptor: something was entered as the
/// target, and silently substituting a different session is precisely the
/// behaviour 26.8 forbids. [`classify`] cannot produce one (it splits on
/// whitespace), so this only concerns direct callers.
pub fn resume_target(d: &ChatDescriptor, argument: Option<&str>) -> Result<String, ResumeRefused> {
    let candidate = match argument.map(resume_target_text) {
        Some(entered) => entered,
        None => match d.context_session_id() {
            Some(id) => id.to_string(),
            None => return Err(ResumeRefused::NoTarget),
        },
    };
    if !crate::resume::is_session_id(&candidate) {
        return Err(ResumeRefused::MalformedTarget { entered: candidate });
    }
    Ok(candidate)
}

/// Perform one Resume_Action (26.2). The only workspace-changing path in
/// chat, and the only caller of [`crate::resume::resume`] from this module.
///
/// `argument` is the `/resume` argument exactly as classified — `None` when
/// the human entered `/resume` alone. See the section banner for the
/// single-caller contract, the refusal split, and the privacy decision.
///
/// The executor is invoked as a plain Rust function call with a fixed
/// argument list; the only argument vector in the whole path is the one
/// `Platform::launch_terminal` builds at the single exec boundary, and it
/// carries an empty command, so there is no shell and nothing to interpolate
/// (26.17, 26.18, 28.1).
pub fn resume_action(
    platform: &dyn crate::platform::Platform,
    db_path: &std::path::Path,
    d: &ChatDescriptor,
    argument: Option<&str>,
) -> ResumeReport {
    // Steps 1-2: resolve, then shape-check. Both refusals return here, so
    // nothing below — no collect, no store open, no focus, no launch — has
    // run at that point (26.9, 26.12).
    let target = resume_target(d, argument)?;

    // Step 3: the executor, once. Level 1 vs Level 2, the validated
    // directory, and the no-fallback rule are all its decisions.
    match crate::resume::resume(platform, db_path, &target) {
        // `session_id` is the target the executor focused; taking it from the
        // outcome rather than from `target` keeps the presented line a
        // statement about what happened (26.24). It needs no sanitising: it
        // passed `is_session_id`, so it is 21 characters of `sess_` and
        // lowercase hex and can carry nothing else.
        Ok(crate::resume::ResumeOutcome::FocusedLive { session_id }) => {
            Ok(ResumeDone::FocusedLive { session_id })
        }
        Ok(crate::resume::ResumeOutcome::OpenedTerminal { directory }) => {
            Ok(ResumeDone::OpenedTerminal {
                session_id: target,
                directory: one_line(&directory, MAX_RESUME_TEXT_CHARS),
            })
        }
        Err(reason) => Err(ResumeRefused::Executor {
            target,
            reason: one_line(&reason, MAX_RESUME_TEXT_CHARS),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{ChatLease, RawProcess};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn sandbox() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("pitwall-m8-chat-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn descriptor_at(path: &str) -> ChatDescriptor {
        ChatDescriptor::capture(
            7,
            "opencode",
            "prov/model",
            "Work",
            Some("sess_0123456789abcdef"),
            1_700_000_100,
            path,
        )
        .unwrap()
    }

    #[test]
    fn capture_records_every_fact_verbatim() {
        let dir = sandbox();
        let path = dir.to_string_lossy().into_owned();
        let d = descriptor_at(&path);
        assert_eq!(d.number(), 7);
        assert_eq!(d.number_text(), "007");
        assert_eq!(d.harness(), "opencode");
        assert_eq!(d.model(), "prov/model");
        assert_eq!(d.model_label(), "prov/model");
        assert_eq!(d.context_label(), "Work");
        assert_eq!(d.context_session_id(), Some("sess_0123456789abcdef"));
        assert_eq!(d.started_at_epoch(), 1_700_000_100);
        assert_eq!(d.project_dir(), path.as_str());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_model_presents_the_agent_default() {
        let dir = sandbox();
        let path = dir.to_string_lossy().into_owned();
        let d = ChatDescriptor::capture(1, "claude", "", WORKSPACE_LABEL, None, 0, &path).unwrap();
        assert_eq!(d.model(), "");
        assert_eq!(d.model_label(), AGENT_DEFAULT_LABEL);
        assert_eq!(d.context_label(), "Workspace");
        assert_eq!(d.context_session_id(), None);
        assert_eq!(d.number_text(), "001");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn number_text_is_always_three_digits() {
        let dir = sandbox();
        let path = dir.to_string_lossy().into_owned();
        for (n, want) in [(1u16, "001"), (42, "042"), (999, "999")] {
            let d = ChatDescriptor::capture(n, "codex", "", "Work", None, 0, &path).unwrap();
            assert_eq!(d.number_text(), want);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn capture_refuses_every_invalid_fact() {
        let dir = sandbox();
        let path = dir.to_string_lossy().into_owned();
        let file = dir.join("afile");
        std::fs::write(&file, "x").unwrap();
        let file_path = file.to_string_lossy().into_owned();
        // `bad` is short on purpose: every refusal reads as one line.
        let bad = |n: u16, h: &str, m: &str, l: &str, s: Option<&str>, e: i64, p: &str| {
            ChatDescriptor::capture(n, h, m, l, s, e, p).is_err()
        };
        // Number outside 001..999.
        assert!(bad(0, "opencode", "", "Work", None, 0, &path));
        assert!(bad(1000, "opencode", "", "Work", None, 0, &path));
        // Unknown harness (delegated to agents::KNOWN).
        assert!(bad(1, "evil;id", "", "Work", None, 0, &path));
        // Malformed model (delegated to summary::valid_model).
        assert!(bad(1, "opencode", "bad model", "Work", None, 0, &path));
        // Empty, oversized and unsanitised context labels.
        assert!(bad(1, "opencode", "", "", None, 0, &path));
        let long = "w".repeat(MAX_CONTEXT_LABEL_CHARS + 1);
        assert!(bad(1, "opencode", "", &long, None, 0, &path));
        assert!(bad(1, "opencode", "", "a\u{00B7}b", None, 0, &path));
        assert!(bad(1, "opencode", "", "a\nb", None, 0, &path));
        // Malformed session id (delegated to resume::is_session_id).
        assert!(bad(1, "opencode", "", "Work", Some("nope"), 0, &path));
        // Placeholder clock value.
        assert!(bad(1, "opencode", "", "Work", None, -1, &path));
        // Project dir must be absolute, present, and a directory.
        assert!(bad(1, "opencode", "", "Work", None, 0, "relative/dir"));
        assert!(bad(1, "opencode", "", "Work", None, 0, "/nope/nowhere"));
        assert!(bad(1, "opencode", "", "Work", None, 0, &file_path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chat_timeout_tracks_the_summary_timeout() {
        assert_eq!(CHAT_TIMEOUT_SECS, crate::summary::DEFAULT_TIMEOUT_SECS);
    }

    // --- Chat_Title_Grammar (task 8.2) -----------------------------------

    #[test]
    fn the_separator_constant_and_char_agree() {
        assert_eq!(TITLE_SEPARATOR, format!(" {TITLE_SEPARATOR_CHAR} "));
        // Three characters, four bytes: the reason every cap here counts
        // characters.
        assert_eq!(TITLE_SEPARATOR.chars().count(), 3);
        assert_eq!(TITLE_SEPARATOR.len(), 4);
    }

    #[test]
    fn format_title_writes_the_grammar() {
        let dir = sandbox();
        let path = dir.to_string_lossy().into_owned();
        let d = descriptor_at(&path);
        assert_eq!(
            format_title(&d),
            "Pitwall Chat 007 \u{00B7} opencode \u{00B7} prov/model \u{00B7} Work"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn format_title_uses_the_agent_default_and_workspace_literals() {
        let dir = sandbox();
        let path = dir.to_string_lossy().into_owned();
        let d = ChatDescriptor::capture(1, "claude", "", WORKSPACE_LABEL, None, 0, &path).unwrap();
        assert_eq!(
            format_title(&d),
            "Pitwall Chat 001 \u{00B7} claude \u{00B7} agent default \u{00B7} Workspace"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn titles_round_trip_through_parse() {
        let dir = sandbox();
        let path = dir.to_string_lossy().into_owned();
        let cases = [
            (7u16, "opencode", "prov/model", "Work", Some("sess_0123456789abcdef")),
            (1, "claude", "", "Workspace", None),
            (999, "codex", "a/b-c_d.e:f", "two words", None),
            // A label that is only spaces is still a label capture accepts;
            // parse must not trim it, or the round trip would break.
            (42, "codex", "", " padded ", None),
        ];
        for (n, h, m, l, s) in cases {
            let d = ChatDescriptor::capture(n, h, m, l, s, 0, &path).unwrap();
            let title = format_title(&d);
            let parsed = parse_title(&title).expect("own title must parse");
            assert_eq!(parsed.number(), d.number());
            assert_eq!(parsed.number_text(), d.number_text());
            assert_eq!(parsed.harness(), d.harness());
            assert_eq!(parsed.model(), d.model());
            assert_eq!(parsed.model_label(), d.model_label());
            assert_eq!(parsed.context_label(), title_context_label(&d));
            assert_eq!(parsed.context_label(), d.context_label());
            // The other direction: rendering a parsed title reproduces it.
            assert_eq!(parsed.render(), title);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_extreme_model_shortens_only_the_label() {
        let dir = sandbox();
        let path = dir.to_string_lossy().into_owned();
        // The longest model `valid_model` accepts (128 ASCII chars) leaves
        // exactly 39 characters of label budget out of 200.
        let model = format!("p/{}", "a".repeat(126));
        assert!(crate::summary::valid_model(&model));
        let label = "L".repeat(MAX_CONTEXT_LABEL_CHARS);
        let d = ChatDescriptor::capture(7, "opencode", &model, &label, None, 0, &path).unwrap();
        let title = format_title(&d);
        assert_eq!(title.chars().count(), MAX_CHAT_TITLE_CHARS);
        let parsed = parse_title(&title).expect("a capped title still parses");
        assert_eq!(parsed.model(), model);
        assert_eq!(parsed.context_label(), "L".repeat(39));
        assert_eq!(parsed.context_label(), title_context_label(&d));
        assert_eq!(parsed.render(), title);
        // Nothing else was sacrificed for the cap.
        assert_eq!(parsed.number(), 7);
        assert_eq!(parsed.harness(), "opencode");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_title_accepts_only_the_exact_grammar() {
        let sep = TITLE_SEPARATOR;
        let ok = format!("Pitwall Chat 007{sep}opencode{sep}prov/model{sep}Work");
        assert!(parse_title(&ok).is_some());

        let near_misses = [
            // Wrong literal or casing.
            format!("pitwall chat 007{sep}opencode{sep}prov/model{sep}Work"),
            format!("Pitwall  Chat 007{sep}opencode{sep}prov/model{sep}Work"),
            format!("Pitwall Chat007{sep}opencode{sep}prov/model{sep}Work"),
            // Anything wrapped around the title (a tmux/emulator prefix).
            format!("1: Pitwall Chat 007{sep}opencode{sep}prov/model{sep}Work"),
            // Number: not exactly three ASCII digits, or out of range.
            format!("Pitwall Chat 7{sep}opencode{sep}prov/model{sep}Work"),
            format!("Pitwall Chat 0007{sep}opencode{sep}prov/model{sep}Work"),
            format!("Pitwall Chat 00a{sep}opencode{sep}prov/model{sep}Work"),
            format!("Pitwall Chat 000{sep}opencode{sep}prov/model{sep}Work"),
            format!("Pitwall Chat \u{0660}\u{0660}\u{0661}{sep}opencode{sep}prov/model{sep}Work"),
            // Field count.
            format!("Pitwall Chat 007{sep}opencode{sep}prov/model"),
            format!("Pitwall Chat 007{sep}opencode{sep}prov/model{sep}Work{sep}extra"),
            "Pitwall Chat 007 - opencode - prov/model - Work".to_string(),
            // Harness: unknown, or carrying stray whitespace.
            format!("Pitwall Chat 007{sep}opencodex{sep}prov/model{sep}Work"),
            format!("Pitwall Chat 007{sep}opencode {sep}prov/model{sep}Work"),
            format!("Pitwall Chat 007{sep}{sep}prov/model{sep}Work"),
            // Model: empty, malformed, or a near-miss on the literal.
            format!("Pitwall Chat 007{sep}opencode{sep}{sep}Work"),
            format!("Pitwall Chat 007{sep}opencode{sep}noslash{sep}Work"),
            format!("Pitwall Chat 007{sep}opencode{sep}bad model{sep}Work"),
            format!("Pitwall Chat 007{sep}opencode{sep}agent  default{sep}Work"),
            format!("Pitwall Chat 007{sep}opencode{sep}Agent default{sep}Work"),
            // Label: empty, over-long, or carrying the separator character.
            format!("Pitwall Chat 007{sep}opencode{sep}prov/model{sep}"),
            format!(
                "Pitwall Chat 007{sep}opencode{sep}prov/model{sep}{}",
                "L".repeat(MAX_CONTEXT_LABEL_CHARS + 1)
            ),
            format!("Pitwall Chat 007{sep}opencode{sep}prov/model{sep}Wo\u{00B7}rk"),
            // A control character anywhere.
            format!("Pitwall Chat 007{sep}opencode{sep}prov/model{sep}Wo\u{0007}rk"),
            format!("Pitwall Chat 007{sep}opencode{sep}prov/model{sep}Work\u{001b}]2;x"),
            // Not a title at all.
            String::new(),
            "zsh".to_string(),
            "OC | some agent session".to_string(),
        ];

        for t in near_misses {
            assert!(parse_title(&t).is_none(), "must reject {t:?}");
        }

        // Every field valid, but the whole title exceeds the 200-char cap:
        // the cap alone rejects this one.
        let too_long = format!(
            "Pitwall Chat 007{sep}opencode{sep}p/{}{sep}{}",
            "a".repeat(126),
            "L".repeat(MAX_CONTEXT_LABEL_CHARS)
        );
        assert_eq!(too_long.chars().count(), 209);
        assert!(parse_title(&too_long).is_none());
    }

    #[test]
    fn parse_title_accepts_every_known_harness() {
        let sep = TITLE_SEPARATOR;
        for a in crate::agents::KNOWN {
            let t = format!("Pitwall Chat 123{sep}{}{sep}agent default{sep}Workspace", a.id);
            let parsed = parse_title(&t).expect("known harness must parse");
            assert_eq!(parsed.harness(), a.id);
            assert_eq!(parsed.model(), "");
            assert_eq!(parsed.model_label(), AGENT_DEFAULT_LABEL);
            assert_eq!(parsed.number(), 123);
        }
    }

    // --- context label sanitiser -----------------------------------------

    #[test]
    fn sanitiser_collapses_strips_removes_and_caps() {
        // Whitespace runs collapse; the ends are trimmed.
        assert_eq!(sanitize_context_label("  my   project  "), "my project");
        // Newlines and tabs read as whitespace, not as strippable controls.
        assert_eq!(sanitize_context_label("line1\nline2\tx"), "line1 line2 x");
        // Other controls vanish without widening the label.
        assert_eq!(sanitize_context_label("a\u{0000}b\u{001b}c\u{007f}"), "abc");
        // The separator character is removed.
        assert_eq!(sanitize_context_label("a\u{00B7}b"), "ab");
        assert_eq!(sanitize_context_label("a \u{00B7} b"), "a b");
        // Caps at MAX_CONTEXT_LABEL_CHARS characters, counting characters.
        let long = "x".repeat(MAX_CONTEXT_LABEL_CHARS + 10);
        let capped_long = sanitize_context_label(&long);
        assert_eq!(capped_long.chars().count(), MAX_CONTEXT_LABEL_CHARS);
        // Multi-byte, single-codepoint input: the cap counts characters, so
        // 48 characters survive and none is cut in half.
        let wide = "\u{00E9}".repeat(MAX_CONTEXT_LABEL_CHARS + 10);
        let capped = sanitize_context_label(&wide);
        assert_eq!(capped.chars().count(), MAX_CONTEXT_LABEL_CHARS);
        assert_eq!(capped.len(), MAX_CONTEXT_LABEL_CHARS * 2);
        assert!(capped.chars().all(|c| c == '\u{00E9}'));
        // Never ends on a space, even when the cap lands on one.
        let spaced = format!("{} ab", "y".repeat(MAX_CONTEXT_LABEL_CHARS - 1));
        let out = sanitize_context_label(&spaced);
        assert_eq!(out, "y".repeat(MAX_CONTEXT_LABEL_CHARS - 1));
        // Nothing presentable survives.
        assert_eq!(sanitize_context_label("   "), "");
        assert_eq!(sanitize_context_label("\u{00B7}\u{00B7}"), "");
        assert_eq!(sanitize_context_label(""), "");
    }

    #[test]
    fn sanitised_labels_satisfy_every_label_rule() {
        let long = "z".repeat(100);
        for raw in ["  my   project  ", "line1\nline2", "a\u{00B7}b", long.as_str()] {
            let label = sanitize_context_label(raw);
            assert!(!label.is_empty());
            assert!(label.chars().count() <= MAX_CONTEXT_LABEL_CHARS);
            assert!(label.chars().all(is_label_char));
        }
    }

    #[test]
    fn context_label_for_falls_back_to_workspace() {
        assert_eq!(context_label_for(None), WORKSPACE_LABEL);
        assert_eq!(context_label_for(Some("   ")), WORKSPACE_LABEL);
        assert_eq!(context_label_for(Some("\u{00B7}")), WORKSPACE_LABEL);
        assert_eq!(context_label_for(Some("  pitwall ")), "pitwall");
    }

    // --- OSC emission ----------------------------------------------------

    #[test]
    fn osc_sequence_is_esc_bracket_two_semicolon_title_bel() {
        let seq = window_title_sequence("Pitwall Chat 007").unwrap();
        assert_eq!(seq, "\u{001b}]2;Pitwall Chat 007\u{0007}");
        // A control character in the title could close or re-open the
        // sequence, so no sequence is built at all.
        assert!(window_title_sequence("bad\u{0007}title").is_none());
        assert!(window_title_sequence("bad\u{001b}title").is_none());
        assert!(window_title_sequence("").is_none());
    }

    #[test]
    fn write_window_title_emits_exactly_the_sequence_or_nothing() {
        let dir = sandbox();
        let path = dir.to_string_lossy().into_owned();
        let d = descriptor_at(&path);
        let title = format_title(&d);

        let mut buf: Vec<u8> = Vec::new();
        write_window_title(&mut buf, &title).unwrap();
        let want = format!("\u{001b}]2;{title}\u{0007}");
        assert_eq!(String::from_utf8(buf).unwrap(), want);

        let mut buf: Vec<u8> = Vec::new();
        write_window_title(&mut buf, "no\u{0007}").unwrap();
        assert!(buf.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Chat_Identity_Allocator and the O_EXCL lease (task 8.3) ----------

    /// A process observation entry. `is_live_pitwall_chat` reads argv[0]'s
    /// basename (or `exe_name`) plus argv[1], so a chat is `pitwall chat`
    /// and anything else is not.
    fn raw_process(pid: u32, command: &str, exe_name: &str) -> RawProcess {
        RawProcess {
            pid,
            ppid: 1,
            name: exe_name.to_string(),
            command: command.to_string(),
            exe_name: exe_name.to_string(),
            cwd: "/home/u".to_string(),
            state_code: 'S',
            starttime_ticks: 100,
        }
    }

    fn chat_proc(pid: u32) -> RawProcess {
        raw_process(pid, "/usr/bin/pitwall chat", "pitwall")
    }

    /// A live process that is *not* a chat: holding a lease does not make it
    /// one.
    fn other_proc(pid: u32) -> RawProcess {
        raw_process(pid, "/usr/bin/zsh -l", "zsh")
    }

    fn write_lease(dir: &std::path::Path, number: u16, body: &str) {
        std::fs::write(dir.join(lease_file_name(number)), body).unwrap();
    }

    fn lease_body(dir: &std::path::Path, number: u16) -> String {
        std::fs::read_to_string(dir.join(lease_file_name(number))).unwrap()
    }

    fn own_body() -> String {
        format!("{}\n", std::process::id())
    }

    #[test]
    fn lease_file_names_carry_three_digits() {
        assert_eq!(lease_file_name(1), "chat-001.lease");
        assert_eq!(lease_file_name(42), "chat-042.lease");
        assert_eq!(lease_file_name(999), "chat-999.lease");
    }

    #[test]
    fn allocator_claims_the_lowest_number_and_records_the_owner_pid() {
        let dir = sandbox();
        let procs = [chat_proc(4242)];
        let lease = allocate_chat_number_in(&dir, &[], &[], &procs).unwrap();
        assert_eq!(lease.number(), 1);
        assert_eq!(lease.number_text(), "001");
        let path = dir.join("chat-001.lease");
        assert_eq!(lease.path(), Some(path.as_path()));
        // The body is the pid the collector corroborates a title against.
        assert_eq!(lease_body(&dir, 1), own_body());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(dir_mode & 0o777, 0o700);
        }
        drop(lease);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn observed_windows_and_live_leases_are_both_skipped() {
        let dir = sandbox();
        let procs = [chat_proc(4242), chat_proc(4243)];
        write_lease(&dir, 3, "4243\n");
        let leases = [ChatLease {
            number: 3,
            pid: 4243,
        }];
        // 1 and 2 are held by observed chat windows, 3 by a live lease. Out
        // of range values are not numbers at all and change nothing.
        let lease = allocate_chat_number_in(&dir, &[2, 1, 0, 1000], &leases, &procs).unwrap();
        assert_eq!(lease.number(), 4);
        assert_eq!(lease_body(&dir, 3), "4243\n", "a live lease is untouched");
        drop(lease);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_lease_is_reclaimed_and_its_number_reused() {
        // Owner pid absent from the observation.
        let dir = sandbox();
        let procs = [chat_proc(4242)];
        write_lease(&dir, 1, "424242\n");
        let leases = [ChatLease {
            number: 1,
            pid: 424242,
        }];
        let lease = allocate_chat_number_in(&dir, &[], &leases, &procs).unwrap();
        assert_eq!(lease.number(), 1, "a released number is available again");
        assert_eq!(lease_body(&dir, 1), own_body());
        drop(lease);
        let _ = std::fs::remove_dir_all(&dir);

        // Owner pid alive, but it is not a `pitwall chat`: also stale.
        let dir = sandbox();
        let procs = [other_proc(777)];
        write_lease(&dir, 1, "777\n");
        let leases = [ChatLease {
            number: 1,
            pid: 777,
        }];
        let lease = allocate_chat_number_in(&dir, &[], &leases, &procs).unwrap();
        assert_eq!(lease.number(), 1);
        assert_eq!(lease_body(&dir, 1), own_body());
        drop(lease);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_observation_reclaims_nothing() {
        let dir = sandbox();
        write_lease(&dir, 1, "424242\n");
        let leases = [ChatLease {
            number: 1,
            pid: 424242,
        }];
        // No processes observed means observation failed, not that nothing
        // runs: the lease keeps its number and keeps its file.
        let lease = allocate_chat_number_in(&dir, &[], &leases, &[]).unwrap();
        assert_eq!(lease.number(), 2);
        assert_eq!(lease_body(&dir, 1), "424242\n");
        drop(lease);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_exclusive_create_resolves_leases_the_observation_missed() {
        // A lease created inside the observation gap by a live chat: the
        // O_EXCL create fails, re-validation finds it held, we advance.
        let dir = sandbox();
        let procs = [chat_proc(4242)];
        write_lease(&dir, 1, "4242\n");
        let lease = allocate_chat_number_in(&dir, &[], &[], &procs).unwrap();
        assert_eq!(lease.number(), 2);
        assert_eq!(lease_body(&dir, 1), "4242\n");
        drop(lease);
        let _ = std::fs::remove_dir_all(&dir);

        // Same situation, but the owner is demonstrably gone: unlink and
        // retry the same number.
        let dir = sandbox();
        let procs = [chat_proc(4242)];
        write_lease(&dir, 1, "424242\n");
        let lease = allocate_chat_number_in(&dir, &[], &[], &procs).unwrap();
        assert_eq!(lease.number(), 1);
        assert_eq!(lease_body(&dir, 1), own_body());
        drop(lease);
        let _ = std::fs::remove_dir_all(&dir);

        // An unreadable owner is never unlinked: no owner, no proof.
        let dir = sandbox();
        let procs = [chat_proc(4242)];
        write_lease(&dir, 1, "not-a-pid");
        let lease = allocate_chat_number_in(&dir, &[], &[], &procs).unwrap();
        assert_eq!(lease.number(), 2);
        assert_eq!(lease_body(&dir, 1), "not-a-pid");
        drop(lease);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exhaustion_refuses_and_creates_no_lease() {
        let dir = sandbox();
        let procs = [chat_proc(4242)];
        let all: Vec<u16> = (MIN_CHAT_NUMBER..=MAX_CHAT_NUMBER).collect();
        let err = allocate_chat_number_in(&dir, &all, &[], &procs).unwrap_err();
        assert!(err.contains("001..999"), "{err}");
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "a refusal creates nothing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_is_explicit_idempotent_and_drop_backed() {
        let dir = sandbox();
        let procs = [chat_proc(4242)];
        let mut lease = allocate_chat_number_in(&dir, &[], &[], &procs).unwrap();
        let path = lease.path().unwrap().to_path_buf();
        assert!(path.exists());

        // Explicit release at `/exit`.
        lease.release();
        assert!(!path.exists());
        assert!(lease.path().is_none());
        // Idempotent: a second release (and the `Drop` still to come) must
        // find nothing to do.
        lease.release();

        // The released number is the lowest available one again (10.3).
        let reused = allocate_chat_number_in(&dir, &[], &[], &procs).unwrap();
        assert_eq!(reused.number(), 1);
        assert_eq!(reused.path(), Some(path.as_path()));
        drop(reused);
        assert!(!path.exists(), "Drop releases the lease");

        // A guard that already released must never unlink somebody else's
        // later lease of the same number.
        let third = allocate_chat_number_in(&dir, &[], &[], &procs).unwrap();
        let third_path = third.path().unwrap().to_path_buf();
        drop(lease);
        assert!(third_path.exists(), "a released guard unlinks nothing");
        drop(third);
        assert!(!third_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lease_bodies_are_read_strictly() {
        let dir = sandbox();
        let read = |body: &str| {
            let path = dir.join("probe.lease");
            std::fs::write(&path, body).unwrap();
            read_lease_pid(&path)
        };
        assert_eq!(read("4242\n"), Some(4242));
        assert_eq!(read("4242"), Some(4242));
        assert_eq!(read("  4242  \n"), Some(4242));
        // Empty, zero, non-decimal, more than one field, and an implausibly
        // long pid all yield "no owner" — which never unlinks anything.
        assert_eq!(read(""), None);
        assert_eq!(read("0\n"), None);
        assert_eq!(read("abc"), None);
        assert_eq!(read("42 43"), None);
        assert_eq!(read("99999999999"), None);
        // A planted large file is read bounded, so it cannot be a pid.
        assert_eq!(read(&"1".repeat(4096)), None);
        // An absent file is not an owner either.
        assert_eq!(read_lease_pid(&dir.join("absent.lease")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- closed-vocabulary input classifier (task 9.1) --------------------

    /// The question text of an accepted question, or a panic naming what was
    /// classified instead — every question assertion below reads better this
    /// way than through a `match`.
    fn question_text(line: &str) -> String {
        match classify(line) {
            ChatInput::Question(q) => q.text().to_string(),
            other => panic!("{line:?} classified as {other:?}, expected a question"),
        }
    }

    #[test]
    fn the_vocabulary_is_the_documented_six_entries() {
        let names: Vec<&str> = VOCABULARY.iter().map(|e| e.name).collect();
        let want = ["/help", "/context", "/sessions", "/clear", "/resume", "/exit"];
        assert_eq!(names, want);
        // Exactly one entry may change workspace state (26.1).
        let acting: Vec<&str> = VOCABULARY
            .iter()
            .filter(|e| e.effect.changes_workspace_state())
            .map(|e| e.name)
            .collect();
        assert_eq!(acting, ["/resume"]);
        // `/resume` is the only entry taking an argument, and it is optional.
        let with_args: Vec<&str> = VOCABULARY
            .iter()
            .filter(|e| e.argument.is_some())
            .map(|e| e.name)
            .collect();
        assert_eq!(with_args, ["/resume"]);
        assert_eq!(VOCABULARY[4].usage(), "/resume [session-id]");
        assert_eq!(VOCABULARY[0].usage(), "/help");
        // Every entry carries help text; the listing has no blank cells.
        assert!(VOCABULARY.iter().all(|e| !e.help.is_empty()));
    }

    #[test]
    fn every_vocabulary_entry_classifies_to_its_own_variant() {
        assert_eq!(classify("/help"), ChatInput::Info(InfoCommand::Help));
        assert_eq!(classify("/context"), ChatInput::Info(InfoCommand::Context));
        assert_eq!(classify("/sessions"), ChatInput::Info(InfoCommand::Sessions));
        assert_eq!(classify("/clear"), ChatInput::Info(InfoCommand::Clear));
        assert_eq!(classify("/exit"), ChatInput::End);
        let no_target = ChatInput::Act(ActionCommand::Resume { target: None });
        assert_eq!(classify("/resume"), no_target);

        // Matching and the listing cannot drift: every name classifies to a
        // vocabulary entry, and that entry is the one carrying the name.
        for entry in VOCABULARY {
            let input = classify(entry.name);
            let found = input
                .vocabulary_entry()
                .unwrap_or_else(|| panic!("{} must classify to an entry", entry.name));
            assert_eq!(found.name, entry.name);
            assert_eq!(input.effect(), Some(entry.effect));
            let acts = entry.effect.changes_workspace_state();
            assert_eq!(input.changes_workspace_state(), acts, "{}", entry.name);
            // No vocabulary entry invokes the harness — only a question does.
            assert!(!input.invokes_harness(), "{} runs no harness", entry.name);
        }
    }

    #[test]
    fn whitespace_around_a_command_is_tolerated_and_nothing_else_is() {
        // Surrounding whitespace, including tabs and a trailing newline.
        assert_eq!(classify("  /exit  "), ChatInput::End);
        assert_eq!(classify("\t/help\n"), ChatInput::Info(InfoCommand::Help));
        assert_eq!(
            classify("  /resume   sess_0123456789abcdef  "),
            ChatInput::Act(ActionCommand::Resume {
                target: Some("sess_0123456789abcdef".to_string())
            })
        );
        // Whitespace *inside* the word is not whitespace around it.
        assert!(matches!(classify("/ exit"), ChatInput::Unknown { .. }));
        assert!(matches!(classify("/re sume"), ChatInput::Unknown { .. }));
    }

    #[test]
    fn resume_carries_its_optional_argument_without_validating_it() {
        // No argument: the target is resolved from the descriptor later (26.7).
        let no_target = ChatInput::Act(ActionCommand::Resume { target: None });
        assert_eq!(classify("/resume"), no_target);
        // One argument, carried verbatim — shape validation belongs to the
        // Resume_Action bridge (26.11), not here.
        for raw in [
            "sess_0123456789abcdef",
            "sess_not_hex_at_all",
            "nope",
            "SESS_0123456789ABCDEF",
            "`id`",
            "$(id)",
            "-rf",
            "/etc/passwd",
        ] {
            let carried = ChatInput::Act(ActionCommand::Resume { target: Some(raw.to_string()) });
            assert_eq!(classify(&format!("/resume {raw}")), carried, "{raw:?}");
        }
        // Control characters are stripped for safe printing; that cannot turn
        // a refused target into an accepted one.
        assert_eq!(
            classify("/resume sess_0123456789ab\u{0007}cdef"),
            ChatInput::Act(ActionCommand::Resume {
                target: Some("sess_0123456789abcdef".to_string())
            })
        );
        // Over-long arguments are bounded, and stay far too long to pass the
        // 21-character shape check.
        let long = "s".repeat(MAX_RESUME_TARGET_CHARS + 40);
        let t = match classify(&format!("/resume {long}")) {
            ChatInput::Act(ActionCommand::Resume { target }) => target.unwrap(),
            other => panic!("one argument is still the resume command: {other:?}"),
        };
        assert_eq!(t.chars().count(), MAX_RESUME_TARGET_CHARS);
        assert!(!crate::resume::is_session_id(&t));
        // More than one argument is not the resume command: no guessing which
        // word was meant.
        assert!(matches!(
            classify("/resume sess_0123456789abcdef extra"),
            ChatInput::Unknown { .. }
        ));
    }

    #[test]
    fn commands_match_exactly_and_never_loosely() {
        let unknown = [
            // Case: a loose match here would act on a typo.
            "/Resume",
            "/RESUME",
            "/Help",
            "/EXIT",
            // Prefixes and suffixes.
            "/res",
            "/resum",
            "/resumes",
            "/resume-now",
            "/exi",
            "/exits",
            "/helpme",
            "/clearall",
            "/context2",
            // Arguments where the entry accepts none.
            "/help me",
            "/exit please",
            "/clear all",
            "/context now",
            "/sessions all",
            // Not a command word at all.
            "/",
            "//resume",
            "/ ",
            // Shell-flavoured attempts: none of these is the resume command.
            "/resume;id",
            "/resume;/resume",
            "/resume|sh",
            "/resume&&id",
            "/resume$(id)",
            "/resume`id`",
            "/exit;rm -rf /",
        ];
        for line in unknown {
            let input = classify(line);
            assert!(
                matches!(input, ChatInput::Unknown { .. }),
                "{line:?} classified as {input:?}, expected an unknown command"
            );
            // 14.8: nothing is invoked and nothing can change.
            assert!(!input.changes_workspace_state());
            assert!(!input.invokes_harness());
            assert!(input.vocabulary_entry().is_none());
            assert!(input.effect().is_none());
        }
        // The refusal names what was entered, bounded.
        let entered = "/resumes".to_string();
        assert_eq!(classify("/resumes"), ChatInput::Unknown { entered });
        let huge = format!("/{}", "z".repeat(MAX_UNKNOWN_COMMAND_CHARS * 3));
        let ChatInput::Unknown { entered } = classify(&huge) else {
            panic!("an over-long word is an unknown command");
        };
        assert_eq!(entered.chars().count(), MAX_UNKNOWN_COMMAND_CHARS);
    }

    #[test]
    fn anything_not_starting_with_a_slash_is_a_question() {
        // Ordinary questions.
        assert_eq!(question_text("what is child1 doing?"), "what is child1 doing?");
        // A question that talks *about* a command is still a question: wording
        // never becomes behaviour (27.3).
        assert_eq!(question_text("should I /resume child1?"), "should I /resume child1?");
        assert_eq!(question_text("resume child1"), "resume child1");
        assert_eq!(question_text("exit"), "exit");
        // Slashes mid-text: paths and dates are not commands.
        assert_eq!(question_text("does src/main.rs matter?"), "does src/main.rs matter?");
        assert_eq!(question_text("what changed on 12/03?"), "what changed on 12/03?");
        // A line whose slash is hidden behind a control character is a
        // question, because the prefix test runs before control stripping.
        assert_eq!(question_text("\u{1b}/exit"), "/exit");
        assert_eq!(
            question_text("\u{0007}/resume sess_0123456789abcdef"),
            "/resume sess_0123456789abcdef"
        );
        // Leading whitespace does not change the class.
        assert_eq!(question_text("   how busy is the workspace?  "), "how busy is the workspace?");
        // Only a question reaches the harness, and it changes nothing.
        let q = classify("what is child1 doing?");
        assert!(q.invokes_harness());
        assert!(!q.changes_workspace_state());
        assert!(q.vocabulary_entry().is_none());
    }

    #[test]
    fn blank_and_control_only_input_is_a_skipped_no_op() {
        for line in ["", " ", "   \t ", "\n", "\r\n", "\u{007f}", "\u{1b}\u{0007}"] {
            let input = classify(line);
            assert_eq!(input, ChatInput::Blank, "{line:?} must be a no-op");
            assert!(!input.changes_workspace_state());
            assert!(!input.invokes_harness());
        }
    }

    #[test]
    fn question_text_is_control_stripped_and_scrubbed() {
        // Controls vanish; surrounding whitespace they expose is trimmed.
        assert_eq!(question_text("what \u{0007}now?"), "what now?");
        assert_eq!(question_text("tail\u{001b}[2J  "), "tail[2J");
        // Tabs are content, not escapes: `strip_controls` keeps them.
        assert_eq!(question_text("a\tb"), "a\tb");
        // The existing scrubber is applied to the question text (15.5); no
        // new sanitiser exists here.
        let scrubbed = question_text("is token=abc123def456 still valid?");
        assert!(scrubbed.contains("[redacted]"), "{scrubbed}");
        assert!(!scrubbed.contains("abc123def456"), "{scrubbed}");
        // Token-shaped literals too. Assembled at runtime so the repository
        // never carries a credential-shaped string (see the CI secret scan).
        let token = ["ghp", "0123456789abcdefghij"].join("_");
        let pat = question_text(&format!("does {token} still work?"));
        assert!(pat.contains("[redacted]"), "{pat}");
        assert!(!pat.contains(&token), "{pat}");
        // Ordinary words survive: over-redaction would destroy the question.
        assert_eq!(question_text("nothing secret here"), "nothing secret here");
    }

    #[test]
    fn an_over_cap_question_is_refused_never_truncated() {
        // Exactly at the cap is accepted.
        let at_cap = "a".repeat(MAX_CHAT_QUESTION_CHARS);
        let ChatInput::Question(q) = classify(&at_cap) else {
            panic!("a question at the cap must be accepted");
        };
        assert_eq!(q.chars(), MAX_CHAT_QUESTION_CHARS);
        assert_eq!(q.text(), at_cap);

        // One character over refuses, naming the length that was entered —
        // following `assign`'s precedent, because truncating changes meaning.
        let over = ChatInput::QuestionTooLong { chars: MAX_CHAT_QUESTION_CHARS + 1 };
        assert_eq!(classify(&"a".repeat(MAX_CHAT_QUESTION_CHARS + 1)), over);
        // Counted in characters, not bytes: a 2000-character multi-byte
        // question is 4000 bytes and still accepted.
        let wide = "\u{00E9}".repeat(MAX_CHAT_QUESTION_CHARS);
        assert_eq!(wide.len(), MAX_CHAT_QUESTION_CHARS * 2);
        assert!(matches!(classify(&wide), ChatInput::Question(_)));

        // Redaction lengthens text, so the cap is re-checked after scrubbing:
        // what travels into argv is the scrubbed string, so it is the scrubbed
        // string that must fit.
        let secrets = "token=x ".repeat(250);
        assert_eq!(secrets.chars().count(), MAX_CHAT_QUESTION_CHARS);
        match classify(&secrets) {
            ChatInput::QuestionTooLong { chars } => {
                assert!(chars > MAX_CHAT_QUESTION_CHARS, "{chars}")
            }
            other => panic!("expected a refusal after redaction, got {other:?}"),
        }

        // A refused question is still not a command and still invokes nothing.
        let refused = classify(&"a".repeat(MAX_CHAT_QUESTION_CHARS + 1));
        assert!(!refused.changes_workspace_state());
        assert!(!refused.invokes_harness());
        assert!(refused.vocabulary_entry().is_none());
    }

    #[test]
    fn only_an_entered_resume_command_can_change_workspace_state() {
        // A representative spread of inputs: exactly the `/resume` forms are
        // acting, and every acting classification is `Act` (14.10, 26.3).
        let acting = ["/resume", "/resume sess_0123456789abcdef", "  /resume  "];
        let too_long = "a".repeat(MAX_CHAT_QUESTION_CHARS + 1);
        let inert = [
            "",
            "   ",
            "/help",
            "/context",
            "/sessions",
            "/clear",
            "/exit",
            "/Resume",
            "/resumes",
            "/resume a b",
            "/unknown",
            "resume",
            "resume child1",
            "please resume sess_0123456789abcdef",
            "run /resume for me",
            "\u{1b}/resume",
            too_long.as_str(),
        ];
        for line in acting {
            let input = classify(line);
            assert!(
                matches!(input, ChatInput::Act(ActionCommand::Resume { .. })),
                "{line:?} must be the acting command, got {input:?}"
            );
            assert!(input.changes_workspace_state());
            // Even the acting command runs no harness: it is not a question.
            assert!(!input.invokes_harness());
        }
        for line in inert {
            let input = classify(line);
            assert!(
                !input.changes_workspace_state(),
                "{line:?} must not be able to change workspace state, got {input:?}"
            );
            assert!(!matches!(input, ChatInput::Act(_)));
        }
    }

    #[test]
    fn the_listing_marks_read_only_and_acting_entries() {
        let listing = render_vocabulary();
        // Every entry appears in its typed form with its help text (14.8).
        for entry in VOCABULARY {
            assert!(listing.contains(&entry.usage()), "{listing}");
            assert!(listing.contains(entry.help), "{listing}");
        }
        // 27.8: informational and end entries read-only, `/resume` marked as
        // changing workspace state — on their own lines, so a reader cannot
        // attach the wrong mark to the wrong entry.
        for line in listing.lines() {
            for entry in VOCABULARY {
                if line.trim_start().starts_with(entry.name) {
                    assert!(
                        line.contains(entry.effect.marker()),
                        "{:?} must be marked {:?}",
                        entry.name,
                        entry.effect.marker()
                    );
                }
            }
        }
        assert!(listing.contains("read-only"));
        assert!(listing.contains("CHANGES WORKSPACE STATE"));
        let resume_line = listing
            .lines()
            .find(|l| l.trim_start().starts_with("/resume"))
            .expect("the acting entry is listed");
        assert!(resume_line.contains("CHANGES WORKSPACE STATE"), "{resume_line}");
        assert!(!resume_line.contains("read-only"), "{resume_line}");
        // The listing also states what a non-command input does (27.5).
        assert!(listing.contains("question"), "{listing}");
    }

    #[test]
    fn an_unknown_command_prints_the_whole_vocabulary_and_ran_nothing() {
        let ChatInput::Unknown { entered } = classify("/nope --now") else {
            panic!("an unknown command");
        };
        assert_eq!(entered, "/nope");
        let msg = unknown_command_message(&entered);
        assert!(msg.contains("'/nope'"), "{msg}");
        assert!(msg.contains("nothing was run"), "{msg}");
        assert!(msg.ends_with(&render_vocabulary()), "{msg}");
        for entry in VOCABULARY {
            assert!(msg.contains(entry.name), "{msg}");
        }
    }
}

#[cfg(test)]
mod responder_tests {
    //! Chat_Responder (task 9.2). Every test here runs a *fake* harness from
    //! a temp directory and stages its context in a sandbox runtime
    //! directory: no real agent is invoked, and the production runtime dir is
    //! never touched.

    use super::*;
    use crate::platform::{ChatLease, RawProcess};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn sandbox(tag: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("pitwall-m8-resp-{tag}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A harness stand-in: an executable script under a directory that plays
    /// the role of a `PATH` entry, named exactly as `agents::KNOWN` expects.
    fn fake_harness(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join(name);
        std::fs::write(&bin, body).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    /// A platform that observes nothing and refuses to do anything.
    ///
    /// `launch_terminal` and `focus_window_address` panic on purpose: the
    /// observation side is allowed to hold a `Platform`, but nothing on the
    /// question path may ever use its acting capabilities (27.1, 27.2).
    struct QuietPlatform;

    impl crate::platform::Platform for QuietPlatform {
        fn processes(&self) -> Vec<RawProcess> {
            Vec::new()
        }
        fn windows(&self) -> Vec<crate::platform::WindowInfo> {
            Vec::new()
        }
        fn git_info(&self, _dir: &str) -> crate::platform::GitInfo {
            crate::platform::GitInfo::default()
        }
        fn boot_epoch(&self) -> i64 {
            1_700_000_000
        }
        fn clock_ticks_per_sec(&self) -> i64 {
            100
        }
        fn hostname(&self) -> String {
            "testbox".to_string()
        }
        fn launch_terminal(&self, _spec: &crate::platform::TerminalSpec<'_>) -> Result<(), String> {
            panic!("the question path must never launch a terminal");
        }
        fn chat_leases(&self) -> Vec<ChatLease> {
            Vec::new()
        }
        fn inline_image_capability(&self) -> crate::platform::InlineImage {
            crate::platform::InlineImage::None
        }
        fn focus_window_address(&self, _address: &str) -> Result<(), String> {
            panic!("the question path must never focus a window");
        }
        fn process_io(&self, _pid: u32) -> Option<crate::platform::IoCounters> {
            None
        }
        fn terminal_text(&self, _pid: u32, _class: &str) -> crate::platform::TerminalText {
            crate::platform::TerminalText::Unavailable {
                reason: "mock has no terminals",
            }
        }
    }

    fn descriptor(harness: &str, model: &str, project_dir: &Path) -> ChatDescriptor {
        ChatDescriptor::capture(
            9,
            harness,
            model,
            "Work",
            None,
            1_700_000_100,
            &project_dir.to_string_lossy(),
        )
        .unwrap()
    }

    fn question(line: &str) -> ChatQuestion {
        match classify(line) {
            ChatInput::Question(q) => q,
            other => panic!("{line:?} should classify as a question, got {other:?}"),
        }
    }

    /// A real observation from a platform that sees nothing: the store path
    /// cannot exist, so every store read degrades to empty and no database is
    /// created (15.6).
    fn quiet_observation(d: &ChatDescriptor) -> ChatObservation {
        observe(
            &QuietPlatform,
            d,
            Path::new("/nonexistent/pitwall-chat-responder"),
        )
    }

    /// Does any argv element carry `needle`?
    fn argv_carries(call: &HarnessCall, needle: &str) -> bool {
        call.argv().iter().any(|a| a.contains(needle))
    }

    /// Ephemeral context documents left behind in a runtime directory.
    fn staged_contexts(runtime_dir: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(runtime_dir) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("ctx-"))
            .collect()
    }

    #[test]
    fn the_instruction_states_the_answer_discipline() {
        // 13.1-13.4 and the observe-only boundary, in one fixed string.
        let i = CHAT_INSTRUCTION;
        assert!(i.contains("bounded Pitwall workspace context"), "{i}");
        assert!(i.contains("unavailable"), "{i}");
        assert!(i.contains("activity evidence"), "{i}");
        assert!(i.contains("confidence"), "{i}");
        assert!(i.contains("do not execute tasks"), "{i}");
        assert!(i.contains("do not continue the work"), "{i}");
        // Nothing in it invites an action.
        assert!(!i.contains("resume it"), "{i}");
    }

    #[test]
    fn per_harness_argv_matches_the_design_table() {
        let dir = sandbox("argv");
        let ctx_path = dir.join("ctx-1234-0123456789abcdef.json");
        let ctx = ctx_path.as_path();
        let ctx_text = ctx_path.to_string_lossy().into_owned();
        let msg = chat_message(&question("what is running?"));
        let bin = Path::new("/opt/bin/harness");

        // opencode: run --format json --dir DIR -f CTX -m MODEL <MSG>.
        let oc = descriptor("opencode", "prov/model", &dir);
        let call = build_chat_call(&oc, bin, ctx, &msg).unwrap();
        let want = vec![
            "/opt/bin/harness".to_string(),
            "run".to_string(),
            "--format".to_string(),
            "json".to_string(),
            "--dir".to_string(),
            oc.project_dir().to_string(),
            "-f".to_string(),
            ctx_text.clone(),
            "-m".to_string(),
            "prov/model".to_string(),
            msg.clone(),
        ];
        assert_eq!(call.argv().to_vec(), want);
        assert!(!call.context_on_stdin(), "opencode takes the file");

        // An empty model means the harness default: no `-m` at all (16.3).
        let oc_default = descriptor("opencode", "", &dir);
        let call = build_chat_call(&oc_default, bin, ctx, &msg).unwrap();
        assert!(!call.argv().iter().any(|a| a == "-m"), "{:?}", call.argv());
        assert_eq!(call.argv().last().unwrap(), &msg);

        // claude: -p <MSG>, document on stdin, context path nowhere.
        let cl = descriptor("claude", "prov/model", &dir);
        let call = build_chat_call(&cl, bin, ctx, &msg).unwrap();
        let want = vec!["/opt/bin/harness".to_string(), "-p".to_string(), msg.clone()];
        assert_eq!(call.argv().to_vec(), want);
        assert!(call.context_on_stdin());

        // codex: exec <MSG>, same rules.
        let cx = descriptor("codex", "", &dir);
        let call = build_chat_call(&cx, bin, ctx, &msg).unwrap();
        let want = vec!["/opt/bin/harness".to_string(), "exec".to_string(), msg.clone()];
        assert_eq!(call.argv().to_vec(), want);
        assert!(call.context_on_stdin());

        // Neither stdin harness leaks the staged path into argv (14.4).
        for harness in ["claude", "codex"] {
            let d = descriptor(harness, "", &dir);
            let call = build_chat_call(&d, bin, ctx, &msg).unwrap();
            assert!(!argv_carries(&call, &ctx_text), "{harness}");
            assert!(!argv_carries(&call, "ctx-"), "{harness}");
        }

        // No shell anywhere, for any harness.
        for harness in ["opencode", "claude", "codex"] {
            let d = descriptor(harness, "", &dir);
            let call = build_chat_call(&d, bin, ctx, &msg).unwrap();
            assert!(!argv_carries(&call, "sh -c"), "{harness}");
            assert!(!argv_carries(&call, "bash -c"), "{harness}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_question_travels_in_exactly_one_argv_element() {
        let dir = sandbox("one-element");
        // Flag-shaped, path-shaped and shell-shaped text, all inside one
        // question: none of it may become argv structure (14.2, 14.4).
        let hostile = "--dir /etc; rm -rf ~ && -m other/model `id`";
        let q = question(hostile);
        let msg = chat_message(&q);
        let bin = Path::new("/opt/bin/harness");
        let ctx = Path::new("/run/user/0/pitwall/ctx-1-a.json");
        for harness in ["opencode", "claude", "codex"] {
            let d = descriptor(harness, "", &dir);
            let call = build_chat_call(&d, bin, ctx, &msg).unwrap();
            let carrying = call.argv().iter().filter(|a| a.contains("rm -rf ~")).count();
            assert_eq!(carrying, 1, "{harness}: {:?}", call.argv());
            // And that one element is the message, in its trailing position.
            assert_eq!(call.argv().last().unwrap(), &msg);
            // The instruction rides in the same element, so the harness can
            // never receive the question without its rules.
            assert!(msg.starts_with(CHAT_INSTRUCTION));
            assert!(msg.contains(q.text()));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_answer_is_text_only_scrubbed_and_leaves_no_context_behind() {
        let dir = sandbox("answer");
        let bin_dir = dir.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let runtime = dir.join("run");
        // A tool-call event, a metadata event, and one text event whose text
        // carries an OSC 2 sequence: the payloads must be dropped and the
        // escape bytes stripped before anything is presented.
        fake_harness(
            &bin_dir,
            "opencode",
            concat!(
                "#!/bin/sh\n",
                "printf '%s\\n' '{\"type\":\"tool\",\"name\":\"bash\",\"input\":\"rm -rf /\"}'\n",
                "printf '%s\\n' '{\"type\":\"meta\",\"tokens\":42}'\n",
                "printf '%s\\n' '{\"type\":\"text\",\"text\":\"Idle. \\u001b]2;x\\u0007\"}'\n",
            ),
        );
        let d = descriptor("opencode", "", &dir);
        let obs = quiet_observation(&d);
        let answer = respond_in(
            &d,
            &obs,
            &question("what is running?"),
            &runtime,
            &[bin_dir.clone()],
            Duration::from_secs(10),
        )
        .unwrap();

        assert!(answer.starts_with("Idle."), "{answer:?}");
        // 13.7: tool-call and metadata payloads never reach the human.
        assert!(!answer.contains("rm -rf"), "{answer:?}");
        assert!(!answer.contains("tokens"), "{answer:?}");
        // 15.5 / 16.2: no escape bytes survive, so an answer cannot retitle
        // the window whose title is half of this chat's identity.
        assert!(!answer.contains('\u{1b}'), "{answer:?}");
        assert!(!answer.contains('\u{7}'), "{answer:?}");
        // 12.8 / 15.7: the staged context is gone.
        assert!(staged_contexts(&runtime).is_empty(), "{:?}", staged_contexts(&runtime));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_document_reaches_a_stdin_harness_and_never_argv() {
        let dir = sandbox("stdin");
        let bin_dir = dir.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let runtime = dir.join("run");
        // Reads stdin to EOF and reports whether the bounded document arrived.
        fake_harness(
            &bin_dir,
            "claude",
            concat!(
                "#!/bin/sh\n",
                "doc=$(cat)\n",
                "case \"$doc\" in\n",
                "  *sessions*) printf '%s\\n' '{\"type\":\"text\",\"text\":\"got ctx\"}' ;;\n",
                "  *) printf '%s\\n' '{\"type\":\"text\",\"text\":\"no ctx\"}' ;;\n",
                "esac\n",
            ),
        );
        let d = descriptor("claude", "", &dir);
        let obs = quiet_observation(&d);
        assert!(obs.document().contains("sessions"), "{}", obs.document());

        // The document is not in the argument vector (14.4, 15.1-15.4).
        let call = build_chat_call(
            &d,
            &bin_dir.join("claude"),
            Path::new("/run/user/0/pitwall/ctx-1-a.json"),
            &chat_message(&question("what is running?")),
        )
        .unwrap();
        assert!(
            !call.argv().iter().any(|a| a.contains("\"sessions\"")),
            "{:?}",
            call.argv()
        );

        let answer = respond_in(
            &d,
            &obs,
            &question("what is running?"),
            &runtime,
            &[bin_dir.clone()],
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(answer, "got ctx");
        assert!(staged_contexts(&runtime).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_text_is_one_short_failure_line_and_the_chat_stays_ready() {
        let dir = sandbox("empty");
        let bin_dir = dir.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let runtime = dir.join("run");
        // Exits 0 with nothing but a tool call: after extraction there is no
        // text at all (13.5).
        fake_harness(
            &bin_dir,
            "opencode",
            concat!(
                "#!/bin/sh\n",
                "printf '%s\\n' '{\"type\":\"tool\",\"input\":\"tool payload\"}'\n",
            ),
        );
        let d = descriptor("opencode", "", &dir);
        let obs = quiet_observation(&d);
        let err = respond_in(
            &d,
            &obs,
            &question("what is running?"),
            &runtime,
            &[bin_dir.clone()],
            Duration::from_secs(10),
        )
        .unwrap_err();

        assert_eq!(err, ChatError::NoAnswer);
        assert_eq!(err.class(), "no answer");
        let line = err.message();
        assert!(line.contains("ready for the next input"), "{line}");
        // No internals: not the payload, not the binary, not the staged path.
        assert!(!line.contains("tool payload"), "{line}");
        assert!(!line.contains("opencode"), "{line}");
        assert!(!line.contains("ctx-"), "{line}");
        assert!(staged_contexts(&runtime).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_spawn_failure_is_one_short_failure_line_and_the_chat_stays_ready() {
        let dir = sandbox("spawn");
        let bin_dir = dir.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let runtime = dir.join("run");
        // Executable and discovered, but its interpreter does not exist, so
        // `exec` fails and the harness never runs. (A plain text file would
        // not do: `execvp` falls back to `/bin/sh` on `ENOEXEC`, which would
        // exercise the exit-status path instead of the spawn path.)
        fake_harness(
            &bin_dir,
            "opencode",
            "#!/nonexistent/pitwall-no-such-interpreter\n",
        );
        let d = descriptor("opencode", "", &dir);
        let obs = quiet_observation(&d);
        let err = respond_in(
            &d,
            &obs,
            &question("what is running?"),
            &runtime,
            &[bin_dir.clone()],
            Duration::from_secs(10),
        )
        .unwrap_err();

        assert_eq!(err, ChatError::HarnessUnavailable);
        assert_eq!(err.class(), "harness unavailable");
        assert!(err.message().contains("ready for the next input"));
        assert!(!err.message().contains("Exec format"), "{err}");
        assert!(staged_contexts(&runtime).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_timeout_kills_the_harness_and_the_chat_stays_ready() {
        let dir = sandbox("timeout");
        let bin_dir = dir.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let runtime = dir.join("run");
        fake_harness(&bin_dir, "opencode", "#!/bin/sh\nsleep 30\n");
        let d = descriptor("opencode", "", &dir);
        let obs = quiet_observation(&d);
        let err = respond_in(
            &d,
            &obs,
            &question("what is running?"),
            &runtime,
            &[bin_dir.clone()],
            Duration::from_secs(1),
        )
        .unwrap_err();

        // 13.6: killed, one timeout line, still ready.
        assert_eq!(err, ChatError::TimedOut { secs: 1 });
        assert_eq!(err.class(), "timeout");
        let line = err.message();
        assert!(line.contains("1s"), "{line}");
        assert!(line.contains("ready for the next input"), "{line}");
        // The kill path cleans up too.
        assert!(staged_contexts(&runtime).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_harness_refuses_before_staging_anything() {
        let dir = sandbox("missing");
        let bin_dir = dir.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let runtime = dir.join("run");
        let d = descriptor("opencode", "", &dir);
        let obs = quiet_observation(&d);
        let err = respond_in(
            &d,
            &obs,
            &question("what is running?"),
            &runtime,
            &[bin_dir.clone()],
            Duration::from_secs(10),
        )
        .unwrap_err();

        assert_eq!(
            err,
            ChatError::HarnessNotInstalled {
                harness: "opencode".to_string()
            }
        );
        assert!(err.message().contains("ready for the next input"));
        // Nothing was staged, so the runtime dir was never even created.
        assert!(staged_contexts(&runtime).is_empty());
        assert!(!runtime.exists(), "refusal must not create the runtime dir");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_known_harness_has_a_recorded_private_delivery_channel() {
        // The table and the harness set agree today, so nothing is refused.
        for agent in crate::agents::KNOWN {
            assert!(
                delivery_for(agent.id).is_some(),
                "{} has no recorded delivery channel",
                agent.id
            );
        }
        // A harness outside the table is refused, and the refusal says why
        // without offering argv as a fallback (§9.3).
        assert!(delivery_for("gemini").is_none());
        let err = ChatError::DeliveryUnverified {
            harness: "gemini".to_string(),
        };
        let line = err.message();
        assert!(line.contains("gemini"), "{line}");
        assert!(line.contains("no verified private channel"), "{line}");
        assert!(line.contains("command line"), "{line}");
        assert!(line.contains("ready for the next input"), "{line}");
    }

    #[test]
    fn presented_answers_are_control_stripped_and_scrubbed() {
        // Built at runtime so the literal never looks like a real secret.
        let secret = format!("{}={}", "token", "AbCd1234EfGh");
        let raw = format!("state ok \u{1b}]2;stolen\u{7} {secret}");
        let shown = present_answer(&raw);
        assert!(shown.starts_with("state ok"), "{shown}");
        assert!(!shown.contains('\u{1b}'), "{shown}");
        assert!(shown.contains("token=[redacted]"), "{shown}");
        assert!(!shown.contains("AbCd1234EfGh"), "{shown}");
        // Nothing usable left ⇒ empty, which the responder turns into the
        // "no answer" class rather than presenting a blank line.
        assert_eq!(present_answer("   \u{1b}\u{7}  "), "");
    }

    // --- scoping (12.3, 12.4) ---------------------------------------------

    fn session(id: &str, project: Option<(&str, &str)>) -> crate::collector::TerminalSession {
        crate::collector::TerminalSession {
            id: id.to_string(),
            window: None,
            root_pid: 100,
            role: crate::collector::WindowRole::Terminal,
            project: project.map(|(pid, dir)| crate::collector::ProjectInfo {
                id: pid.to_string(),
                dir: dir.to_string(),
                name: dir.rsplit('/').next().unwrap_or("proj").to_string(),
                is_git_repo: false,
                branch: None,
                git_clean: None,
            }),
            agent: crate::collector::AgentIdentity {
                kind: crate::collector::AgentKind::Unknown,
                confidence: crate::collector::Confidence::Low,
                evidence: Vec::new(),
            },
            chat: None,
            state: crate::collector::SessionState::Sleeping,
            process_count: 1,
            processes: Vec::new(),
            last_activity_epoch: 1_700_000_050,
            last_activity_kind: crate::collector::LAST_ACTIVITY_KIND,
            summary: "one session".to_string(),
        }
    }

    fn snapshot(
        sessions: Vec<crate::collector::TerminalSession>,
    ) -> crate::collector::WorkspaceSnapshot {
        crate::collector::WorkspaceSnapshot {
            schema_version: 1,
            collected_at_epoch: 1_700_000_100,
            hostname: "testbox".to_string(),
            sessions,
        }
    }

    fn checkpoint(id: i64, session_id: &str, project_id: &str) -> crate::store::Checkpoint {
        crate::store::Checkpoint {
            id,
            created_at: 1_700_000_000 + id,
            project_id: project_id.to_string(),
            session_id: session_id.to_string(),
            project_dir: "/home/u/Work".to_string(),
            branch: None,
            git_clean: None,
            agent_kind: "unknown".to_string(),
            agent_confidence: "low".to_string(),
            state: "idle".to_string(),
            last_activity_epoch: 1_700_000_050,
            window_address: None,
            window_class: None,
            note: None,
            trigger: "manual".to_string(),
            observation_id: None,
        }
    }

    fn notification(id: i64, session_id: &str, project_id: &str) -> crate::store::Notification {
        crate::store::Notification {
            id,
            kind: "attention".to_string(),
            session_id: session_id.to_string(),
            project_id: project_id.to_string(),
            project_name: "Work".to_string(),
            branch: None,
            agent_kind: "unknown".to_string(),
            state: "idle".to_string(),
            checkpoint_id: None,
            created_at: 1_700_000_000 + id,
            severity: "info".to_string(),
            detail: "idle".to_string(),
            read_at: None,
        }
    }

    #[test]
    fn no_context_session_scopes_to_the_whole_workspace() {
        // 12.4: nothing is filtered out.
        let scope = scope_to_context(
            None,
            snapshot(vec![
                session("sess_aaaaaaaaaaaaaaaa", Some(("proj_a", "/home/u/A"))),
                session("sess_bbbbbbbbbbbbbbbb", Some(("proj_b", "/home/u/B"))),
            ]),
            Vec::new(),
            Vec::new(),
            vec![
                checkpoint(1, "sess_aaaaaaaaaaaaaaaa", "proj_a"),
                checkpoint(2, "sess_bbbbbbbbbbbbbbbb", "proj_b"),
            ],
            vec![notification(1, "sess_bbbbbbbbbbbbbbbb", "proj_b")],
        );
        assert_eq!(scope.snapshot().sessions.len(), 2);
        assert_eq!(scope.checkpoints().len(), 2);
        assert_eq!(scope.notifications().len(), 1);
    }

    #[test]
    fn a_context_session_scopes_to_that_session_and_its_project() {
        // 12.3: the scoped session, its project's other session, and the
        // rows belonging to either — and nothing from another project.
        let scoped = "sess_aaaaaaaaaaaaaaaa";
        let scope = scope_to_context(
            Some(scoped),
            snapshot(vec![
                session(scoped, Some(("proj_a", "/home/u/A"))),
                session("sess_cccccccccccccccc", Some(("proj_a", "/home/u/A"))),
                session("sess_bbbbbbbbbbbbbbbb", Some(("proj_b", "/home/u/B"))),
            ]),
            vec![
                crate::store::PrevSession {
                    session_id: "sess_cccccccccccccccc".to_string(),
                    project_id: Some("proj_a".to_string()),
                    project_dir: Some("/home/u/A".to_string()),
                    agent_kind: "unknown".to_string(),
                    branch: None,
                    git_clean: None,
                    state: "idle".to_string(),
                },
                crate::store::PrevSession {
                    session_id: "sess_bbbbbbbbbbbbbbbb".to_string(),
                    project_id: Some("proj_b".to_string()),
                    project_dir: Some("/home/u/B".to_string()),
                    agent_kind: "unknown".to_string(),
                    branch: None,
                    git_clean: None,
                    state: "idle".to_string(),
                },
            ],
            vec![checkpoint(3, "sess_bbbbbbbbbbbbbbbb", "proj_b")],
            vec![
                checkpoint(1, scoped, "proj_a"),
                checkpoint(2, "sess_bbbbbbbbbbbbbbbb", "proj_b"),
            ],
            vec![
                notification(1, scoped, "proj_a"),
                notification(2, "sess_bbbbbbbbbbbbbbbb", "proj_b"),
            ],
        );

        let ids: Vec<&str> = scope
            .snapshot()
            .sessions
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(ids, vec![scoped, "sess_cccccccccccccccc"]);
        assert_eq!(scope.prev_sessions().len(), 1);
        assert_eq!(scope.prev_sessions()[0].session_id, "sess_cccccccccccccccc");
        assert_eq!(scope.checkpoints().len(), 1);
        assert_eq!(scope.checkpoints()[0].session_id, scoped);
        assert_eq!(scope.notifications().len(), 1);
        assert_eq!(scope.notifications()[0].session_id, scoped);
    }

    #[test]
    fn a_vanished_context_session_keeps_only_its_own_rows() {
        // The scope's subject ended: no live session is in scope, and the
        // context does not silently widen to the whole workspace.
        let scoped = "sess_aaaaaaaaaaaaaaaa";
        let scope = scope_to_context(
            Some(scoped),
            snapshot(vec![session(
                "sess_bbbbbbbbbbbbbbbb",
                Some(("proj_b", "/home/u/B")),
            )]),
            vec![crate::store::PrevSession {
                session_id: scoped.to_string(),
                project_id: Some("proj_a".to_string()),
                project_dir: Some("/home/u/A".to_string()),
                agent_kind: "unknown".to_string(),
                branch: None,
                git_clean: None,
                state: "idle".to_string(),
            }],
            Vec::new(),
            vec![
                checkpoint(1, scoped, "proj_a"),
                checkpoint(2, "sess_bbbbbbbbbbbbbbbb", "proj_b"),
            ],
            vec![notification(2, "sess_bbbbbbbbbbbbbbbb", "proj_b")],
        );
        assert!(scope.snapshot().sessions.is_empty());
        assert_eq!(scope.prev_sessions().len(), 1);
        assert_eq!(scope.checkpoints().len(), 1);
        assert_eq!(scope.checkpoints()[0].session_id, scoped);
        assert!(scope.notifications().is_empty());
    }

    #[test]
    fn the_context_stays_inside_the_existing_bounds() {
        let dir = sandbox("bounds");
        let d = descriptor("opencode", "", &dir);
        let obs = quiet_observation(&d);
        // 12.2: the same 16 KB document bound the summary path enforces, and
        // the renderer's honest truncation count.
        assert!(
            obs.document().len() <= crate::context::MAX_CONTEXT_BYTES,
            "len={}",
            obs.document().len()
        );
        assert_eq!(obs.truncated_sessions(), 0);
        // 12.9: the document is the bounded context document, not a snapshot.
        assert!(obs.document().starts_with("{\"sessions\":["));
        // The caps chat applies are the caps `cmd_summarize` applies.
        assert_eq!(MAX_CHAT_EVENTS, 20);
        assert_eq!(MAX_CHAT_CHECKPOINTS, 10);
        assert_eq!(crate::context::MAX_SESSIONS, 6);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod resume_tests {
    //! Resume_Action bridge (task 9.4).
    //!
    //! Every test here drives a **recording** platform, following
    //! `resume.rs`'s `MockPlatform` pattern: each acting capability the
    //! `Platform` trait exposes is captured, so "no executor call happened"
    //! and "exactly one terminal was opened" are asserted mechanically
    //! rather than argued (26.9, 26.12, 26.14).
    //!
    //! No test touches a real compositor, a real terminal or a real agent.

    use super::*;
    use crate::platform::{
        ChatLease, GitInfo, InlineImage, Platform, RawProcess, TerminalSpec, WindowInfo,
    };
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// One recorded terminal launch: where, and what was to run inside it.
    ///
    /// The command is recorded rather than asserted away, because "no agent
    /// was started" is exactly the claim that this vector stayed empty
    /// (26.20).
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Launch {
        directory: String,
        command: Vec<String>,
    }

    /// The recording platform. Mirrors `resume.rs`'s mock, including the fact
    /// that `windows()` is derived from a session list so `collector::collect`
    /// produces real `sess_*` ids.
    struct MockPlatform {
        sessions: Vec<(String, String)>,
        launched: std::cell::RefCell<Vec<Launch>>,
        focused: std::cell::RefCell<Vec<String>>,
        fail_launch: bool,
        fail_focus: bool,
    }

    impl MockPlatform {
        fn with(sessions: Vec<(String, String)>) -> Self {
            MockPlatform {
                sessions,
                launched: std::cell::RefCell::new(Vec::new()),
                focused: std::cell::RefCell::new(Vec::new()),
                fail_launch: false,
                fail_focus: false,
            }
        }

        /// Nothing observed and nothing to act on: the platform a refusal
        /// test uses, where any recorded action is a failure.
        fn quiet() -> Self {
            MockPlatform::with(Vec::new())
        }

        /// Did the executor reach the platform at all?
        fn acted(&self) -> bool {
            !self.launched.borrow().is_empty() || !self.focused.borrow().is_empty()
        }
    }

    impl Platform for MockPlatform {
        fn processes(&self) -> Vec<RawProcess> {
            Vec::new()
        }
        fn windows(&self) -> Vec<WindowInfo> {
            self.sessions
                .iter()
                .enumerate()
                .map(|(n, _)| WindowInfo {
                    address: format!("0x{}", n + 1),
                    class: "foot".to_string(),
                    initial_class: "foot".to_string(),
                    title: "t".to_string(),
                    workspace: "1".to_string(),
                    pid: 100 + n as u32,
                })
                .collect()
        }
        fn git_info(&self, _dir: &str) -> GitInfo {
            GitInfo::default()
        }
        fn boot_epoch(&self) -> i64 {
            1_700_000_000
        }
        fn clock_ticks_per_sec(&self) -> i64 {
            100
        }
        fn hostname(&self) -> String {
            "testbox".to_string()
        }
        fn launch_terminal(&self, spec: &TerminalSpec<'_>) -> Result<(), String> {
            if self.fail_launch {
                return Err("boom".to_string());
            }
            self.launched.borrow_mut().push(Launch {
                directory: spec.directory.to_string(),
                command: spec.command.to_vec(),
            });
            Ok(())
        }
        fn chat_leases(&self) -> Vec<ChatLease> {
            Vec::new()
        }
        fn inline_image_capability(&self) -> InlineImage {
            InlineImage::None
        }
        fn focus_window_address(&self, address: &str) -> Result<(), String> {
            if self.fail_focus {
                return Err("gone".to_string());
            }
            self.focused.borrow_mut().push(address.to_string());
            Ok(())
        }
        fn process_io(&self, _pid: u32) -> Option<crate::platform::IoCounters> {
            None
        }
        fn terminal_text(&self, _pid: u32, _class: &str) -> crate::platform::TerminalText {
            crate::platform::TerminalText::Unavailable {
                reason: "mock has no terminals",
            }
        }
    }

    /// A private directory tree per test, plus the database path inside it.
    fn sandbox(tag: &str) -> (PathBuf, PathBuf) {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("pitwall-m8-resume-{tag}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join(crate::store::DB_FILENAME);
        (dir, db)
    }

    /// A chat scoped to one context session.
    fn scoped_descriptor(dir: &Path, context: &str) -> ChatDescriptor {
        ChatDescriptor::capture(
            4,
            "opencode",
            "",
            "Work",
            Some(context),
            1_700_000_100,
            &dir.to_string_lossy(),
        )
        .unwrap()
    }

    /// A chat scoped to the whole workspace: no context session id, so
    /// `/resume` alone has no target (26.9).
    fn workspace_descriptor(dir: &Path) -> ChatDescriptor {
        ChatDescriptor::capture(
            4,
            "opencode",
            "",
            WORKSPACE_LABEL,
            None,
            1_700_000_100,
            &dir.to_string_lossy(),
        )
        .unwrap()
    }

    /// One checkpoint row, exactly as `resume.rs`'s tests write it.
    fn checkpoint_row(store: &mut crate::store::Store, session_id: &str, project_dir: &str) {
        let now = 1_700_000_200;
        store
            .insert_checkpoint(
                now,
                "proj_x",
                session_id,
                project_dir,
                Some("main"),
                Some(true),
                "opencode",
                "high",
                "sleeping",
                now - 60,
                Some("0x1"),
                Some("foot"),
                None,
                crate::store::trigger::DISAPPEARANCE,
                None,
            )
            .unwrap();
    }

    // --- the bridge's own refusals: no executor call at all ----------------

    #[test]
    fn a_missing_target_refuses_and_calls_no_executor() {
        // 26.9: `/resume` alone, from a workspace-scoped chat.
        let (dir, db) = sandbox("no-target");
        let plat = MockPlatform::quiet();
        let d = workspace_descriptor(&dir);

        let report = resume_action(&plat, &db, &d, None);

        let refused = report.unwrap_err();
        assert_eq!(refused, ResumeRefused::NoTarget);
        assert_eq!(refused.class(), "no target");
        assert!(!refused.reached_executor());
        // Nothing was focused, nothing was launched, and the database the
        // executor would have opened was never created.
        assert!(!plat.acted(), "a refusal must reach no executor");
        assert!(!db.exists(), "no lookup means no store open");
        // The refusal names what is missing and how to supply it.
        let line = refused.line();
        assert!(line.contains("no session id was entered"), "{line}");
        assert!(line.contains("records no context session"), "{line}");
        assert!(line.contains("/resume sess_"), "{line}");
        assert!(line.contains("nothing was resumed"), "{line}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_malformed_target_refuses_and_calls_no_executor() {
        // 26.11, 26.12: shape first, before any lookup or action — and the
        // entered argument wins even when a valid context session exists, so
        // a typo is refused instead of silently resuming something else.
        let (dir, db) = sandbox("malformed");
        let plat = MockPlatform::quiet();
        let d = scoped_descriptor(&dir, "sess_0123456789abcdef");

        for entered in [
            "not-a-session",
            "sess_0123456789abcde", // one hex digit short
            "sess_0123456789abcdef0", // one too long
            "sess_0123456789ABCDEF", // upper case
            "sess_0123456789abcdef;rm -rf /",
            "proj_0123456789abcdef",
            "",
        ] {
            let refused = resume_action(&plat, &db, &d, Some(entered)).unwrap_err();
            assert_eq!(refused.class(), "malformed target", "{entered:?}");
            assert!(!refused.reached_executor(), "{entered:?}");
            assert!(
                matches!(refused, ResumeRefused::MalformedTarget { .. }),
                "{entered:?}"
            );
            let line = refused.line();
            assert!(line.contains("hexadecimal digits"), "{line}");
            assert!(line.contains("nothing was resumed"), "{line}");
        }
        assert!(!plat.acted(), "a malformed target must reach no executor");
        assert!(!db.exists(), "no shape, no lookup");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn target_resolution_prefers_the_argument_then_the_descriptor() {
        // 26.7, 26.8: pure resolution, in order.
        let (dir, _db) = sandbox("resolve");
        let scoped = scoped_descriptor(&dir, "sess_0123456789abcdef");
        let workspace = workspace_descriptor(&dir);

        // An entered argument wins over the descriptor's context session.
        assert_eq!(
            resume_target(&scoped, Some("sess_aaaaaaaaaaaaaaaa")).unwrap(),
            "sess_aaaaaaaaaaaaaaaa"
        );
        // No argument falls back to the descriptor.
        assert_eq!(
            resume_target(&scoped, None).unwrap(),
            "sess_0123456789abcdef"
        );
        // No argument and no context session: refused, not guessed.
        assert_eq!(
            resume_target(&workspace, None).unwrap_err(),
            ResumeRefused::NoTarget
        );
        // An argument still works for a workspace-scoped chat.
        assert_eq!(
            resume_target(&workspace, Some("sess_aaaaaaaaaaaaaaaa")).unwrap(),
            "sess_aaaaaaaaaaaaaaaa"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_an_entered_resume_command_can_produce_a_target() {
        // 26.3-26.6, 27.3: the bridge is reachable from the `Act` arm alone.
        // A sentence *asking* for a resume is a question and carries no
        // action, whatever it says.
        match classify("/resume sess_0123456789abcdef") {
            ChatInput::Act(ActionCommand::Resume { target }) => {
                assert_eq!(target.as_deref(), Some("sess_0123456789abcdef"));
            }
            other => panic!("expected the resume action, got {other:?}"),
        }
        for line in [
            "resume sess_0123456789abcdef",
            "please resume sess_0123456789abcdef now",
            "/resumesess_0123456789abcdef",
        ] {
            let input = classify(line);
            assert!(
                !input.changes_workspace_state(),
                "{line:?} must not act, got {input:?}"
            );
            assert!(
                !matches!(input, ChatInput::Act(_)),
                "{line:?} must not be an action, got {input:?}"
            );
        }
    }

    // --- the executor's two operations ------------------------------------

    #[test]
    fn a_live_target_is_focused_and_no_terminal_is_opened() {
        // 26.13, 28.7: Level 1. The id comes from `collect`, because that is
        // where real session ids come from.
        let (dir, db) = sandbox("live");
        let plat = MockPlatform::with(vec![("live".to_string(), "/tmp".to_string())]);
        let snapshot = crate::collector::collect(&plat);
        assert_eq!(snapshot.sessions.len(), 1);
        let sid = snapshot.sessions[0].id.clone();
        let d = workspace_descriptor(&dir);

        let done = resume_action(&plat, &db, &d, Some(&sid)).unwrap();

        assert_eq!(
            done,
            ResumeDone::FocusedLive {
                session_id: sid.clone()
            }
        );
        assert_eq!(done.session_id(), sid);
        assert_eq!(plat.focused.borrow().len(), 1);
        assert!(plat.launched.borrow().is_empty(), "no duplicate terminal");
        // 26.21: the line names the target and the operation.
        let line = done.line();
        assert!(line.contains(&sid), "{line}");
        assert!(line.contains("focused its live window"), "{line}");
        assert!(line.contains("no new terminal"), "{line}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_checkpointed_target_opens_exactly_one_terminal() {
        // 26.14: Level 2, from the descriptor's own context session (26.7),
        // so this also covers `/resume` with no argument succeeding.
        let (dir, db) = sandbox("checkpointed");
        let project = dir.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let target = "sess_aaaaaaaaaaaaaaaa";
        let mut store = crate::store::Store::open(&db).unwrap();
        checkpoint_row(&mut store, target, &project.to_string_lossy());
        drop(store);
        let plat = MockPlatform::with(Vec::new());
        let d = scoped_descriptor(&dir, target);

        let done = resume_action(&plat, &db, &d, None).unwrap();

        let directory = project.to_string_lossy().into_owned();
        assert_eq!(
            done,
            ResumeDone::OpenedTerminal {
                session_id: target.to_string(),
                directory: directory.clone(),
            }
        );
        // Exactly one terminal, at the validated directory, carrying no
        // command — so nothing ran inside it (26.20).
        let launched = plat.launched.borrow().clone();
        assert_eq!(launched.len(), 1);
        assert_eq!(launched[0].directory, directory);
        assert!(launched[0].command.is_empty(), "{:?}", launched[0].command);
        assert!(plat.focused.borrow().is_empty());
        // 26.21: target, operation, and where.
        let line = done.line();
        assert!(line.contains(target), "{line}");
        assert!(line.contains("opened one terminal"), "{line}");
        assert!(line.contains(&directory), "{line}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- refusals the executor owns, carried verbatim ---------------------

    #[test]
    fn a_non_resumable_target_carries_the_executors_reason() {
        // 26.15, 26.22: valid shape, nothing live, no checkpoint. The bridge
        // does not re-derive "unavailable" — it carries what the executor
        // said.
        let (dir, db) = sandbox("unavailable");
        let plat = MockPlatform::with(Vec::new());
        let d = workspace_descriptor(&dir);
        let target = "sess_dddddddddddddddd";

        let refused = resume_action(&plat, &db, &d, Some(target)).unwrap_err();

        match &refused {
            ResumeRefused::Executor {
                target: t, reason, ..
            } => {
                assert_eq!(t, target);
                // Verbatim from `resume::resume`.
                assert!(reason.contains("unknown session"), "{reason}");
                assert!(reason.contains("no checkpoint"), "{reason}");
            }
            other => panic!("expected an executor refusal, got {other:?}"),
        }
        assert_eq!(refused.class(), "resume refused");
        assert!(refused.reached_executor());
        // The executor was reached, but it changed nothing.
        assert!(!plat.acted());
        let line = refused.line();
        assert!(line.contains(target), "{line}");
        assert!(line.contains("unknown session"), "{line}");
        assert!(line.contains("nothing else changed"), "{line}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_project_directory_refusal_names_the_directory() {
        // 26.16: the executor validates the directory and names it; the
        // bridge carries that name through and substitutes nothing.
        let (dir, db) = sandbox("bad-dir");
        let target = "sess_bbbbbbbbbbbbbbbb";
        let mut store = crate::store::Store::open(&db).unwrap();
        checkpoint_row(&mut store, target, "/no/such/project");
        drop(store);
        let plat = MockPlatform::with(Vec::new());
        let d = workspace_descriptor(&dir);

        let refused = resume_action(&plat, &db, &d, Some(target)).unwrap_err();

        let line = refused.line();
        assert!(line.contains("/no/such/project"), "{line}");
        assert!(line.contains("unavailable"), "{line}");
        assert!(!plat.acted(), "an invalid directory opens no terminal");
        // No substitute directory anywhere in the line.
        assert!(!line.contains(&dir.to_string_lossy().into_owned()), "{line}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_focus_is_reported_and_never_falls_back() {
        // The executor owns the no-fallback rule; the bridge must not soften
        // it into "well, open a terminal then".
        let (dir, db) = sandbox("focus-fail");
        let mut plat = MockPlatform::with(vec![("live".to_string(), "/tmp".to_string())]);
        plat.fail_focus = true;
        let sid = crate::collector::collect(&plat).sessions[0].id.clone();
        let d = workspace_descriptor(&dir);

        let refused = resume_action(&plat, &db, &d, Some(&sid)).unwrap_err();

        let line = refused.line();
        assert!(line.contains("not falling back"), "{line}");
        assert!(plat.launched.borrow().is_empty(), "no fallback terminal");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_launch_is_reported_without_a_second_attempt() {
        let (dir, db) = sandbox("launch-fail");
        let project = dir.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let target = "sess_eeeeeeeeeeeeeeee";
        let mut store = crate::store::Store::open(&db).unwrap();
        checkpoint_row(&mut store, target, &project.to_string_lossy());
        drop(store);
        let mut plat = MockPlatform::with(Vec::new());
        plat.fail_launch = true;
        let d = workspace_descriptor(&dir);

        let refused = resume_action(&plat, &db, &d, Some(target)).unwrap_err();

        let line = refused.line();
        assert!(line.contains("launch failed"), "{line}");
        assert!(plat.launched.borrow().is_empty(), "the launch was refused");
        assert!(plat.focused.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- invariants that hold on every path -------------------------------

    #[test]
    fn no_resume_starts_an_agent() {
        // 26.20. The mechanical part: the only way an agent could start on
        // this path is a command inside a launched terminal, and every launch
        // this bridge causes carries an empty command. The structural part:
        // `resume_action` takes no binary search path and no timeout, so no
        // `summary::run_agent` call is constructible from it, and
        // `resume::resume` has exactly two outcomes, neither of them an agent.
        let (dir, db) = sandbox("no-agent");
        let project = dir.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let target = "sess_ffffffffffffffff";
        let mut store = crate::store::Store::open(&db).unwrap();
        checkpoint_row(&mut store, target, &project.to_string_lossy());
        drop(store);
        let plat = MockPlatform::with(Vec::new());
        let d = workspace_descriptor(&dir);

        let done = resume_action(&plat, &db, &d, Some(target)).unwrap();

        assert!(matches!(done, ResumeDone::OpenedTerminal { .. }));
        for launch in plat.launched.borrow().iter() {
            assert!(
                launch.command.is_empty(),
                "a resume terminal must run nothing: {:?}",
                launch.command
            );
        }
        // And the harness names never appear in a resume line.
        let line = done.line();
        for agent in crate::agents::KNOWN {
            assert!(!line.contains(agent.id), "{line}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_presented_line_carries_a_pid_or_a_window_address() {
        // 15.2, 26.25. The focused window's address (`0x1`) and its pid (100)
        // exist in the mock and are used by the executor, and neither may
        // surface in what the human reads.
        let (dir, db) = sandbox("privacy");
        let plat = MockPlatform::with(vec![("live".to_string(), "/tmp".to_string())]);
        let sid = crate::collector::collect(&plat).sessions[0].id.clone();
        let d = workspace_descriptor(&dir);

        let done = resume_action(&plat, &db, &d, Some(&sid)).unwrap();
        // The executor did focus by address...
        assert_eq!(plat.focused.borrow()[0], "0x1");
        // ...and the address never reached the presented line.
        let line = done.line();
        assert!(!line.contains("0x"), "{line}");
        assert!(!line.contains("pid"), "{line}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_reason_carrying_control_bytes_or_a_secret_is_made_safe() {
        // 26.25: executor text is bounded, flattened to one line, and
        // scrubbed — the same treatment `present_answer` gives harness text.
        let secret = format!("{}={}", "token", "AbCd1234EfGh");
        let raw = format!("project directory unavailable: /tmp/a\nb\u{1b}]2;steal\u{7} {secret}");
        let refused = ResumeRefused::Executor {
            target: "sess_0123456789abcdef".to_string(),
            reason: one_line(&raw, MAX_RESUME_TEXT_CHARS),
        };
        let line = refused.line();
        assert!(!line.contains('\u{1b}'), "{line}");
        assert!(!line.contains('\n'), "{line}");
        assert!(line.contains("token=[redacted]"), "{line}");
        assert!(!line.contains("AbCd1234EfGh"), "{line}");
        // The reason still says what it said.
        assert!(line.contains("project directory unavailable"), "{line}");
        // And it stays bounded, in characters rather than bytes.
        let long = "x".repeat(MAX_RESUME_TEXT_CHARS + 100);
        assert_eq!(
            one_line(&long, MAX_RESUME_TEXT_CHARS).chars().count(),
            MAX_RESUME_TEXT_CHARS
        );
        let wide = "é".repeat(MAX_RESUME_TEXT_CHARS + 100);
        assert_eq!(
            one_line(&wide, MAX_RESUME_TEXT_CHARS).chars().count(),
            MAX_RESUME_TEXT_CHARS
        );
    }

    #[test]
    fn every_outcome_leaves_the_chat_ready() {
        // 26.23: one line, and it always ends by saying so.
        let reports: Vec<ResumeReport> = vec![
            Ok(ResumeDone::FocusedLive {
                session_id: "sess_0123456789abcdef".to_string(),
            }),
            Ok(ResumeDone::OpenedTerminal {
                session_id: "sess_0123456789abcdef".to_string(),
                directory: "/home/u/project".to_string(),
            }),
            Err(ResumeRefused::NoTarget),
            Err(ResumeRefused::MalformedTarget {
                entered: "nope".to_string(),
            }),
            Err(ResumeRefused::Executor {
                target: "sess_0123456789abcdef".to_string(),
                reason: "unknown session sess_0123456789abcdef (no checkpoint)".to_string(),
            }),
        ];
        for report in &reports {
            let line = resume_line(report);
            assert!(line.ends_with(RESUME_READY), "{line}");
            // One line, always.
            assert_eq!(line.lines().count(), 1, "{line}");
            assert!(!line.contains('\u{1b}'), "{line}");
        }
    }
}
