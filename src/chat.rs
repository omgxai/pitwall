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
    let budget = room.clamp(1, MAX_CONTEXT_LABEL_CHARS);
    debug_assert!(
        room >= 39,
        "title head grew beyond the documented worst case"
    );
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
/// authority to unlink one lease. `Debug` only, so the allocator's
/// `Result<ChatNumberLease, _>` can be unwrapped in tests.
#[derive(Debug)]
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
        _ => ChatInput::Unknown {
            entered: unknown_command_text(word),
        },
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
        return ChatInput::QuestionTooLong {
            chars: scrubbed_chars,
        };
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
pub const CHAT_INSTRUCTION: &str = "Act as Pitwall's race engineer answering one question about the human's workspace. Answer the user's actual question first, then provide relevant workspace context. Answer only from the attached, bounded Pitwall workspace context: it is the whole of what you may use. When that context does not carry the evidence the question needs, say plainly that the information is unavailable, and never close the gap with a guess, an assumption, or general knowledge. Read that context as follows. Each entry under \"sessions\" is one observed session, and the record existing is itself what makes that session observed; its \"agent\" and \"agent_confidence\" fields carry the observed agent identity and how well evidenced it is; its \"state\" field describes only that session's current process state, and is one of running, sleeping, stopped or unknown. Those four are process-observation states, not presence or absence. A state of sleeping means the observed session is present and currently waiting at the process level: it does not mean the session is absent, and it does not mean its work is finished. So when you are asked whether a session exists, is observed, is present or is there, answer from whether a record for it appears, never from its state; call a session absent only when no record for it appears at all. Treat process activity and input-output counters as activity evidence only, never as proof of task progress or completion: report what that evidence shows about activity, never that the work advanced or finished. Name an agent only with the confidence the context records for it, and say the agent is unconfirmed when that confidence is low. Use plain language, and name the project and the agent exactly as the context records them. Do not mention internal diagnostics such as process counts, confidence mechanics, missing scrollback, or file paths unless they are themselves the answer. Do not invent work, completion, blockers, or attention items. Answer in 1-4 short sentences. You are observing only: do not execute tasks, do not modify files, do not start, resume, or delegate anything, and do not continue the work. If the question asks for an action, say which in-chat command the human would enter to perform it, and take no action yourself.";

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
        s.id == session_id
            || s.project
                .as_ref()
                .is_some_and(|p| same_project(scoped, &p.id))
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
        let recent = store
            .checkpoints_since(collected_at, MAX_CHAT_EVENTS as i64)
            .ok()?;
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
    crate::context::scrub_string(clean.trim())
        .trim()
        .to_string()
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
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
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
            (
                7u16,
                "opencode",
                "prov/model",
                "Work",
                Some("sess_0123456789abcdef"),
            ),
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
            let t = format!(
                "Pitwall Chat 123{sep}{}{sep}agent default{sep}Workspace",
                a.id
            );
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
        for raw in [
            "  my   project  ",
            "line1\nline2",
            "a\u{00B7}b",
            long.as_str(),
        ] {
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
        let want = [
            "/help",
            "/context",
            "/sessions",
            "/clear",
            "/resume",
            "/exit",
        ];
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
        assert_eq!(
            classify("/sessions"),
            ChatInput::Info(InfoCommand::Sessions)
        );
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
            let carried = ChatInput::Act(ActionCommand::Resume {
                target: Some(raw.to_string()),
            });
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
        assert_eq!(
            question_text("what is child1 doing?"),
            "what is child1 doing?"
        );
        // A question that talks *about* a command is still a question: wording
        // never becomes behaviour (27.3).
        assert_eq!(
            question_text("should I /resume child1?"),
            "should I /resume child1?"
        );
        assert_eq!(question_text("resume child1"), "resume child1");
        assert_eq!(question_text("exit"), "exit");
        // Slashes mid-text: paths and dates are not commands.
        assert_eq!(
            question_text("does src/main.rs matter?"),
            "does src/main.rs matter?"
        );
        assert_eq!(
            question_text("what changed on 12/03?"),
            "what changed on 12/03?"
        );
        // A line whose slash is hidden behind a control character is a
        // question, because the prefix test runs before control stripping.
        assert_eq!(question_text("\u{1b}/exit"), "/exit");
        assert_eq!(
            question_text("\u{0007}/resume sess_0123456789abcdef"),
            "/resume sess_0123456789abcdef"
        );
        // Leading whitespace does not change the class.
        assert_eq!(
            question_text("   how busy is the workspace?  "),
            "how busy is the workspace?"
        );
        // Only a question reaches the harness, and it changes nothing.
        let q = classify("what is child1 doing?");
        assert!(q.invokes_harness());
        assert!(!q.changes_workspace_state());
        assert!(q.vocabulary_entry().is_none());
    }

    #[test]
    fn blank_and_control_only_input_is_a_skipped_no_op() {
        for line in [
            "",
            " ",
            "   \t ",
            "\n",
            "\r\n",
            "\u{007f}",
            "\u{1b}\u{0007}",
        ] {
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
        let over = ChatInput::QuestionTooLong {
            chars: MAX_CHAT_QUESTION_CHARS + 1,
        };
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
        assert!(
            resume_line.contains("CHANGES WORKSPACE STATE"),
            "{resume_line}"
        );
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

    // --- branding and the textual header fallback (task 9.7) --------------
    //
    // Every case here is decided from arguments alone: the capability arrives
    // as a value and the asset as a path in a sandbox directory. No terminal
    // is probed, no real asset is read, and the production data directory is
    // never consulted.

    /// The capability probe's three answers, named short so that every
    /// decision below reads as one line.
    const NO_INLINE: crate::platform::InlineImage = crate::platform::InlineImage::None;
    const SIXEL: crate::platform::InlineImage = crate::platform::InlineImage::Sixel;
    const KITTY: crate::platform::InlineImage = crate::platform::InlineImage::Kitty;

    /// A PNG head that [`renderable_asset`] accepts: signature, IHDR length,
    /// IHDR type, and [`BRANDING_DISPLAY_PX`] square dimensions.
    ///
    /// Only the leading bytes matter — the decision is signature-based and
    /// reads the IHDR fields, so this is exactly as much file as it needs.
    fn png_head_256() -> Vec<u8> {
        let mut bytes: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&BRANDING_DISPLAY_PX.to_be_bytes());
        bytes.extend_from_slice(&BRANDING_DISPLAY_PX.to_be_bytes());
        // Bit depth, colour type, compression, filter, interlace.
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    #[test]
    fn a_terminal_without_inline_images_is_text_only_even_with_an_asset() {
        let dir = sandbox();
        // A *renderable* asset, so the only reason for the textual header is
        // the absent capability (20.8).
        let asset = dir.join(BRANDING_ASSET_FILE);
        std::fs::write(&asset, png_head_256()).unwrap();
        let accepted = renderable_asset(&asset);
        assert_eq!(accepted, Some(asset.clone()));

        let no_capability = plan_branding(NO_INLINE, Some(asset.as_path()));
        assert_eq!(no_capability, Branding::TextOnly);
        // Sixel is the textual header too: Pitwall carries no sixel encoder,
        // and "capability present but unusable" is not a third outcome.
        let sixel = plan_branding(SIXEL, Some(asset.as_path()));
        assert_eq!(sixel, Branding::TextOnly);
        // And with no asset located at all, the same answer.
        let neither = plan_branding(NO_INLINE, None);
        assert_eq!(neither, Branding::TextOnly);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inline_capability_without_a_usable_asset_is_silently_text_only() {
        let dir = sandbox();
        let missing = dir.join(BRANDING_ASSET_FILE);
        assert!(!missing.exists(), "the sandbox starts empty");

        let planned = plan_branding(KITTY, Some(missing.as_path()));
        assert_eq!(planned, Branding::TextOnly);
        // Nothing is said about the picture that is not there (20.8, 28.10).
        let mut sink: Vec<u8> = Vec::new();
        print_branding(&mut sink, &planned).unwrap();
        assert!(
            sink.is_empty(),
            "a missing asset must be silent, wrote {:?}",
            String::from_utf8_lossy(&sink)
        );

        // A file that is present but not an emittable asset is the same
        // silent answer, not a warning.
        let jpeg = dir.join("pitwallpixelart.jpeg");
        std::fs::write(&jpeg, b"\xff\xd8\xff\xe0 not a png").unwrap();
        let unrenderable = plan_branding(KITTY, Some(jpeg.as_path()));
        assert_eq!(unrenderable, Branding::TextOnly);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_only_branding_writes_not_one_byte() {
        let mut sink: Vec<u8> = Vec::new();
        print_branding(&mut sink, &Branding::TextOnly).unwrap();
        assert!(
            sink.is_empty(),
            "TextOnly writes no note and no blank line, wrote {:?}",
            String::from_utf8_lossy(&sink)
        );
    }

    #[test]
    fn the_textual_header_is_printed_whole_when_there_is_no_asset() {
        let dir = sandbox();
        let path = dir.to_string_lossy().into_owned();
        let d = descriptor_at(&path);

        let mut printed: Vec<u8> = Vec::new();
        print_header(&mut printed, &d, &Branding::TextOnly, Palette::plain()).unwrap();
        let printed_text = String::from_utf8(printed).expect("the header is UTF-8");
        // The exact 20.8 guarantee: the complete textual header, and the
        // branding stage adds nothing to it in either direction.
        let expected = render_header(&d, Palette::plain());
        assert_eq!(printed_text, expected);

        // Complete: wordmark, title, fields, coverage, prompt banner.
        assert!(printed_text.contains(WORDMARK), "{printed_text}");
        let title = header_title(&d);
        assert!(printed_text.contains(&title), "{printed_text}");
        for (label, value) in header_fields(&d) {
            assert!(printed_text.contains(label), "{printed_text}");
            assert!(printed_text.contains(&value), "{printed_text}");
        }
        for line in context_block(&d) {
            assert!(printed_text.contains(&line), "{printed_text}");
        }
        for line in prompt_banner() {
            assert!(printed_text.contains(&line), "{printed_text}");
        }

        // And no apology, no warning, and no alternative graphics mechanism
        // announced: the absent picture is never mentioned (20.8).
        let lower = printed_text.to_lowercase();
        let forbidden = "image picture graphic logo sixel kitty unsupported warning";
        for word in forbidden.split_whitespace() {
            assert!(
                !lower.contains(word),
                "the textual header must not mention {word:?}: {printed_text}"
            );
        }
        assert!(
            !printed_text.contains('\u{1b}'),
            "no escape sequence belongs in the plain textual header: {printed_text:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_inline_terminal_with_a_usable_asset_emits_that_file_then_the_header() {
        let dir = sandbox();
        let path = dir.to_string_lossy().into_owned();
        let d = descriptor_at(&path);
        let asset = dir.join("pitwall-256.png");
        std::fs::write(&asset, png_head_256()).unwrap();

        let planned = plan_branding(KITTY, Some(asset.as_path()));
        let inline = Branding::Inline {
            path: asset.clone(),
        };
        assert_eq!(planned, inline);

        // The image is the escape sequence for that exact file plus one
        // newline, and the header text below it is unchanged — the picture
        // adds to the header, it does not replace part of it (20.7, 20.8).
        let sequence = kitty_file_image_sequence(&asset).expect("an absolute, control-free path");
        let mut want: Vec<u8> = sequence.into_bytes();
        want.push(b'\n');
        let mut image_only: Vec<u8> = Vec::new();
        print_branding(&mut image_only, &planned).unwrap();
        assert_eq!(image_only, want);

        let mut whole: Vec<u8> = Vec::new();
        print_header(&mut whole, &d, &planned, Palette::plain()).unwrap();
        let mut expected = want.clone();
        expected.extend_from_slice(render_header(&d, Palette::plain()).as_bytes());
        assert_eq!(whole, expected);

        let _ = std::fs::remove_dir_all(&dir);
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
        let want = vec![
            "/opt/bin/harness".to_string(),
            "-p".to_string(),
            msg.clone(),
        ];
        assert_eq!(call.argv().to_vec(), want);
        assert!(call.context_on_stdin());

        // codex: exec <MSG>, same rules.
        let cx = descriptor("codex", "", &dir);
        let call = build_chat_call(&cx, bin, ctx, &msg).unwrap();
        let want = vec![
            "/opt/bin/harness".to_string(),
            "exec".to_string(),
            msg.clone(),
        ];
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
            let carrying = call
                .argv()
                .iter()
                .filter(|a| a.contains("rm -rf ~"))
                .count();
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
            std::slice::from_ref(&bin_dir),
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
        assert!(
            staged_contexts(&runtime).is_empty(),
            "{:?}",
            staged_contexts(&runtime)
        );
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
        // The canary is the document's structural JSON prefix, not the bare
        // `sessions` field name. `CHAT_INSTRUCTION` legitimately names that
        // field when it tells the harness how to read the context, so a
        // bare-name needle would read that prose as leaked context. This
        // prefix is `render_document`'s own opening literal and cannot occur
        // in prose, while still matching wherever the document itself does.
        assert!(
            !call.argv().iter().any(|a| a.contains("{\"sessions\":[")),
            "{:?}",
            call.argv()
        );

        let answer = respond_in(
            &d,
            &obs,
            &question("what is running?"),
            &runtime,
            std::slice::from_ref(&bin_dir),
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
            std::slice::from_ref(&bin_dir),
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
            std::slice::from_ref(&bin_dir),
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
            std::slice::from_ref(&bin_dir),
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
            std::slice::from_ref(&bin_dir),
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
            "sess_0123456789abcde",   // one hex digit short
            "sess_0123456789abcdef0", // one too long
            "sess_0123456789ABCDEF",  // upper case
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
        assert!(
            !line.contains(&dir.to_string_lossy().into_owned()),
            "{line}"
        );
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

#[cfg(test)]
mod property_tests {
    //! Property tests for the chat surface: tasks 8.4, 9.5, 9.6 and the Rust
    //! half of 13.4.
    //!
    //! `proptest` is a dev-dependency pinned `=1.5.0`; each design property
    //! below is exactly **one** test at 100+ cases, tagged with its full
    //! property text on the line above the test. The example tests in
    //! [`super::tests`], [`super::responder_tests`] and
    //! [`super::resume_tests`] pin individual shapes byte-for-byte; nothing
    //! here repeats them — every test below generalises over generated
    //! inputs.
    //!
    //! One module rather than three because six of the fourteen properties
    //! span the descriptor, the classifier, the responder and the resume
    //! bridge at once, and they share one sandbox, one recording platform
    //! and one fake harness.
    //!
    //! **Hermetic by construction.** Every case runs in its own temp
    //! directory: the runtime directory arrives through the
    //! [`respond_in`]/[`allocate_chat_number_in`] seams, harnesses are shell
    //! scripts in a temp `PATH` entry discovered through
    //! [`crate::agents::discover_in`], the continuity store is a temp SQLite
    //! file, and the `Platform` is a recording mock. No case touches the
    //! production runtime directory, the real config file, a real agent, a
    //! real compositor or a real terminal.

    use super::*;
    use crate::platform::{
        ChatLease, GitInfo, InlineImage, IoCounters, Platform, RawProcess, TerminalSpec,
        TerminalText, WindowInfo,
    };
    use proptest::prelude::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    // -----------------------------------------------------------------
    // Sandboxes
    // -----------------------------------------------------------------

    static PROP_SEQ: AtomicU64 = AtomicU64::new(0);

    /// A private directory tree per case, unique per process *and* per case
    /// so 100+ cases and parallel test threads never share one.
    ///
    /// `Drop` removes it, which matters here in a way it does not in an
    /// example test: a failed `prop_assert!` returns early, so an explicit
    /// clean-up at the end of the body would be skipped exactly in the runs
    /// that produce the most files.
    struct Sandbox {
        root: PathBuf,
        /// A `PATH` entry holding fake harness binaries.
        bin_dir: PathBuf,
        /// Stands in for the XDG data directory (the continuity store).
        data_dir: PathBuf,
        /// Stands in for `context::ephemeral_dir()`.
        runtime_dir: PathBuf,
        /// One line per fake-harness invocation.
        log: PathBuf,
    }

    impl Sandbox {
        fn new(tag: &str) -> Sandbox {
            let n = PROP_SEQ.fetch_add(1, Ordering::SeqCst);
            let root = std::env::temp_dir().join(format!(
                "pitwall-m8-chatprop-{tag}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("sandbox root");
            Sandbox {
                bin_dir: root.join("bin"),
                data_dir: root.join("data"),
                runtime_dir: root.join("run"),
                log: root.join("harness.log"),
                root,
            }
        }

        /// Same, plus a fake `opencode` that records each invocation and
        /// answers with one text event carrying `answer`.
        fn with_harness(tag: &str, answer: &str) -> Sandbox {
            let sandbox = Sandbox::new(tag);
            counting_harness(&sandbox.bin_dir, "opencode", &sandbox.log, answer);
            sandbox
        }

        fn bin_dirs(&self) -> Vec<PathBuf> {
            vec![self.bin_dir.clone()]
        }

        fn db(&self) -> PathBuf {
            self.data_dir.join(crate::store::DB_FILENAME)
        }

        /// The absolute project directory a descriptor captures.
        fn project_dir(&self) -> String {
            self.root.to_string_lossy().into_owned()
        }

        /// How many times the fake harness ran.
        fn invocations(&self) -> usize {
            harness_invocations(&self.log)
        }

        /// Ephemeral context documents still present in the runtime dir.
        fn staged(&self) -> Vec<String> {
            let Ok(entries) = std::fs::read_dir(&self.runtime_dir) else {
                return Vec::new();
            };
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("ctx-"))
                .collect()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// An executable stand-in for a harness, named exactly as
    /// [`crate::agents::KNOWN`] expects so `discover_in` finds it.
    ///
    /// Every run appends one line to `log`, which is how "the harness ran
    /// exactly once per question" becomes a count rather than an argument:
    /// the harness is not a `Platform` capability, so a recording mock
    /// cannot see it (see the note on `resume.rs`'s mock).
    fn counting_harness(bin_dir: &Path, name: &str, log: &Path, answer: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(bin_dir).expect("bin dir");
        let bin = bin_dir.join(name);
        // Drains stdin first, so the same script stands in for a file-delivery
        // harness (stdin is `null`, `cat` sees EOF at once) and for a
        // stdin-delivery one (the document is consumed, so the runner's writer
        // thread never fails).
        let body = format!(
            "#!/bin/sh\ncat >/dev/null 2>/dev/null\nprintf 'invocation\\n' >> '{}'\n\
             printf '%s\\n' '{{\"type\":\"text\",\"text\":\"{}\"}}'\n",
            log.display(),
            answer
        );
        std::fs::write(&bin, body).expect("fake harness");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("mode 0755");
        bin
    }

    /// A discoverable but non-executing binary: enough for argv construction,
    /// which is pure.
    fn fake_binary(bin_dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(bin_dir).expect("bin dir");
        let bin = bin_dir.join(name);
        std::fs::write(&bin, "#!/bin/sh\nexit 0\n").expect("fake binary");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("mode 0755");
        bin
    }

    fn harness_invocations(log: &Path) -> usize {
        std::fs::read_to_string(log)
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }

    /// Does `haystack` carry `needle` as a byte substring? Used to search a
    /// SQLite file, which is not text.
    fn bytes_contain(haystack: &[u8], needle: &str) -> bool {
        let needle = needle.as_bytes();
        !needle.is_empty()
            && haystack.len() >= needle.len()
            && haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// Every regular file under `dir`, recursively. An unreadable directory
    /// is "nothing there", not an error.
    fn files_under(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                out.extend(files_under(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    // -----------------------------------------------------------------
    // The recording platform
    // -----------------------------------------------------------------

    /// One recorded terminal launch. The command is kept rather than
    /// asserted away, because "no agent was started" is exactly the claim
    /// that this vector stayed empty (26.20).
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Launch {
        directory: String,
        command: Vec<String>,
    }

    /// A `Platform` that observes what it is told to and **records** every
    /// acting capability the trait exposes, following `resume.rs`'s mock.
    ///
    /// Recording rather than panicking (as `responder_tests::QuietPlatform`
    /// does) because two properties need a *count* of actions rather than
    /// their absence: Property 18 asserts zero actions per non-acting turn
    /// while still allowing an entered `/resume` to focus one window, and
    /// Property 26 asserts "exactly one".
    ///
    /// `observations` counts calls to [`Platform::processes`], which
    /// `collector::collect` makes exactly once per observation. That counter
    /// is what makes "each harness invocation is preceded by a fresh
    /// observation taken after its question was read" checkable.
    struct RecordingPlatform {
        windows: Vec<WindowInfo>,
        processes: Vec<RawProcess>,
        leases: Vec<ChatLease>,
        text: TerminalText,
        io: Option<IoCounters>,
        fail_launch: bool,
        fail_focus: bool,
        launched: std::cell::RefCell<Vec<Launch>>,
        focused: std::cell::RefCell<Vec<String>>,
        observations: std::cell::Cell<usize>,
    }

    impl RecordingPlatform {
        /// Observes nothing, can do nothing: the platform a refusal case
        /// uses, where any recorded action is a failure.
        fn quiet() -> RecordingPlatform {
            RecordingPlatform {
                windows: Vec::new(),
                processes: Vec::new(),
                leases: Vec::new(),
                text: TerminalText::Unavailable {
                    reason: "mock has no terminals",
                },
                io: None,
                fail_launch: false,
                fail_focus: false,
                launched: std::cell::RefCell::new(Vec::new()),
                focused: std::cell::RefCell::new(Vec::new()),
                observations: std::cell::Cell::new(0),
            }
        }

        fn with_windows(windows: Vec<WindowInfo>) -> RecordingPlatform {
            RecordingPlatform {
                windows,
                ..RecordingPlatform::quiet()
            }
        }

        /// Total acting operations recorded: window focuses plus terminal
        /// launches. There is no process-signal capability in the trait at
        /// all (see [`crate::platform::Platform`]), so the third operation
        /// Property 18 names has no way to be performed from here; the only
        /// kill anywhere on the chat path is `summary::run_agent` killing
        /// its own timed-out child, which the harness invocation count
        /// covers.
        fn actions(&self) -> usize {
            self.launched.borrow().len() + self.focused.borrow().len()
        }

        fn observations(&self) -> usize {
            self.observations.get()
        }
    }

    impl Platform for RecordingPlatform {
        fn processes(&self) -> Vec<RawProcess> {
            self.observations.set(self.observations.get() + 1);
            self.processes.clone()
        }
        fn windows(&self) -> Vec<WindowInfo> {
            self.windows.clone()
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
            self.leases.clone()
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
        fn process_io(&self, _pid: u32) -> Option<IoCounters> {
            self.io
        }
        fn terminal_text(&self, _pid: u32, _class: &str) -> TerminalText {
            self.text.clone()
        }
    }

    fn window(address: &str, class: &str, title: &str, pid: u32) -> WindowInfo {
        WindowInfo {
            address: address.to_string(),
            class: class.to_string(),
            initial_class: class.to_string(),
            title: title.to_string(),
            workspace: "1".to_string(),
            pid,
        }
    }

    fn process(pid: u32, ppid: u32, command: &str, exe_name: &str, cwd: &str) -> RawProcess {
        RawProcess {
            pid,
            ppid,
            name: exe_name.to_string(),
            command: command.to_string(),
            exe_name: exe_name.to_string(),
            cwd: cwd.to_string(),
            state_code: 'S',
            starttime_ticks: 4200,
        }
    }

    /// A live `pitwall chat`, exactly as
    /// [`crate::collector::is_pitwall_chat_argv`] recognises one.
    fn chat_process(pid: u32) -> RawProcess {
        process(pid, 1, "/usr/bin/pitwall chat", "pitwall", "/home/u")
    }

    // -----------------------------------------------------------------
    // Shared generators and small oracles
    // -----------------------------------------------------------------

    /// Models a chat can be configured with: the empty value (the agent
    /// default) and [`crate::summary::valid_model`] ids.
    const MODEL_IDS: &[&str] = &["", "prov/model", "openrouter/anth-3.5", "a/b-c_d.e:f"];

    /// Context labels [`ChatDescriptor::capture`] accepts: non-empty, no
    /// control character, no title separator, within
    /// [`MAX_CONTEXT_LABEL_CHARS`]. The padded one is deliberate — a label
    /// that is only partly spaces must survive the title round trip
    /// untouched.
    const CONTEXT_LABELS: &[&str] = &[
        "Work",
        WORKSPACE_LABEL,
        "my project",
        " padded ",
        "a",
        "caf\u{00e9}",
    ];

    fn session_id() -> impl Strategy<Value = String> {
        proptest::string::string_regex("sess_[0-9a-f]{16}").expect("static regex")
    }

    fn word() -> impl Strategy<Value = String> {
        proptest::string::string_regex("[a-z][a-z0-9]{3,7}").expect("static regex")
    }

    /// The question of an accepted line, or a panic naming what was
    /// classified instead.
    fn question_of(line: &str) -> ChatQuestion {
        match classify(line) {
            ChatInput::Question(q) => q,
            other => panic!("{line:?} must classify as a question, got {other:?}"),
        }
    }

    fn descriptor_in(
        sandbox: &Sandbox,
        number: u16,
        harness: &str,
        model: &str,
        label: &str,
        context: Option<&str>,
    ) -> ChatDescriptor {
        ChatDescriptor::capture(
            number,
            harness,
            model,
            label,
            context,
            1_700_000_100,
            &sandbox.project_dir(),
        )
        .expect("a generated descriptor must be valid by construction")
    }

    /// The per-harness argv of design §4.6's table, written out
    /// independently of [`build_chat_call`] so the comparison is an oracle
    /// rather than a restatement.
    fn expected_argv(d: &ChatDescriptor, bin: &Path, ctx: &Path, message: &str) -> Vec<String> {
        let bin_text = bin.to_string_lossy().into_owned();
        match d.harness() {
            "opencode" => {
                let mut argv = vec![
                    bin_text,
                    "run".to_string(),
                    "--format".to_string(),
                    "json".to_string(),
                    "--dir".to_string(),
                    d.project_dir().to_string(),
                    "-f".to_string(),
                    ctx.to_string_lossy().into_owned(),
                ];
                if !d.model().is_empty() {
                    argv.push("-m".to_string());
                    argv.push(d.model().to_string());
                }
                argv.push(message.to_string());
                argv
            }
            "claude" => vec![bin_text, "-p".to_string(), message.to_string()],
            "codex" => vec![bin_text, "exec".to_string(), message.to_string()],
            other => panic!("no argv shape recorded for harness {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Task 8.4 — Properties 7, 8 and 11
    // -----------------------------------------------------------------

    proptest! {
        // 128 cases: the generated shape is (0..8 observed numbers) ×
        // (0..8 live leases) × (0..3 out-of-range values) × (1..4
        // allocations in a row) × (exhaustion checked or not).
        #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

        // **Validates: Requirements 10.1, 10.2, 10.3, 10.5, 21.2**
        // Feature: pitwall-chat-and-brief-ticker, Property 7: Chat numbers are the lowest available and always distinct — For any set of in-use numbers drawn from `1..=999`, the allocator returns the smallest number not in that set (formatted as three zero-padded digits), or refuses when the set is full; and for any sequence of allocations against an accumulating in-use set, no number is ever issued twice while its holder lives.
        #[test]
        fn prop7_chat_numbers_are_the_lowest_available_and_always_distinct(
            observed in proptest::collection::vec(1u16..=999u16, 0..8),
            leased in proptest::collection::vec(1u16..=999u16, 0..8),
            out_of_range in proptest::collection::vec(
                prop_oneof![Just(0u16), 1000u16..2000u16],
                0..3,
            ),
            run_length in 1usize..4,
            check_exhaustion in any::<bool>(),
        ) {
            let sandbox = Sandbox::new("p7");
            let dir = sandbox.root.clone();

            // The test process plays the role of a running chat, so a lease
            // body carrying its own pid is correctly seen as *held*. That is
            // what makes the accumulating half of the property meaningful:
            // each claim leaves a real file behind, and the next allocation
            // has to take the `EEXIST` branch and re-validate it.
            let own = std::process::id();
            let foreign = if own == 4242 { 4243 } else { 4242 };
            let processes = vec![chat_process(own), chat_process(foreign)];

            // The leases the observation carries: real files under the
            // sandbox, each owned by a live `pitwall chat`.
            for number in &leased {
                std::fs::write(
                    dir.join(lease_file_name(*number)),
                    format!("{foreign}\n"),
                )
                .expect("sandbox lease");
            }
            let leases: Vec<ChatLease> = leased
                .iter()
                .map(|n| ChatLease { number: *n, pid: foreign })
                .collect();

            // Out-of-range values are not chat numbers at all, so they must
            // change nothing about the answer.
            let mut observed_numbers = observed.clone();
            observed_numbers.extend(out_of_range.iter().copied());

            let mut in_use: std::collections::BTreeSet<u16> =
                observed.iter().copied().collect();
            in_use.extend(leased.iter().copied());

            // ---- the lowest available number, three zero-padded digits ----
            let mut held: Vec<ChatNumberLease> = Vec::new();
            let mut issued: Vec<u16> = Vec::new();
            for step in 0..run_length {
                // The oracle: the lowest number in range that neither the
                // observation nor this run already holds.
                let mut taken = in_use.clone();
                taken.extend(issued.iter().copied());
                let want = (MIN_CHAT_NUMBER..=MAX_CHAT_NUMBER)
                    .find(|n| !taken.contains(n))
                    .expect("at most 22 numbers are held, so one is free");

                let lease = allocate_chat_number_in(
                    &dir,
                    &observed_numbers,
                    &leases,
                    &processes,
                )
                .expect("a free number exists");

                prop_assert_eq!(lease.number(), want, "step {}", step);
                let text = lease.number_text();
                let want_text = format!("{want:03}");
                prop_assert_eq!(text.as_str(), want_text.as_str());
                prop_assert_eq!(text.chars().count(), 3);
                prop_assert!(text.bytes().all(|b| b.is_ascii_digit()), "{}", text);
                let path = dir.join(lease_file_name(want));
                prop_assert_eq!(lease.path(), Some(path.as_path()));
                prop_assert!(path.exists(), "the claim leaves its lease behind");
                // Distinctness while the holder lives (21.2).
                prop_assert!(
                    !issued.contains(&want),
                    "number {} was issued twice while its holder lives",
                    want
                );
                issued.push(want);
                held.push(lease);
            }
            prop_assert_eq!(issued.len(), run_length);
            // Every number issued in this run is still held, simultaneously.
            for number in &issued {
                prop_assert!(dir.join(lease_file_name(*number)).exists());
                prop_assert!(!in_use.contains(number), "an in-use number was handed out");
            }
            // A number the observation reported is never touched by the
            // allocator: 10.3 falls out of availability, not bookkeeping.
            for number in &leased {
                prop_assert_eq!(
                    std::fs::read_to_string(dir.join(lease_file_name(*number)))
                        .unwrap_or_default(),
                    format!("{foreign}\n")
                );
            }

            // ---- a full set refuses, and claims nothing ----
            if check_exhaustion {
                let full = Sandbox::new("p7-full");
                let all: Vec<u16> = (MIN_CHAT_NUMBER..=MAX_CHAT_NUMBER).collect();
                let err = allocate_chat_number_in(&full.root, &all, &[], &processes)
                    .expect_err("every number in use must refuse");
                prop_assert!(err.contains("001..999"), "{}", err);
                prop_assert_eq!(
                    std::fs::read_dir(&full.root).expect("sandbox").count(),
                    0,
                    "a refusal creates nothing"
                );
            }

            // Release in the order a chat would, before the sandbox goes.
            drop(held);
            for number in &issued {
                prop_assert!(
                    !dir.join(lease_file_name(*number)).exists(),
                    "Drop releases every held number"
                );
            }
        }
    }

    proptest! {
        // 108 cases: 3 first harnesses × 3 second harnesses × 4 first models
        // × ... — well above the 100 floor, and every case writes and rewrites
        // a real config file.
        #![proptest_config(ProptestConfig { cases: 108, ..ProptestConfig::default() })]

        // **Validates: Requirements 11.3, 11.4, 21.3**
        // Feature: pitwall-chat-and-brief-ticker, Property 8: A descriptor never changes after capture — For any Chat_Descriptor and any subsequent mutation of the Config_Store, the descriptor's fields, its rendered header, its window title and its harness argv are unchanged.
        #[test]
        fn prop8_a_descriptor_never_changes_after_capture(
            first_agent_ix in 0usize..crate::agents::KNOWN.len(),
            second_agent_ix in 0usize..crate::agents::KNOWN.len(),
            first_model_ix in 0usize..MODEL_IDS.len(),
            second_model_ix in 0usize..MODEL_IDS.len(),
            number in 1u16..=999u16,
            label_ix in 0usize..CONTEXT_LABELS.len(),
            context in proptest::option::of(session_id()),
            epoch in 0i64..2_000_000_000i64,
            summary_enabled in any::<bool>(),
        ) {
            // **The honest form chosen here.** Requirement 11.4 talks about a
            // Config_Store mutation, and `chat.rs` deliberately never reads
            // one — so "mutate the Config_Store" is performed for real,
            // through `config::save_to` on a *sandbox* config file, and the
            // property is stated as: the mutation demonstrably lands (the
            // store reloads with the new values) and the descriptor captured
            // before it is byte-identical afterwards, field by field, and so
            // are its title, its header and its harness argv. The structural
            // half — that there is no code path by which a running chat could
            // re-read the store — is asserted below against the module's own
            // production source, which is the strongest form available from
            // inside the module.
            let sandbox = Sandbox::new("p8");
            let config_path = sandbox.root.join("config/pitwall/config");
            let first_agent = crate::agents::KNOWN[first_agent_ix].id;
            let second_agent = crate::agents::KNOWN[second_agent_ix].id;
            let first_model = MODEL_IDS[first_model_ix];
            let second_model = MODEL_IDS[second_model_ix];

            let mut cfg = crate::config::Config::default();
            cfg.set(crate::config::KEY_AGENT, first_agent).expect("a known agent");
            cfg.set(crate::config::KEY_MODEL, first_model).expect("a valid model");
            cfg.set(
                crate::config::KEY_SUMMARY_ENABLED,
                if summary_enabled { "true" } else { "false" },
            )
            .expect("a boolean toggle");
            crate::config::save_to(&config_path, &cfg).expect("sandbox config");

            // The one read, exactly as `cmd_chat` performs it at startup.
            let at_capture = crate::config::load_from(&config_path);
            let d = ChatDescriptor::capture(
                number,
                &at_capture.agent,
                &at_capture.model,
                CONTEXT_LABELS[label_ix],
                context.as_deref(),
                epoch,
                &sandbox.project_dir(),
            )
            .expect("the configured values are valid by construction");

            // Everything a later stage can observe about this descriptor.
            let fields_before = (
                d.number(),
                d.number_text(),
                d.harness().to_string(),
                d.model().to_string(),
                d.model_label().to_string(),
                d.context_label().to_string(),
                d.context_session_id().map(str::to_string),
                d.started_at_epoch(),
                d.project_dir().to_string(),
            );
            let title_before = format_title(&d);
            let header_before = render_header(&d, Palette::plain());
            let header_fields_before = header_fields(&d);
            let bin = sandbox.bin_dir.join(crate::agents::KNOWN[first_agent_ix].binary);
            let ctx = sandbox.runtime_dir.join("ctx-1-0123456789abcdef.json");
            let message = chat_message(&question_of("what is running?"));
            let argv_before = build_chat_call(&d, &bin, &ctx, &message)
                .expect("every known harness has a delivery row")
                .argv()
                .to_vec();

            // ---- mutate the Config_Store ----
            let mut mutated = crate::config::load_from(&config_path);
            mutated.set(crate::config::KEY_AGENT, second_agent).expect("a known agent");
            mutated.set(crate::config::KEY_MODEL, second_model).expect("a valid model");
            mutated
                .set(
                    crate::config::KEY_SUMMARY_ENABLED,
                    if summary_enabled { "false" } else { "true" },
                )
                .expect("a boolean toggle");
            crate::config::save_to(&config_path, &mutated).expect("config mutation");

            // The mutation really landed; otherwise this property would prove
            // nothing at all.
            let after_mutation = crate::config::load_from(&config_path);
            prop_assert_eq!(after_mutation.agent.as_str(), second_agent);
            prop_assert_eq!(after_mutation.model.as_str(), second_model);
            prop_assert_ne!(after_mutation.summary_enabled, at_capture.summary_enabled);

            // ---- and the descriptor did not move ----
            let fields_after = (
                d.number(),
                d.number_text(),
                d.harness().to_string(),
                d.model().to_string(),
                d.model_label().to_string(),
                d.context_label().to_string(),
                d.context_session_id().map(str::to_string),
                d.started_at_epoch(),
                d.project_dir().to_string(),
            );
            prop_assert_eq!(fields_after, fields_before);
            let title_after = format_title(&d);
            prop_assert_eq!(title_after.as_str(), title_before.as_str());
            let header_after = render_header(&d, Palette::plain());
            prop_assert_eq!(header_after.as_str(), header_before.as_str());
            prop_assert_eq!(header_fields(&d), header_fields_before);
            prop_assert_eq!(
                build_chat_call(&d, &bin, &ctx, &message)
                    .expect("still a known harness")
                    .argv()
                    .to_vec(),
                argv_before
            );
            // It still carries what the store said *at capture*, which is a
            // different value whenever the mutation was not a no-op (11.5:
            // the changed value belongs to the next chat, not this one).
            prop_assert_eq!(d.harness(), at_capture.agent.as_str());
            prop_assert_eq!(d.model(), at_capture.model.as_str());
            if first_agent != second_agent {
                prop_assert_ne!(d.harness(), after_mutation.agent.as_str());
            }
            if first_model != second_model {
                prop_assert_ne!(d.model(), after_mutation.model.as_str());
            }

            // The structural half: the module's production half names no
            // config reader, so no running chat can reach one (11.3, 11.4).
            const MODULE_SOURCE: &str = include_str!("chat.rs");
            let production = MODULE_SOURCE
                .split("#[cfg(test)]")
                .next()
                .expect("the module has a production half");
            prop_assert!(
                !production.contains("crate::config"),
                "chat.rs must contain no config read"
            );
        }
    }

    /// A string that is deliberately **not** a Chat_Title_Grammar title, one
    /// near miss per shape. Each is rejected by a different clause of
    /// [`parse_title`]; the property proves that claim rather than assuming
    /// it.
    fn malformed_title(
        shape: usize,
        number: u16,
        harness: &str,
        model_label: &str,
        label: &str,
    ) -> String {
        let sep = TITLE_SEPARATOR;
        match shape {
            // Four digits, never three.
            0 => format!("{CHAT_LABEL} {number:04}{sep}{harness}{sep}{model_label}{sep}{label}"),
            // Hyphen separators: the title then has one field, not four.
            1 => format!("{CHAT_LABEL} {number:03} - {harness} - {model_label} - {label}"),
            // A harness that is not a `crate::agents::KNOWN` id.
            2 => format!("{CHAT_LABEL} {number:03}{sep}notaharness{sep}{model_label}{sep}{label}"),
            // An empty context label.
            3 => format!("{CHAT_LABEL} {number:03}{sep}{harness}{sep}{model_label}{sep}"),
            // The reserved literal, mis-cased.
            4 => format!("Pitwall chat {number:03}{sep}{harness}{sep}{model_label}{sep}{label}"),
            // Five fields.
            5 => format!(
                "{CHAT_LABEL} {number:03}{sep}{harness}{sep}{model_label}{sep}{label}{sep}extra"
            ),
            // A model that is neither the agent-default literal nor valid.
            6 => format!("{CHAT_LABEL} {number:03}{sep}{harness}{sep}bad model{sep}{label}"),
            // A control character anywhere.
            7 => format!(
                "{CHAT_LABEL} {number:03}{sep}{harness}{sep}{model_label}{sep}{label}\u{0007}"
            ),
            // Not a title at all.
            _ => "user@host:~".to_string(),
        }
    }

    proptest! {
        // 216 cases: 3 harnesses × 5 model shapes × 8 label shapes × 9
        // malformed shapes is 1080 combinations, so 216 samples each shape
        // many times over while keeping the run cheap (the whole test is
        // pure apart from one sandbox directory per case).
        #![proptest_config(ProptestConfig { cases: 216, ..ProptestConfig::default() })]

        // **Validates: Requirements 16.2, 16.3, 16.4, 17.5**
        // Feature: pitwall-chat-and-brief-ticker, Property 11: Title formatting and parsing are inverses — For any Chat_Descriptor, `parse_title(format_title(d))` yields the same number, harness, model and context label; and for any string that is not a well-formed Pitwall Chat title, `parse_title` yields nothing.
        #[test]
        fn prop11_title_formatting_and_parsing_are_inverses(
            number in 1u16..=999u16,
            harness_ix in 0usize..crate::agents::KNOWN.len(),
            model_ix in 0usize..(MODEL_IDS.len() + 1),
            label_shape in 0usize..(CONTEXT_LABELS.len() + 2),
            malformed_shape in 0usize..9,
        ) {
            let sandbox = Sandbox::new("p11");
            let harness = crate::agents::KNOWN[harness_ix].id;
            // The extra model index is the longest id `valid_model` accepts
            // (128 ASCII characters) — the only case that makes the
            // 200-character title cap bite.
            let long_model = format!("p/{}", "a".repeat(126));
            let model = if model_ix == MODEL_IDS.len() {
                long_model.as_str()
            } else {
                MODEL_IDS[model_ix]
            };
            // The extra label shapes are the longest label `capture` accepts
            // and the longest one that is always emitted whole (39).
            let longest_label = "L".repeat(MAX_CONTEXT_LABEL_CHARS);
            let widest_safe_label = "M".repeat(39);
            let label = match label_shape {
                i if i < CONTEXT_LABELS.len() => CONTEXT_LABELS[i],
                i if i == CONTEXT_LABELS.len() => longest_label.as_str(),
                _ => widest_safe_label.as_str(),
            };
            let d = ChatDescriptor::capture(
                number,
                harness,
                model,
                label,
                None,
                0,
                &sandbox.project_dir(),
            )
            .expect("a generated descriptor is valid by construction");

            // ---- forward: every identity field comes back ----
            let title = format_title(&d);
            prop_assert!(
                title.chars().count() <= MAX_CHAT_TITLE_CHARS,
                "{} characters: {}",
                title.chars().count(),
                title
            );
            let parsed = parse_title(&title).expect("a formatted title must parse");
            prop_assert_eq!(parsed.number(), d.number());
            prop_assert_eq!(parsed.number_text(), d.number_text());
            prop_assert_eq!(parsed.harness(), d.harness());
            prop_assert_eq!(parsed.model(), d.model());
            // 16.3: an empty model presents as the agent-default literal on
            // the way out and parses back to empty on the way in.
            prop_assert_eq!(parsed.model_label(), d.model_label());
            prop_assert_eq!(d.model().is_empty(), title.contains(AGENT_DEFAULT_LABEL));

            // The one field the formatter may shorten is the context label,
            // and only to hold the title cap — so the inverse claim for it is
            // stated against `title_context_label`, which *is* what the
            // formatter emitted. Everything else is exact.
            let emitted_label = title_context_label(&d);
            prop_assert_eq!(parsed.context_label(), emitted_label.as_str());
            prop_assert!(
                d.context_label().starts_with(parsed.context_label()),
                "the label may lose its tail, never change: {:?} vs {:?}",
                d.context_label(),
                parsed.context_label()
            );
            // 16.4: a chat with no context session carries the workspace
            // literal, and the title says so.
            if d.context_session_id().is_none() && d.context_label() == WORKSPACE_LABEL {
                prop_assert_eq!(parsed.context_label(), WORKSPACE_LABEL);
            }

            // ---- the unconditional exact inverse ----
            let rendered = parsed.render();
            prop_assert_eq!(rendered.as_str(), title.as_str());
            prop_assert_eq!(parse_title(&rendered), Some(parsed.clone()));

            // ---- backward: nothing that is not the grammar parses ----
            let malformed = malformed_title(
                malformed_shape,
                number,
                harness,
                d.model_label(),
                parsed.context_label(),
            );
            prop_assert_ne!(malformed.as_str(), title.as_str());
            prop_assert!(
                parse_title(&malformed).is_none(),
                "must yield nothing for {:?}",
                malformed
            );
        }
    }

    // -----------------------------------------------------------------
    // Task 9.5 — Properties 14, 15, 17, 18 and 19
    //
    // Properties 14 and 18 quantify over a *sequence of chat inputs*, so
    // they need the input loop. The loop itself is `main.rs`'s `chat_loop`
    // (design §4.6), which cannot be called from a library test; [`drive`]
    // below is its dispatch table, arm for arm, over the `_in` seams — the
    // same classification, the same single caller of the resume bridge, the
    // same observe-then-respond order, and no other way to reach either.
    // -----------------------------------------------------------------

    /// The input kinds a human can type, one representative each. Indexes
    /// into this table are what the generators sample.
    const INPUT_KINDS: usize = 13;

    /// `/resume` with no argument.
    const KIND_RESUME_BARE: usize = 6;
    /// `/resume <target>`.
    const KIND_RESUME_TARGET: usize = 7;
    /// A *question* whose wording asks for a session to be resumed.
    const KIND_RESUME_WORDING: usize = 10;

    fn input_line(kind: usize, marker: &str, target: &str) -> String {
        match kind {
            0 => String::new(),
            1 => "   \t ".to_string(),
            2 => "/help".to_string(),
            3 => "/context".to_string(),
            4 => "/sessions".to_string(),
            5 => "/clear".to_string(),
            KIND_RESUME_BARE => "/resume".to_string(),
            KIND_RESUME_TARGET => format!("/resume {target}"),
            8 => "/nope --now".to_string(),
            9 => format!("what is {marker} doing?"),
            KIND_RESUME_WORDING => format!("please resume {target} for me, {marker}"),
            11 => format!("{}?", "a".repeat(MAX_CHAT_QUESTION_CHARS)),
            _ => "/exit".to_string(),
        }
    }

    /// The class each kind must produce — taken from the generation choice,
    /// never from a second reading of the classifier.
    fn expected_class(kind: usize) -> &'static str {
        match kind {
            0 | 1 => "blank",
            2..=5 => "info",
            KIND_RESUME_BARE | KIND_RESUME_TARGET => "act",
            8 => "unknown",
            9 | KIND_RESUME_WORDING => "question",
            11 => "too-long",
            _ => "end",
        }
    }

    /// How many inputs of `kinds` a loop actually processes: `/exit` returns,
    /// so everything after it is never read.
    fn processed_len(kinds: &[usize]) -> usize {
        kinds
            .iter()
            .position(|k| expected_class(*k) == "end")
            .map_or(kinds.len(), |i| i + 1)
    }

    const CLASSES: &[&str] = &[
        "blank", "info", "act", "end", "unknown", "question", "too-long",
    ];

    fn class_name(input: &ChatInput) -> &'static str {
        match input {
            ChatInput::Blank => "blank",
            ChatInput::Info(_) => "info",
            ChatInput::Act(_) => "act",
            ChatInput::End => "end",
            ChatInput::Unknown { .. } => "unknown",
            ChatInput::Question(_) => "question",
            ChatInput::QuestionTooLong { .. } => "too-long",
        }
    }

    /// The five classes Property 19 names, plus the two refinements
    /// [`classify`] documents: `Blank` is "no input at all" (neither a
    /// command nor a question), and `QuestionTooLong` is the Chat_Question
    /// class refused rather than truncated.
    const SPEC_CLASSES: &[&str] = &[
        "informational command",
        "resume command",
        "end command",
        "unknown command",
        "chat question",
        "no input",
    ];

    fn spec_class(input: &ChatInput) -> &'static str {
        match input {
            ChatInput::Info(_) => "informational command",
            ChatInput::Act(_) => "resume command",
            ChatInput::End => "end command",
            ChatInput::Unknown { .. } => "unknown command",
            ChatInput::Question(_) | ChatInput::QuestionTooLong { .. } => "chat question",
            ChatInput::Blank => "no input",
        }
    }

    /// What one driven turn did.
    struct TurnRecord {
        input: String,
        class: &'static str,
        /// Fake-harness invocations attributable to this turn.
        harness_calls: usize,
        /// [`resume_action`] calls attributable to this turn.
        resume_attempts: usize,
        /// Window focuses plus terminal launches during this turn.
        actions: usize,
        /// Did a fresh observation happen after this line was read and before
        /// the harness ran? Only meaningful for a question.
        observed_before_call: bool,
        /// The answer recorded for this turn, when it was a question.
        ///
        /// Captured per turn rather than read back out of the conversation at
        /// the end, because `/clear` empties the conversation by design
        /// (19.4): an answer erased afterwards was still answered here.
        answer: Option<String>,
    }

    struct DriveOutcome {
        turns: Vec<TurnRecord>,
        conversation: Conversation,
    }

    /// Run the input loop's dispatch table over `lines`.
    ///
    /// Arm for arm identical to `main.rs`'s `chat_loop`: `Blank` re-prompts,
    /// the informational arm observes read-only, the `Act` arm is the only
    /// caller of [`resume_action`], `End` returns, `Unknown` prints the
    /// vocabulary, and the question arm observes and then calls
    /// [`respond_in`] — which receives no platform, no database path and no
    /// resume handle, so it cannot act whatever the answer says.
    fn drive(
        plat: &RecordingPlatform,
        d: &ChatDescriptor,
        sandbox: &Sandbox,
        lines: &[String],
    ) -> DriveOutcome {
        let db = sandbox.db();
        let bin_dirs = sandbox.bin_dirs();
        let mut conversation = Conversation::new();
        let mut turns: Vec<TurnRecord> = Vec::new();
        let at = 1_700_000_200;

        for line in lines {
            let calls_before = sandbox.invocations();
            let actions_before = plat.actions();
            let observations_at_read = plat.observations();
            let mut resume_attempts = 0usize;
            let mut observed_before_call = false;
            let mut ended = false;
            let mut answer_text: Option<String> = None;

            let class = match classify(line) {
                ChatInput::Blank => "blank",
                ChatInput::Info(cmd) => {
                    match cmd {
                        InfoCommand::Help => {
                            conversation.record(Role::Pitwall, at, &render_vocabulary());
                        }
                        InfoCommand::Context => {
                            let observation = observe(plat, d, &sandbox.data_dir);
                            let mut text = context_block(d).join("\n");
                            text.push('\n');
                            text.push_str(observation.document());
                            conversation.record(Role::Pitwall, at, &text);
                        }
                        InfoCommand::Sessions => {
                            let scope = scope_to_context(
                                d.context_session_id(),
                                crate::collector::collect(plat),
                                Vec::new(),
                                Vec::new(),
                                Vec::new(),
                                Vec::new(),
                            );
                            let ids: Vec<&str> = scope
                                .snapshot()
                                .sessions
                                .iter()
                                .map(|s| s.id.as_str())
                                .collect();
                            conversation.record(Role::Pitwall, at, &ids.join("\n"));
                        }
                        InfoCommand::Clear => conversation.clear(),
                    }
                    "info"
                }
                ChatInput::Act(ActionCommand::Resume { target }) => {
                    resume_attempts += 1;
                    let report = resume_action(plat, &db, d, target.as_deref());
                    conversation.record(Role::Pitwall, at, &resume_line(&report));
                    "act"
                }
                ChatInput::End => {
                    ended = true;
                    "end"
                }
                ChatInput::Unknown { entered } => {
                    conversation.record(Role::Pitwall, at, &unknown_command_message(&entered));
                    "unknown"
                }
                ChatInput::Question(question) => {
                    conversation.record(Role::You, at, question.text());
                    let observation = observe(plat, d, &sandbox.data_dir);
                    observed_before_call = plat.observations() > observations_at_read;
                    let answer = match respond_in(
                        d,
                        &observation,
                        &question,
                        &sandbox.runtime_dir,
                        &bin_dirs,
                        Duration::from_secs(20),
                    ) {
                        Ok(answer) => answer,
                        Err(e) => e.message(),
                    };
                    conversation.record(Role::Pitwall, at, &answer);
                    answer_text = Some(answer);
                    "question"
                }
                ChatInput::QuestionTooLong { chars } => {
                    conversation.record(
                        Role::Pitwall,
                        at,
                        &format!("that question is {chars} characters; nothing was sent."),
                    );
                    "too-long"
                }
            };

            turns.push(TurnRecord {
                input: line.clone(),
                class,
                harness_calls: sandbox.invocations() - calls_before,
                resume_attempts,
                actions: plat.actions() - actions_before,
                observed_before_call,
                answer: answer_text,
            });
            if ended {
                break;
            }
        }

        DriveOutcome {
            turns,
            conversation,
        }
    }

    proptest! {
        // 100 cases (the floor): each case drives 1..4 inputs through the
        // real dispatch table and spawns the fake harness once per generated
        // question, so the case count is held at the minimum the design
        // requires rather than inflated.
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        // **Validates: Requirements 12.5, 12.6, 12.7**
        // Feature: pitwall-chat-and-brief-ticker, Property 14: The harness runs once per question and never otherwise — For any sequence of chat inputs, the number of harness invocations equals the number of Chat_Questions in that sequence, and each invocation is preceded by a fresh observation taken after its question was read.
        #[test]
        fn prop14_the_harness_runs_once_per_question_and_never_otherwise(
            kinds in proptest::collection::vec(0usize..INPUT_KINDS, 1..5),
            marker in word(),
            target in session_id(),
        ) {
            let answer = format!("answer-{marker}");
            let sandbox = Sandbox::with_harness("p14", &answer);
            let plat = RecordingPlatform::quiet();
            let d = descriptor_in(&sandbox, 5, "opencode", "", WORKSPACE_LABEL, None);

            // Startup: a descriptor, a header, a title. No harness (12.6).
            let _ = render_header(&d, Palette::plain());
            prop_assert_eq!(sandbox.invocations(), 0, "starting a chat runs no harness");

            let lines: Vec<String> = kinds
                .iter()
                .map(|k| input_line(*k, &marker, &target))
                .collect();
            let run = drive(&plat, &d, &sandbox, &lines);

            // `/exit` ends the loop, so the turns processed are the inputs up
            // to and including it.
            prop_assert_eq!(run.turns.len(), processed_len(&kinds));

            for (record, kind) in run.turns.iter().zip(kinds.iter()) {
                prop_assert_eq!(
                    record.class,
                    expected_class(*kind),
                    "{:?} was classified {}",
                    record.input,
                    record.class
                );
                // One invocation for a question, none for anything else: no
                // timer, no background run, no second call (12.6, 12.7).
                prop_assert_eq!(
                    record.harness_calls,
                    usize::from(record.class == "question"),
                    "{:?} ran the harness {} time(s); answer was {:?}",
                    record.input,
                    record.harness_calls,
                    record.answer
                );
                if record.class == "question" {
                    // 12.5: the observation is taken *after* the question was
                    // read and before the harness is invoked.
                    prop_assert!(
                        record.observed_before_call,
                        "{:?} answered without a fresh observation",
                        record.input
                    );
                }
            }

            let questions = run.turns.iter().filter(|t| t.class == "question").count();
            prop_assert_eq!(sandbox.invocations(), questions);
            // Every question got that harness's answer back, so the counted
            // invocations are the ones that actually answered.
            //
            // Read from the turn records, not from the conversation at the
            // end: `/clear` empties the conversation by design (19.4), and an
            // answer erased by a later command was still answered when its
            // question was asked. This also binds each answer to its own
            // question turn instead of counting matching turns anywhere in
            // the transcript, so it is the stricter of the two readings.
            let answered = run
                .turns
                .iter()
                .filter(|t| t.answer.as_deref() == Some(answer.as_str()))
                .count();
            prop_assert_eq!(answered, questions);
            // 12.8: nothing staged outlives the run, on any path.
            prop_assert!(sandbox.staged().is_empty(), "{:?}", sandbox.staged());
        }
    }

    /// Fragments a hostile question can be assembled from: shell
    /// metacharacters, flag-shaped words, absolute paths, a newline, a tab
    /// and quoting. None of them may become argv structure.
    const HOSTILE_FRAGMENTS: &[&str] = &[
        "; rm -rf ~",
        "&& id",
        "| sh",
        "$(id)",
        "`id`",
        "--dir /etc",
        "-m other/model",
        "/etc/passwd",
        "\n--format json\n",
        "\t-f /run/user/0/pitwall/ctx-1-a.json",
        "'\"quoted\"'",
        "sh -c 'echo hi'",
        "-p",
    ];

    proptest! {
        // 192 cases: 3 harnesses × 4 models × 13 hostile fragments in 1..5
        // positions. Pure — argv construction spawns nothing.
        #![proptest_config(ProptestConfig { cases: 192, ..ProptestConfig::default() })]

        // **Validates: Requirements 14.1, 14.2, 14.4, 27.4**
        // Feature: pitwall-chat-and-brief-ticker, Property 15: The question is contained in one argv element — For any question string — including shell metacharacters, leading hyphens, newlines and absolute paths — the harness argv gains exactly one element derived from it, that element begins with the fixed chat instruction, contains no control characters, is capped at the question character limit, and no additional flag, path or element appears; argv[0] is an absolute discovered binary and no element invokes a shell.
        #[test]
        fn prop15_the_question_is_contained_in_one_argv_element(
            marker in word(),
            fragment_ixs in proptest::collection::vec(0usize..HOSTILE_FRAGMENTS.len(), 1..5),
            harness_ix in 0usize..crate::agents::KNOWN.len(),
            model_ix in 0usize..MODEL_IDS.len(),
            trailing_spaces in 0usize..3,
        ) {
            let sandbox = Sandbox::new("p15");
            let harness = crate::agents::KNOWN[harness_ix].id;
            let binary_name = crate::agents::KNOWN[harness_ix].binary;
            let planted = fake_binary(&sandbox.bin_dir, binary_name);

            // argv[0] is whatever `discover_in` found on the search path —
            // never a caller-supplied string.
            let discovered = crate::agents::discover_in(&sandbox.bin_dirs())
                .into_iter()
                .find(|a| a.id == harness)
                .and_then(|a| a.path)
                .expect("a planted harness is discovered");
            prop_assert_eq!(discovered.as_path(), planted.as_path());
            prop_assert!(discovered.is_absolute(), "argv[0]: {:?}", discovered);

            // A question that begins with a letter (so it can never be a
            // command) and then carries every hostile shape at once.
            let mut text = marker.clone();
            for ix in &fragment_ixs {
                text.push(' ');
                text.push_str(HOSTILE_FRAGMENTS[*ix]);
            }
            text.push_str(&" ".repeat(trailing_spaces));
            let question = question_of(&text);
            let message = chat_message(&question);
            let ctx = sandbox.runtime_dir.join("ctx-1-0123456789abcdef.json");
            let d = descriptor_in(&sandbox, 3, harness, MODEL_IDS[model_ix], "Work", None);
            let call = build_chat_call(&d, &discovered, &ctx, &message)
                .expect("every known harness has a delivery row");
            let argv = call.argv().to_vec();

            // ---- exactly one element is derived from the question ----
            prop_assert_eq!(
                argv.iter().filter(|a| a.contains(marker.as_str())).count(),
                1,
                "{:?}",
                argv
            );
            prop_assert_eq!(argv.last().map(String::as_str), Some(message.as_str()));
            prop_assert!(message.starts_with(CHAT_INSTRUCTION));
            prop_assert!(message.contains(question.text()));
            for element in argv.iter().take(argv.len() - 1) {
                prop_assert!(
                    !element.contains(question.text()),
                    "the question leaked into {:?}",
                    element
                );
            }

            // ---- no additional flag, path or element appears ----
            // The oracle is design §4.6's table written out independently, so
            // "fixed" is compared against the fixed thing rather than
            // re-derived from the code under test.
            prop_assert_eq!(argv.clone(), expected_argv(&d, &discovered, &ctx, &message));
            // Every element before the message is either the discovered
            // binary or a literal from that table — never anything the human
            // typed (14.4).
            const FIXED: &[&str] = &["run", "--format", "json", "--dir", "-f", "-m", "-p", "exec"];
            for element in argv.iter().skip(1).take(argv.len() - 2) {
                let is_fixed = FIXED.contains(&element.as_str())
                    || element.as_str() == d.project_dir()
                    || element.as_str() == d.model()
                    || Path::new(element) == ctx.as_path();
                prop_assert!(is_fixed, "unexpected argv element {:?} in {:?}", element, argv);
            }

            // ---- the element carries no control character that could
            // re-enter an escape sequence ----
            // `context::strip_controls` deliberately keeps `\n` and `\t`:
            // they are content a human typed, not escapes. Those two are
            // therefore the only controls the element may carry, and an
            // `ESC`, `BEL` or `NUL` may not survive at all.
            for c in message.chars().filter(|c| c.is_control()) {
                prop_assert!(c == '\n' || c == '\t', "control {:?} reached argv", c);
            }
            prop_assert!(!message.contains('\u{1b}'), "ESC reached argv");
            prop_assert!(!message.contains('\u{7}'), "BEL reached argv");
            prop_assert!(!message.contains('\0'), "NUL reached argv");

            // ---- the cap ----
            prop_assert!(question.chars() >= 1);
            prop_assert!(question.chars() <= MAX_CHAT_QUESTION_CHARS);

            // ---- argv[0] is the discovered binary and nothing is a shell ----
            let discovered_text = discovered.to_string_lossy().into_owned();
            prop_assert_eq!(argv[0].as_str(), discovered_text.as_str());
            prop_assert_eq!(
                discovered.file_name().and_then(|n| n.to_str()),
                Some(binary_name)
            );
            for element in argv.iter().take(argv.len() - 1) {
                prop_assert!(
                    !matches!(
                        element.as_str(),
                        "sh" | "bash" | "zsh" | "dash" | "-c" | "/bin/sh" | "/bin/bash"
                    ),
                    "{:?} would invoke a shell",
                    element
                );
            }

            // ---- the document never travels in argv ----
            let staged = ctx.to_string_lossy().into_owned();
            let path_in_argv = argv.iter().any(|a| a.contains(&staged));
            prop_assert_eq!(path_in_argv, !call.context_on_stdin());
        }
    }

    proptest! {
        // 160 cases over 1..7 events drawn from four kinds (tool call,
        // metadata, text with content, text without) — pure, no harness runs.
        #![proptest_config(ProptestConfig { cases: 160, ..ProptestConfig::default() })]

        // **Validates: Requirements 13.5, 13.7**
        // Feature: pitwall-chat-and-brief-ticker, Property 17: Only text events survive extraction — For any interleaving of tool-call, metadata and text events in harness output, the presented answer contains no tool-call payload and no metadata field, and is empty when no text event carries content.
        #[test]
        fn prop17_only_text_events_survive_extraction(
            kinds in proptest::collection::vec(0usize..4, 1..7),
            marker in word(),
            trailing_newline in any::<bool>(),
        ) {
            // Three markers with distinct prefixes, so no assertion can be
            // satisfied by accident through a shared substring.
            let tool_marker = format!("tool{marker}");
            let meta_marker = format!("meta{marker}");
            let text_marker = format!("text{marker}");

            let mut raw = String::new();
            let mut expected: Vec<String> = Vec::new();
            for (i, kind) in kinds.iter().enumerate() {
                match kind {
                    // A tool call: its payload is the thing that must never
                    // be presented.
                    0 => raw.push_str(&format!(
                        "{{\"type\":\"tool\",\"name\":\"bash\",\"input\":\"{tool_marker}-{i} rm -rf /\"}}\n"
                    )),
                    // Metadata: counters and cost, no text-bearing field.
                    1 => raw.push_str(&format!(
                        "{{\"type\":\"meta\",\"tokens\":{i},\"cost\":\"{meta_marker}-{i}\"}}\n"
                    )),
                    // A text event carrying content.
                    2 => {
                        let body = format!("{text_marker}-{i}");
                        raw.push_str(&format!("{{\"type\":\"text\",\"text\":\"{body}\"}}\n"));
                        expected.push(body);
                    }
                    // A text event carrying nothing.
                    _ => raw.push_str("{\"type\":\"text\",\"text\":\"   \"}\n"),
                }
            }
            if trailing_newline {
                raw.push('\n');
            }

            let answer = present_answer(&crate::summary::extract_summary_text(&raw));

            // ---- no tool-call payload, no metadata field ----
            prop_assert!(!answer.contains(tool_marker.as_str()), "{answer}");
            prop_assert!(!answer.contains(meta_marker.as_str()), "{answer}");
            prop_assert!(!answer.contains("rm -rf"), "{answer}");
            prop_assert!(!answer.contains("tokens"), "{answer}");
            prop_assert!(!answer.contains("\"type\""), "{answer}");
            prop_assert!(!answer.contains("bash"), "{answer}");

            // ---- exactly the text events that carried content, in order ----
            let want_answer = expected.join("\n");
            prop_assert_eq!(answer.as_str(), want_answer.as_str());
            // ---- and empty exactly when none did (13.5) ----
            prop_assert_eq!(answer.is_empty(), expected.is_empty());
        }
    }

    proptest! {
        // 100 cases (the floor): each drives 1..4 inputs, opens a real
        // continuity store for the executor's Level 2 lookup and spawns the
        // fake harness once per generated question.
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        // **Validates: Requirements 14.5, 14.6, 14.10, 26.2, 26.3, 26.4, 26.5, 26.6, 27.1, 27.2, 27.3, 27.10**
        // Feature: pitwall-chat-and-brief-ticker, Property 18: Workspace state changes only from an entered resume command — For any sequence of chat inputs, the number of Resume_Action attempts equals the number of explicitly entered resume commands in that sequence, and the count of observed window-focus, terminal-launch and process-signal operations attributable to Chat_Questions, chat startup and context refreshes is zero — regardless of question wording, including wording that asks for a session to be resumed.
        #[test]
        fn prop18_workspace_state_changes_only_from_an_entered_resume_command(
            kinds in proptest::collection::vec(0usize..INPUT_KINDS, 1..5),
            marker in word(),
            unknown_target in session_id(),
            target_shape in 0usize..3,
        ) {
            let answer = format!("answer-{marker}");
            let sandbox = Sandbox::with_harness("p18", &answer);
            // One observed window, so a `/resume` that names it can actually
            // focus something: "zero actions from a question" is a weaker
            // claim on a platform where nothing is focusable.
            let plat = RecordingPlatform::with_windows(vec![window(
                "0x1",
                "foot",
                "user@host:~",
                100,
            )]);
            let live = crate::collector::collect(&plat)
                .sessions
                .first()
                .map(|s| s.id.clone())
                .expect("one observed window is one session");

            // The target an entered `/resume <id>` carries: the live session
            // (focus succeeds), a valid-shaped session nothing knows about
            // (the executor refuses), or a malformed one (the bridge refuses
            // before the executor).
            let target = match target_shape {
                0 => live.clone(),
                1 => unknown_target.clone(),
                _ => format!("not-a-session-{marker}"),
            };

            // ---- chat startup starts no Resume_Action attempt (26.5) ----
            let d = descriptor_in(&sandbox, 6, "opencode", "", WORKSPACE_LABEL, None);
            let _ = render_header(&d, Palette::plain());
            let _ = plan_branding(plat.inline_image_capability(), None);
            prop_assert_eq!(plat.actions(), 0, "starting a chat changes nothing");

            let lines: Vec<String> = kinds
                .iter()
                .map(|k| input_line(*k, &marker, &target))
                .collect();
            let run = drive(&plat, &d, &sandbox, &lines);
            prop_assert_eq!(run.turns.len(), processed_len(&kinds));

            let mut expected_actions = 0usize;
            for (record, kind) in run.turns.iter().zip(kinds.iter()) {
                prop_assert_eq!(record.class, expected_class(*kind), "{:?}", record.input);
                if record.class == "act" {
                    // Exactly one attempt per entered resume command (26.2).
                    prop_assert_eq!(record.resume_attempts, 1, "{:?}", record.input);
                    // `/resume` alone has no target in a workspace-scoped
                    // chat, so it refuses without reaching the executor;
                    // `/resume <live id>` focuses exactly one window.
                    let acts = *kind == KIND_RESUME_TARGET && target_shape == 0;
                    prop_assert_eq!(
                        record.actions,
                        usize::from(acts),
                        "{:?} performed {} operation(s)",
                        record.input,
                        record.actions
                    );
                    expected_actions += usize::from(acts);
                } else {
                    // A question, a context refresh, an informational
                    // command, an unknown command and a blank line change
                    // nothing at all (14.5, 27.1, 27.2, 27.10).
                    prop_assert_eq!(
                        record.resume_attempts,
                        0,
                        "{:?} attempted a Resume_Action",
                        record.input
                    );
                    prop_assert_eq!(
                        record.actions,
                        0,
                        "{:?} performed {} workspace operation(s)",
                        record.input,
                        record.actions
                    );
                }
            }

            // Attempts equal entered resume commands, exactly.
            let entered_resumes = run.turns.iter().filter(|t| t.class == "act").count();
            prop_assert_eq!(
                run.turns.iter().map(|t| t.resume_attempts).sum::<usize>(),
                entered_resumes
            );

            // 27.3: wording never becomes behaviour. A question that *asks*
            // for a resume is answered and nothing else.
            for (record, _) in run
                .turns
                .iter()
                .zip(kinds.iter())
                .filter(|(_, k)| **k == KIND_RESUME_WORDING)
            {
                prop_assert_eq!(record.class, "question");
                prop_assert_eq!(record.resume_attempts, 0);
                prop_assert_eq!(record.actions, 0);
                // The recorded answer names the refusal class when the
                // harness was not reached, which is the difference between a
                // production defect and a non-hermetic fixture.
                prop_assert_eq!(record.harness_calls, 1, "answer was {:?}", record.answer);
            }

            // The totals, from the platform's own recording.
            prop_assert_eq!(plat.actions(), expected_actions);
            prop_assert_eq!(plat.focused.borrow().len(), expected_actions);
            prop_assert!(
                plat.launched.borrow().is_empty(),
                "no chat input opens a terminal on this workspace"
            );
            // 26.20: nothing this loop launches ever carries a command, so no
            // agent can start inside one.
            prop_assert!(plat.launched.borrow().iter().all(|l| l.command.is_empty()));
            // The only child process any of this creates is the one harness
            // run per question — there is no other spawn on the path, and the
            // `Platform` trait exposes no process-signal capability at all.
            let questions = run.turns.iter().filter(|t| t.class == "question").count();
            prop_assert_eq!(sandbox.invocations(), questions);
        }
    }

    proptest! {
        // 256 cases: three independently generated inputs per case (a pooled
        // representative, a free-form line and a `/`-prefixed word) against a
        // pure classifier.
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        // **Validates: Requirements 14.7, 14.8, 26.1, 27.5, 27.8**
        // Feature: pitwall-chat-and-brief-ticker, Property 19: Input classification is total and the vocabulary is closed — For any input line, classification yields exactly one of informational command, resume command, end command, unknown command or Chat_Question; every input not beginning with `/` is a Chat_Question; every `/`-prefixed input outside the closed vocabulary yields the vocabulary listing and no harness invocation; and the listing marks each entry as read-only or as changing workspace state, matching its class.
        #[test]
        fn prop19_input_classification_is_total_and_the_vocabulary_is_closed(
            kind in 0usize..INPUT_KINDS,
            marker in word(),
            target in session_id(),
            junk in proptest::string::string_regex("[a-zA-Z0-9 /_.:;|&$`'\"?-]{0,24}")
                .expect("static regex"),
            slash_word in proptest::string::string_regex("[a-zA-Z][a-zA-Z0-9-]{0,10}")
                .expect("static regex"),
            leading_spaces in 0usize..3,
        ) {
            let pooled = input_line(kind, &marker, &target);
            let free = format!("{}{}", " ".repeat(leading_spaces), junk);
            let slashed = format!("/{slash_word}");
            let listing = render_vocabulary();

            for line in [pooled.as_str(), free.as_str(), slashed.as_str()] {
                let input = classify(line);

                // ---- total: exactly one class, and exactly one spec class --
                let class = class_name(&input);
                prop_assert_eq!(CLASSES.iter().filter(|c| **c == class).count(), 1);
                prop_assert_eq!(
                    SPEC_CLASSES
                        .iter()
                        .filter(|c| **c == spec_class(&input))
                        .count(),
                    1
                );
                // Acting is a property of the variant, not of a flag (14.10).
                prop_assert_eq!(input.changes_workspace_state(), class == "act");
                prop_assert_eq!(input.invokes_harness(), class == "question");

                // ---- the vocabulary is closed, and the listing agrees ----
                let named = input.vocabulary_entry().is_some();
                prop_assert_eq!(named, matches!(class, "info" | "act" | "end"));
                prop_assert_eq!(input.effect().is_some(), named);
                if let Some(entry) = input.vocabulary_entry() {
                    let effect = input.effect().expect("a named entry has an effect");
                    prop_assert_eq!(entry.effect, effect);
                    prop_assert_eq!(effect.changes_workspace_state(), class == "act");
                    // 27.8: the entry's own line carries its own mark, and
                    // not the other one.
                    let listed = listing
                        .lines()
                        .find(|l| l.trim_start().starts_with(entry.name))
                        .expect("every entry is listed");
                    prop_assert!(listed.contains(effect.marker()), "{listed}");
                    let other = if effect.changes_workspace_state() {
                        Effect::ReadOnly
                    } else {
                        Effect::ChangesWorkspaceState
                    };
                    prop_assert!(!listed.contains(other.marker()), "{listed}");
                    prop_assert!(listed.contains(entry.help), "{listed}");
                }

                // ---- not `/`-prefixed ⇒ never a command (27.5) ----
                if !line.trim().starts_with('/') {
                    prop_assert!(
                        matches!(class, "blank" | "question" | "too-long"),
                        "{:?} became {}",
                        line,
                        class
                    );
                }

                // ---- `/`-prefixed outside the vocabulary ⇒ the listing, and
                // nothing invoked (14.8) ----
                if let ChatInput::Unknown { entered } = &input {
                    let msg = unknown_command_message(entered);
                    prop_assert!(msg.contains("nothing was run"), "{msg}");
                    prop_assert!(msg.ends_with(listing.as_str()), "{msg}");
                    for vocab in VOCABULARY {
                        prop_assert!(msg.contains(vocab.name), "{msg}");
                        prop_assert!(msg.contains(vocab.effect.marker()), "{msg}");
                    }
                    prop_assert!(!input.invokes_harness());
                    prop_assert!(!input.changes_workspace_state());
                }
            }

            // A bare `/`-prefixed word is an unknown command exactly when it
            // is not a vocabulary entry — every entry accepts zero arguments,
            // so the equivalence is total for this shape.
            let in_vocabulary = VOCABULARY.iter().any(|e| e.name == slashed.as_str());
            prop_assert_eq!(
                matches!(classify(&slashed), ChatInput::Unknown { .. }),
                !in_vocabulary
            );

            // Exactly one entry may change workspace state, and it is the
            // resume command (26.1).
            let acting: Vec<&str> = VOCABULARY
                .iter()
                .filter(|e| e.effect.changes_workspace_state())
                .map(|e| e.name)
                .collect();
            prop_assert_eq!(acting, vec!["/resume"]);
        }
    }

    // -----------------------------------------------------------------
    // Task 9.6 — Properties 13, 20, 21, 26 and 27
    // -----------------------------------------------------------------

    use std::collections::HashMap;

    /// One synthetic observed session. Direct construction rather than
    /// `collect`, because these properties need to *choose* the session set,
    /// the projects and the recorded confidences.
    fn synth_session(
        index: usize,
        project_ix: usize,
        confidence: crate::collector::Confidence,
    ) -> crate::collector::TerminalSession {
        let pid = 100 + index as u32;
        crate::collector::TerminalSession {
            id: format!("sess_{index:016x}"),
            window: Some(window(
                &format!("0x{}", index + 1),
                "foot",
                "user@host:~",
                pid,
            )),
            root_pid: pid,
            role: crate::collector::WindowRole::Terminal,
            project: Some(crate::collector::ProjectInfo {
                id: format!("proj_{project_ix:016x}"),
                dir: format!("/home/u/P{project_ix}"),
                name: format!("P{project_ix}"),
                is_git_repo: false,
                branch: None,
                git_clean: None,
            }),
            agent: crate::collector::AgentIdentity {
                kind: crate::collector::AgentKind::Unknown,
                confidence,
                evidence: Vec::new(),
            },
            chat: None,
            state: crate::collector::SessionState::Sleeping,
            process_count: 1,
            processes: Vec::new(),
            last_activity_epoch: 1_700_000_000 + index as i64,
            last_activity_kind: crate::collector::LAST_ACTIVITY_KIND,
            summary: format!("session {index}"),
        }
    }

    fn synth_snapshot(
        sessions: Vec<crate::collector::TerminalSession>,
    ) -> crate::collector::WorkspaceSnapshot {
        crate::collector::WorkspaceSnapshot {
            schema_version: crate::collector::SNAPSHOT_SCHEMA_VERSION,
            collected_at_epoch: 1_700_000_100,
            hostname: "testbox".to_string(),
            sessions,
        }
    }

    fn synth_checkpoint(
        id: i64,
        session_id: &str,
        project_id: &str,
        project_dir: &str,
    ) -> crate::store::Checkpoint {
        crate::store::Checkpoint {
            id,
            created_at: 1_700_000_000 + id,
            project_id: project_id.to_string(),
            session_id: session_id.to_string(),
            project_dir: project_dir.to_string(),
            branch: None,
            git_clean: None,
            agent_kind: "unknown".to_string(),
            agent_confidence: "low".to_string(),
            state: "sleeping".to_string(),
            last_activity_epoch: 1_700_000_050,
            window_address: None,
            window_class: None,
            note: None,
            trigger: "manual".to_string(),
            observation_id: None,
        }
    }

    fn synth_notification(
        id: i64,
        session_id: &str,
        project_id: &str,
    ) -> crate::store::Notification {
        crate::store::Notification {
            id,
            kind: "attention".to_string(),
            session_id: session_id.to_string(),
            project_id: project_id.to_string(),
            project_name: "P0".to_string(),
            branch: None,
            agent_kind: "unknown".to_string(),
            state: "sleeping".to_string(),
            checkpoint_id: None,
            created_at: 1_700_000_000 + id,
            severity: "info".to_string(),
            detail: "idle".to_string(),
            read_at: None,
        }
    }

    fn synth_prev(
        session_id: &str,
        project_id: &str,
        project_dir: &str,
    ) -> crate::store::PrevSession {
        crate::store::PrevSession {
            session_id: session_id.to_string(),
            project_id: Some(project_id.to_string()),
            project_dir: Some(project_dir.to_string()),
            agent_kind: "unknown".to_string(),
            branch: None,
            git_clean: None,
            state: "sleeping".to_string(),
        }
    }

    /// The session blocks of a bounded context document, in order.
    ///
    /// Exact rather than approximate: `render_document` opens each block with
    /// `"  {\n"`, and no field value can contain that run because every value
    /// goes through `output::escape`, which turns a newline into `\n`.
    fn document_session_blocks(document: &str) -> Vec<&str> {
        let region = document
            .split("\n],\"events\":[")
            .next()
            .unwrap_or_default();
        region.split("  {\n").skip(1).collect()
    }

    /// The value of a JSON string field in a session block.
    fn json_string_of(block: &str, field: &str) -> Option<String> {
        let needle = format!("\"{field}\": \"");
        let start = block.find(&needle)? + needle.len();
        let rest = &block[start..];
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    }

    /// Element count of a JSON array field in a session block. The generated
    /// terminal lines carry no comma, so counting separators is exact.
    fn json_array_len(block: &str, field: &str) -> Option<usize> {
        let needle = format!("\"{field}\": [");
        let start = block.find(&needle)? + needle.len();
        let rest = &block[start..];
        let end = rest.find(']')?;
        let body = &rest[..end];
        if body.trim().is_empty() {
            return Some(0);
        }
        Some(body.matches(',').count() + 1)
    }

    /// Lines of a `stable_serialized()` context that open with `prefix`.
    fn stable_lines(stable: &str, prefix: &str) -> usize {
        stable.lines().filter(|l| l.starts_with(prefix)).count()
    }

    proptest! {
        // 128 cases: 1..8 sessions over 3 projects × 4 recorded confidences ×
        // 0..26 terminal lines × 0..14 checkpoints × 0..25 notifications ×
        // 0..30 vanished sessions × 3 scope shapes.
        #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

        // **Validates: Requirements 12.2, 12.3, 12.4, 13.4, 15.3, 21.4**
        // Feature: pitwall-chat-and-brief-ticker, Property 13: Chat context stays inside the existing bounds and scope — For any observed workspace, the context a Chat_Session builds contains at most 6 sessions, 20 derived events, 10 checkpoints and 16 KB of document, carries each kept session's recorded agent confidence, carries at most the bounded first/last terminal-line window, reports any omission honestly, and includes the descriptor's context session and its project whenever that session is observed.
        #[test]
        fn prop13_chat_context_stays_inside_the_existing_bounds_and_scope(
            facts in proptest::collection::vec((0usize..3, 0usize..4), 1..8),
            scroll_lines in 0usize..26,
            checkpoint_count in 0usize..14,
            notification_count in 0usize..25,
            vanished in 0usize..30,
            scope_choice in 0usize..3,
            scope_pick in 0usize..8,
        ) {
            let sandbox = Sandbox::new("p13");
            const CONFIDENCES: &[crate::collector::Confidence] = &[
                crate::collector::Confidence::High,
                crate::collector::Confidence::Medium,
                crate::collector::Confidence::Low,
                crate::collector::Confidence::Unknown,
            ];
            let sessions: Vec<crate::collector::TerminalSession> = facts
                .iter()
                .enumerate()
                .map(|(i, (project_ix, conf_ix))| synth_session(i, *project_ix, CONFIDENCES[*conf_ix]))
                .collect();
            let snapshot = synth_snapshot(sessions.clone());

            // Bounded terminal text, produced by the *same* function the
            // platform uses at the observation boundary
            // (`context::window_lines`), so "at most the bounded first/last
            // window" is checked against the bound itself.
            let lines: Vec<String> = (0..scroll_lines).map(|i| format!("line-{i}")).collect();
            let (want_first, want_last) = crate::context::window_lines(&lines);
            let plat = RecordingPlatform {
                windows: (0..sessions.len())
                    .map(|i| window(&format!("0x{}", i + 1), "foot", "user@host:~", 100 + i as u32))
                    .collect(),
                text: TerminalText::Lines {
                    first: want_first,
                    last: want_last,
                },
                io: Some(IoCounters {
                    read_bytes: 1024,
                    write_bytes: 2048,
                }),
                ..RecordingPlatform::quiet()
            };

            // Store rows: some belonging to the scoped project, some not.
            let checkpoints: Vec<crate::store::Checkpoint> = (0..checkpoint_count)
                .map(|i| {
                    let owner = i % sessions.len();
                    synth_checkpoint(
                        i as i64 + 1,
                        &format!("sess_{owner:016x}"),
                        &format!("proj_{:016x}", facts[owner].0),
                        &format!("/home/u/P{}", facts[owner].0),
                    )
                })
                .collect();
            let notifications: Vec<crate::store::Notification> = (0..notification_count)
                .map(|i| {
                    let owner = i % sessions.len();
                    synth_notification(
                        i as i64 + 1,
                        &format!("sess_{owner:016x}"),
                        &format!("proj_{:016x}", facts[owner].0),
                    )
                })
                .collect();
            // Previous-observation rows for sessions that are gone: each is
            // one derived event, which is how the 20-event cap is made to
            // bite rather than assumed.
            let prev: Vec<crate::store::PrevSession> = (0..vanished)
                .map(|i| {
                    synth_prev(
                        &format!("sess_{:016x}", 1000 + i),
                        &format!("proj_{:016x}", i % 3),
                        &format!("/home/u/P{}", i % 3),
                    )
                })
                .collect();

            // The scope: no context session, an observed one, or one that
            // ended since the chat started.
            let observed_ids: Vec<String> = sessions.iter().map(|s| s.id.clone()).collect();
            let scoped_id: Option<String> = match scope_choice {
                0 => None,
                1 => Some(observed_ids[scope_pick % observed_ids.len()].clone()),
                _ => Some("sess_ffffffffffffffff".to_string()),
            };
            let scoped_project: Option<String> = scoped_id.as_ref().and_then(|id| {
                sessions
                    .iter()
                    .find(|s| &s.id == id)
                    .and_then(|s| s.project.as_ref().map(|p| p.id.clone()))
            });

            // The unscoped event derivation, as an oracle for the cap.
            let uncapped_events = crate::context::derive_events(
                &prev,
                &snapshot,
                &[],
                usize::MAX,
            )
            .len();

            let scope = scope_to_context(
                scoped_id.as_deref(),
                snapshot.clone(),
                prev.clone(),
                Vec::new(),
                checkpoints.clone(),
                notifications.clone(),
            );
            let scope_ids: Vec<String> =
                scope.snapshot().sessions.iter().map(|s| s.id.clone()).collect();
            let scoped_checkpoints = scope.checkpoints().len();
            let scoped_notifications = scope.notifications().len();
            let scoped_prev = scope.prev_sessions().len();

            // ---- scope (12.3, 12.4, 21.4) ----
            match &scoped_id {
                // The whole observed workspace, unfiltered.
                None => prop_assert_eq!(scope_ids.clone(), observed_ids.clone()),
                Some(id) if observed_ids.contains(id) => {
                    // That session and its project: every session sharing the
                    // project id, and nothing else.
                    let want: Vec<String> = sessions
                        .iter()
                        .filter(|s| {
                            &s.id == id
                                || s.project.as_ref().map(|p| &p.id) == scoped_project.as_ref()
                        })
                        .map(|s| s.id.clone())
                        .collect();
                    prop_assert_eq!(scope_ids.clone(), want);
                    prop_assert!(scope_ids.contains(id));
                }
                // The scope's subject ended: no live session is in scope, and
                // the context does not widen back to the whole workspace.
                Some(_) => prop_assert!(scope_ids.is_empty()),
            }

            let context = scope.into_summary_context();
            let stable = context.stable_serialized();
            let events = stable_lines(&stable, "event=");

            // ---- the caps `cmd_summarize` applies (12.2) ----
            prop_assert!(events <= MAX_CHAT_EVENTS, "{} events", events);
            if scoped_id.is_none() {
                prop_assert_eq!(events, uncapped_events.min(MAX_CHAT_EVENTS));
            }
            prop_assert_eq!(
                stable_lines(&stable, "checkpoint="),
                scoped_checkpoints.min(MAX_CHAT_CHECKPOINTS)
            );
            prop_assert!(stable_lines(&stable, "checkpoint=") <= MAX_CHAT_CHECKPOINTS);
            prop_assert_eq!(
                stable_lines(&stable, "notification="),
                scoped_notifications.min(MAX_CHAT_NOTIFICATIONS)
            );
            prop_assert!(scoped_prev <= prev.len());

            let (document, truncated) =
                crate::context::build_context_from_summary(&plat, &context);

            // ---- 16 KB, 6 sessions, and an honest omission count ----
            prop_assert!(
                document.len() <= crate::context::MAX_CONTEXT_BYTES,
                "{} bytes",
                document.len()
            );
            let blocks = document_session_blocks(&document);
            prop_assert!(blocks.len() <= crate::context::MAX_SESSIONS);
            prop_assert_eq!(blocks.len(), scope_ids.len().min(crate::context::MAX_SESSIONS));
            prop_assert_eq!(truncated, scope_ids.len() - blocks.len());
            prop_assert_eq!(truncated > 0, scope_ids.len() > blocks.len());
            // Bound to a local and asserted with an explicit message: a
            // single-argument `prop_assert!` turns `stringify!` of the
            // expression into its format string, and a `{...}` inside that
            // expression would then be read as a placeholder.
            let truncated_field = format!("\"truncated_sessions\":{truncated}");
            prop_assert!(
                document.contains(&truncated_field),
                "the document must report {}",
                truncated_field
            );

            for block in &blocks {
                let id = json_string_of(block, "id").expect("every block names its session");
                prop_assert!(scope_ids.contains(&id), "{} is out of scope", id);
                // ---- each kept session's *recorded* confidence (13.4) ----
                let want = sessions
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.agent.confidence.as_str())
                    .expect("a kept session is an observed session");
                let recorded = json_string_of(block, "agent_confidence");
                prop_assert_eq!(recorded.as_deref(), Some(want));
                // ---- at most the bounded first/last terminal window (15.3) --
                match crate::context::window_lines(&lines) {
                    (first, last) if lines.is_empty() => {
                        prop_assert!(first.is_empty() && last.is_empty());
                        prop_assert_eq!(json_array_len(block, "term_first"), Some(0));
                        prop_assert_eq!(json_array_len(block, "term_last"), Some(0));
                    }
                    (first, last) => {
                        prop_assert_eq!(json_array_len(block, "term_first"), Some(first.len()));
                        prop_assert_eq!(json_array_len(block, "term_last"), Some(last.len()));
                        prop_assert!(first.len() <= crate::context::WINDOW_LINES);
                        prop_assert!(last.len() <= crate::context::WINDOW_LINES);
                    }
                }
            }

            // ---- the descriptor's context session appears when it is
            // observed and the scoped set fits the bound ----
            if let Some(id) = &scoped_id {
                if observed_ids.contains(id) && scope_ids.len() <= crate::context::MAX_SESSIONS {
                    prop_assert!(
                        document.contains(id.as_str()),
                        "the context session must be in the document"
                    );
                    if let Some(project) = &scoped_project {
                        let name = sessions
                            .iter()
                            .find(|s| &s.id == id)
                            .and_then(|s| s.project.as_ref().map(|p| p.name.clone()))
                            .expect("the scoped session has a project");
                        prop_assert!(document.contains(&name), "its project must be named");
                        prop_assert!(!project.is_empty());
                    }
                }
            }

            // ---- and the production observing path holds the same bounds ---
            // `observe` performs the collect and the degradable store reads;
            // the sandbox holds no database, so those reads yield nothing and
            // none is created (15.6).
            let d = descriptor_in(&sandbox, 2, "opencode", "", WORKSPACE_LABEL, None);
            let live = observe(&plat, &d, &sandbox.data_dir);
            prop_assert!(live.document().len() <= crate::context::MAX_CONTEXT_BYTES);
            let live_blocks = document_session_blocks(live.document()).len();
            prop_assert!(live_blocks <= crate::context::MAX_SESSIONS);
            prop_assert_eq!(
                live.truncated_sessions(),
                plat.windows.len().saturating_sub(live_blocks)
            );
            prop_assert!(!sandbox.db().exists(), "chat opens no database that is absent");
        }
    }

    proptest! {
        // 128 cases over generated secret-shaped, environment-shaped,
        // command-line-shaped, address-shaped and scrollback-shaped values.
        #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

        // **Validates: Requirements 15.1, 15.2, 15.5, 22.7, 26.25**
        // Feature: pitwall-chat-and-brief-ticker, Property 20: Excluded value classes never appear in presented or transmitted values — For any observed workspace and any conversation, no environment value, credential, token, private key, raw process command line, process identifier, window address, shell history line or unbounded scrollback appears in the Chat_Session's presented lines, in the document transmitted to the harness, or in the chat fields of `state.json`; question and answer text are both scrubbed by the existing secret-scrubbing function.
        #[test]
        fn prop20_excluded_value_classes_never_appear_in_presented_or_transmitted_values(
            marker in word(),
            harness_ix in 0usize..crate::agents::KNOWN.len(),
            model_ix in 0usize..MODEL_IDS.len(),
            label_ix in 0usize..CONTEXT_LABELS.len(),
            number in 1u16..=999u16,
            secret_shape in 0usize..3,
            context in proptest::option::of(session_id()),
        ) {
            let sandbox = Sandbox::new("p20");
            // **Every secret-shaped value is assembled at runtime**, so the
            // repository carries no credential-shaped literal and the CI
            // secret scan stays clean — the same technique `context.rs`'s
            // tests use.
            let token = ["ghp", "0123456789abcdefghij"].join("_");
            let key_value = format!("AbCd1234{}", marker.to_uppercase());
            let key_form = format!("{}={}", "token", key_value);
            let pem = format!(
                "{}{}{}",
                "-----BEGIN ", "RSA PRIVATE KEY-----\nMIIabc", "def\n-----END RSA PRIVATE KEY-----\n"
            );
            let (secret, must_vanish) = match secret_shape {
                0 => (token.clone(), token.clone()),
                1 => (key_form.clone(), key_value.clone()),
                _ => (pem.clone(), "MIIabc".to_string()),
            };
            // The other excluded classes, each with a unique marker so its
            // absence is a substring search rather than an argument.
            let env_value = format!("PITWALL_TEST_ENV=env-{marker}");
            let command_line = format!("/usr/bin/opencode --auto {env_value}");
            let window_address = format!("0xdead{marker}");
            let pid = 987_654u32;
            let history_line = format!("history-{marker} export {key_form}");

            // 40 scrollback lines: the middle ones (including the shell
            // history line) are outside the bounded window, and a kept one
            // carries a secret so the scrubber is exercised on the way out.
            let scrollback: Vec<String> = (0..40)
                .map(|i| match i {
                    2 => format!("kept-{marker}-{i} {key_form}"),
                    15 => history_line.clone(),
                    i if (10..30).contains(&i) => format!("middle-{marker}-{i}"),
                    i => format!("kept-{marker}-{i}"),
                })
                .collect();
            let (first, last) = crate::context::window_lines(&scrollback);

            // The observed workspace: one chat window carrying the address,
            // the pid and the raw command line.
            let harness = crate::agents::KNOWN[harness_ix].id;
            let model = MODEL_IDS[model_ix];
            let label = CONTEXT_LABELS[label_ix];
            let mut session = synth_session(0, 0, crate::collector::Confidence::Low);
            session.window = Some(window(
                &window_address,
                "foot",
                &format!(
                    "{CHAT_LABEL} {number:03} \u{00b7} {harness} \u{00b7} {} \u{00b7} {label}",
                    if model.is_empty() { AGENT_DEFAULT_LABEL } else { model }
                ),
                pid,
            ));
            session.root_pid = pid;
            session.role = crate::collector::WindowRole::Chat;
            session.processes = vec![crate::collector::ProcessInfo {
                pid,
                ppid: 1,
                name: "opencode".to_string(),
                command: command_line.clone(),
                exe_name: "opencode".to_string(),
                cwd: "/home/u/P0".to_string(),
                state: crate::collector::ProcessState::Sleeping,
                started_at_epoch: 1_700_000_050,
            }];
            session.chat = Some(crate::collector::ChatFacts {
                number,
                harness: harness.to_string(),
                model: model.to_string(),
                context_label: label.to_string(),
                context_session_id: context.clone(),
                started_at_epoch: 1_700_000_100,
            });
            let snapshot = synth_snapshot(vec![session]);
            let plat = RecordingPlatform {
                windows: vec![window(&window_address, "foot", "user@host:~", pid)],
                text: TerminalText::Lines { first, last },
                io: Some(IoCounters {
                    read_bytes: 1024,
                    write_bytes: 2048,
                }),
                ..RecordingPlatform::quiet()
            };

            // ---- the document transmitted to the harness ----
            let scope = scope_to_context(
                None,
                snapshot.clone(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            );
            let (document, _) =
                crate::context::build_context_from_summary(&plat, &scope.into_summary_context());
            for excluded in [
                must_vanish.as_str(),
                env_value.as_str(),
                command_line.as_str(),
                window_address.as_str(),
                "987654",
                history_line.as_str(),
            ] {
                prop_assert!(
                    !document.contains(excluded),
                    "excluded value {:?} reached the transmitted document",
                    excluded
                );
            }
            // Unbounded scrollback: the middle of the buffer is not there.
            for i in 10..30 {
                let middle = format!("middle-{marker}-{i}");
                prop_assert!(
                    !document.contains(&middle),
                    "scrollback line {} reached the transmitted document",
                    middle
                );
            }
            // A secret inside an otherwise-safe field is redacted, not dropped
            // silently: the line is still there, its secret is not.
            let kept = format!("kept-{marker}-2");
            prop_assert!(
                document.contains(&kept),
                "the scrubbed line must survive; {} is missing",
                kept
            );
            prop_assert!(document.contains("[redacted]"), "{document}");

            // ---- the chat fields of state.json (22.7) ----
            let state = crate::output::snapshot_to_state_json(
                &snapshot,
                &[],
                None,
                &HashMap::new(),
                &crate::output::ConfigEcho::default(),
                &[],
                0,
            );
            let chat_object = extract_chat_object(&state).expect("a chat session emits chat fields");
            for key in [
                "number",
                "harness",
                "model",
                "context_label",
                "context_session_id",
                "started_at",
            ] {
                prop_assert!(chat_object.contains(&format!("\"{key}\":")), "{chat_object}");
            }
            for excluded in [
                must_vanish.as_str(),
                env_value.as_str(),
                command_line.as_str(),
                window_address.as_str(),
                "987654",
                history_line.as_str(),
            ] {
                prop_assert!(
                    !chat_object.contains(excluded),
                    "excluded value {:?} reached the chat fields",
                    excluded
                );
            }

            // ---- the presented lines ----
            // Question and answer both go through the existing scrubber
            // (15.5), in the same order, on both sides of the conversation.
            let asked = format!("does {secret} still work, {marker}?");
            let question = question_of(&asked);
            prop_assert!(
                !question.text().contains(must_vanish.as_str()),
                "{}",
                question.text()
            );
            prop_assert!(question.text().contains("redacted"), "{}", question.text());
            let answered = present_answer(&format!("state ok \u{1b}]2;steal\u{7} {secret}"));
            prop_assert!(!answered.contains(must_vanish.as_str()), "{answered}");
            prop_assert!(answered.contains("redacted"), "{answered}");
            prop_assert!(!answered.contains('\u{1b}'), "{answered}");

            let mut conversation = Conversation::new();
            conversation.record(Role::You, 1_700_000_200, question.text());
            conversation.record(Role::Pitwall, 1_700_000_201, &answered);
            let d = descriptor_in(&sandbox, number, harness, model, label, context.as_deref());
            let mut presented = render_header(&d, Palette::plain());
            for turn in conversation.turns() {
                presented.push_str(&render_turn(turn, Palette::plain()));
            }
            // A resume line is a presented line too, and the executor focuses
            // by address — which must not surface (15.2, 26.25).
            let live = crate::collector::collect(&plat)
                .sessions
                .first()
                .map(|s| s.id.clone())
                .expect("one observed window is one session");
            let report = resume_action(&plat, &sandbox.db(), &d, Some(&live));
            prop_assert!(report.is_ok(), "{:?}", report);
            prop_assert_eq!(plat.focused.borrow().len(), 1);
            {
                let focused = plat.focused.borrow();
                prop_assert_eq!(focused[0].as_str(), window_address.as_str());
            }
            presented.push_str(&resume_line(&report));

            for excluded in [
                must_vanish.as_str(),
                env_value.as_str(),
                command_line.as_str(),
                window_address.as_str(),
                "987654",
                history_line.as_str(),
            ] {
                prop_assert!(
                    !presented.contains(excluded),
                    "excluded value {:?} reached a presented line",
                    excluded
                );
            }
            let middle = format!("middle-{marker}-15");
            prop_assert!(
                !presented.contains(&middle),
                "scrollback line {} reached a presented line",
                middle
            );
        }
    }

    /// The `chat` object of a state artifact. The object carries only strings
    /// and numbers, so the first `}` closes it.
    fn extract_chat_object(state: &str) -> Option<&str> {
        let start = state.find("\"chat\":{")?;
        let rest = &state[start..];
        let end = rest.find('}')?;
        Some(&rest[..=end])
    }

    proptest! {
        // 100 cases (the floor): each opens a real SQLite store, drives 1..4
        // questions through the loop and then searches the database bytes.
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        // **Validates: Requirements 15.6, 19.4, 19.5, 19.6**
        // Feature: pitwall-chat-and-brief-ticker, Property 21: Conversations are never persisted — For any conversation, after the Chat_Session ends no question or answer substring exists in the SQLite database, in `state.json`, in the repository or in any log, the checkpoint count is unchanged, and no resumable entry names the Chat_Session.
        #[test]
        fn prop21_conversations_are_never_persisted(
            marker in word(),
            exchanges in 1usize..4,
            seed_checkpoints in 0usize..4,
            number in 1u16..=999u16,
        ) {
            // **The honest form chosen here.** Chat writes nothing, so the
            // property is stated as a search rather than as a diff of
            // intentions: run a real conversation against a real temp SQLite
            // store, then assert that no question or answer substring exists
            // in the database file's bytes, in any file under the data or
            // runtime directories, or in the rendered state artifact — and
            // that the checkpoint count is the number that was there before.
            // "In the repository" is covered structurally: every path this
            // run can write to is inside the sandbox, and the two it writes
            // at all (the runtime directory and the store) are searched. The
            // fake harness directory is excluded from the search for the
            // obvious reason that a stand-in agent naturally holds its own
            // answer text; a real installed agent is not a Pitwall artifact.
            let question_marker = format!("q{marker}");
            let answer_text = format!("a{marker}");
            let sandbox = Sandbox::with_harness("p21", &answer_text);

            std::fs::create_dir_all(&sandbox.data_dir).expect("data dir");
            let mut store = crate::store::Store::open(&sandbox.db()).expect("temp store");
            for i in 0..seed_checkpoints {
                store
                    .insert_checkpoint(
                        1_700_000_200 + i as i64,
                        "proj_x",
                        &format!("sess_{i:016x}"),
                        &sandbox.project_dir(),
                        Some("main"),
                        Some(true),
                        "opencode",
                        "high",
                        "sleeping",
                        1_700_000_100,
                        Some("0x1"),
                        Some("foot"),
                        None,
                        crate::store::trigger::DISAPPEARANCE,
                        None,
                    )
                    .expect("seed checkpoint");
            }
            let checkpoints_before = store.checkpoint_count().expect("count");
            drop(store);

            let plat = RecordingPlatform::quiet();
            let d = descriptor_in(&sandbox, number, "opencode", "", WORKSPACE_LABEL, None);
            let lines: Vec<String> = (0..exchanges)
                .map(|i| format!("what is {question_marker}-{i} doing?"))
                .collect();
            let run = drive(&plat, &d, &sandbox, &lines);

            // The conversation happened: both sides are held, in memory.
            prop_assert_eq!(run.turns.len(), exchanges);
            prop_assert_eq!(run.conversation.len(), exchanges * 2);
            prop_assert_eq!(sandbox.invocations(), exchanges);
            prop_assert!(run
                .conversation
                .turns()
                .iter()
                .any(|t| t.text().contains(question_marker.as_str())));
            prop_assert!(run
                .conversation
                .turns()
                .iter()
                .any(|t| t.text() == answer_text.as_str()));

            // ---- nothing of it reached the database ----
            let db_bytes = std::fs::read(sandbox.db()).expect("store file");
            prop_assert!(!db_bytes.is_empty(), "the store file is really there");
            prop_assert!(!bytes_contain(&db_bytes, question_marker.as_str()));
            prop_assert!(!bytes_contain(&db_bytes, answer_text.as_str()));

            // ---- nor any other file the run could have written ----
            let mut searched = 0usize;
            for file in files_under(&sandbox.data_dir)
                .into_iter()
                .chain(files_under(&sandbox.runtime_dir))
            {
                let bytes = std::fs::read(&file).unwrap_or_default();
                prop_assert!(
                    !bytes_contain(&bytes, question_marker.as_str()),
                    "a question reached {:?}",
                    file
                );
                prop_assert!(
                    !bytes_contain(&bytes, answer_text.as_str()),
                    "an answer reached {:?}",
                    file
                );
                searched += 1;
            }
            prop_assert!(searched >= 1, "the store file is at least searched");
            // 12.8, 15.7: no ephemeral context outlived the run either.
            prop_assert!(sandbox.staged().is_empty(), "{:?}", sandbox.staged());

            // ---- the checkpoint count is unchanged, and no resumable entry
            // names the chat (19.6) ----
            let store = crate::store::Store::open(&sandbox.db()).expect("reopen");
            prop_assert_eq!(store.checkpoint_count().expect("count"), checkpoints_before);
            let resumable = store.latest_checkpoints(100).expect("resumable");
            prop_assert_eq!(resumable.len(), seed_checkpoints);
            let number_text = d.number_text();
            for cp in &resumable {
                prop_assert_ne!(cp.session_id.as_str(), number_text.as_str());
                prop_assert!(!cp.session_id.contains(question_marker.as_str()));
                prop_assert!(cp.note.is_none());
            }
            drop(store);

            // ---- nor the state artifact ----
            let state = crate::output::snapshot_to_state_json(
                &crate::collector::collect(&plat),
                &resumable,
                None,
                &HashMap::new(),
                &crate::output::ConfigEcho::default(),
                &[],
                0,
            );
            prop_assert!(!state.contains(question_marker.as_str()), "{state}");
            prop_assert!(!state.contains(answer_text.as_str()), "{state}");

            // ---- and ending the chat discards it (19.5) ----
            let mut conversation = run.conversation;
            conversation.clear();
            prop_assert!(conversation.is_empty());
            prop_assert_eq!(conversation.len(), 0);
        }
    }
    proptest! {
        // 140 cases: 7 outcome shapes × 3 bad-directory shapes × 5 malformed
        // shapes is 105 combinations, so 140 samples every one of them and
        // most of them more than once. Each case opens a real temp SQLite
        // store for the executor's Level 2 lookup.
        #![proptest_config(ProptestConfig { cases: 140, ..ProptestConfig::default() })]

        // **Validates: Requirements 26.7, 26.8, 26.9, 26.10, 26.11, 26.12, 26.13, 26.14, 26.15, 26.16, 26.20, 26.21, 26.22, 26.25, 24.18**
        // Feature: pitwall-chat-and-brief-ticker, Property 26: Resume_Action refusals are complete and side-effect free — For any resume target, the outcome is exactly one of: refusal naming the missing target (no argument and no recorded context session), refusal on shape validation, refusal because the session is neither live nor checkpointed, refusal naming the offending directory (not absolute, missing or not a directory), focus of exactly one live window, or exactly one terminal opened at the checkpoint's directory — and every refusal performs zero executor side effects, zero agent spawns and no directory substitution; success lines name the target and the operation, failure lines carry the executor's reason.
        #[test]
        fn prop26_resume_action_refusals_are_complete_and_side_effect_free(
            case_shape in 0usize..7,
            bad_dir_shape in 0usize..3,
            malformed_shape in 0usize..5,
            target in session_id(),
            marker in word(),
        ) {
            let sandbox = Sandbox::new("p26");

            // The four directory shapes a checkpoint can carry, all inside the
            // sandbox: one that validates, one that is not absolute, one that
            // is absent, and one that exists but is a file.
            let good_dir = sandbox.root.join("project");
            std::fs::create_dir_all(&good_dir).expect("checkpoint directory");
            let decoy_dir = sandbox.root.join("decoy");
            std::fs::create_dir_all(&decoy_dir).expect("decoy directory");
            let file_dir = sandbox.root.join("a-file");
            std::fs::write(&file_dir, "not a directory").expect("file standing in for a dir");
            let missing_dir = sandbox.root.join("gone");
            let good_text = good_dir.to_string_lossy().into_owned();
            let decoy_text = decoy_dir.to_string_lossy().into_owned();
            let file_text = file_dir.to_string_lossy().into_owned();
            let missing_text = missing_dir.to_string_lossy().into_owned();
            let relative_text = format!("relative/{marker}");

            // The executor names two of the three bad directories in its
            // reason, so those reasons must be presentable verbatim for the
            // "naming the offending directory" half of the property to be
            // comparing against the reason rather than against a redaction.
            // That is an assumption about the temp directory this machine
            // hands out, so it is asserted rather than hoped for.
            for text in [&good_text, &decoy_text, &file_text, &missing_text] {
                let scrubbed = crate::context::scrub_string(text);
                prop_assert_eq!(
                    scrubbed.as_str(),
                    text.as_str(),
                    "the sandbox path must be presentable verbatim"
                );
                prop_assert!(!text.chars().any(char::is_control), "{}", text);
            }
            prop_assert!(!good_text.contains(decoy_text.as_str()));
            prop_assert!(!missing_text.contains(good_text.as_str()));
            prop_assert!(!file_text.contains(good_text.as_str()));

            // A well-shaped id that is deliberately *not* the target: the
            // first hex digit is flipped, so a collision is impossible rather
            // than improbable.
            let decoy = {
                let hex = &target[5..];
                let mut flipped = String::from(if hex.starts_with('0') { "1" } else { "0" });
                flipped.push_str(&hex[1..]);
                format!("sess_{flipped}")
            };
            prop_assert_ne!(decoy.as_str(), target.as_str());
            prop_assert!(crate::resume::is_session_id(&decoy));

            // One near miss per clause of `resume::is_session_id`: wrong
            // prefix, one digit short, one digit long, uppercase hex, and a
            // letter outside `[0-9a-f]`.
            let malformed = match malformed_shape {
                0 => format!("not-a-session-{marker}"),
                1 => "sess_0123456789abcde".to_string(),
                2 => "sess_0123456789abcdef0".to_string(),
                3 => "sess_0123456789ABCDEF".to_string(),
                _ => "sess_0123456789abcdeg".to_string(),
            };
            prop_assert!(!crate::resume::is_session_id(&malformed), "{}", malformed);

            // Shape 3 is the only live case, so it is the only one whose
            // platform observes a window at all.
            let plat = if case_shape == 3 {
                RecordingPlatform::with_windows(vec![window("0x1", "foot", "user@host:~", 4100)])
            } else {
                RecordingPlatform::quiet()
            };
            let live_id: Option<String> = crate::collector::collect(&plat)
                .sessions
                .first()
                .map(|s| s.id.clone());
            if case_shape == 3 {
                prop_assert!(live_id.is_some(), "one observed window is one session");
            }

            // How the target is supplied, and what it resolves to. Shape 1 is
            // the 26.7 path: nothing is entered, and the descriptor's own
            // context session id is used.
            let (argument, descriptor_context, resolved): (
                Option<String>,
                Option<String>,
                Option<String>,
            ) = match case_shape {
                0 => (None, None, None),
                1 => (None, Some(target.clone()), Some(target.clone())),
                2 => (Some(malformed.clone()), None, None),
                3 => (live_id.clone(), None, live_id.clone()),
                _ => (Some(target.clone()), None, Some(target.clone())),
            };

            // The checkpoint the resolved target holds, when it holds one.
            let checkpoint_dir: Option<String> = match case_shape {
                1 | 4 => Some(good_text.clone()),
                5 => Some(match bad_dir_shape {
                    0 => relative_text.clone(),
                    1 => missing_text.clone(),
                    _ => file_text.clone(),
                }),
                _ => None,
            };

            std::fs::create_dir_all(&sandbox.data_dir).expect("data dir");
            let mut store = crate::store::Store::open(&sandbox.db()).expect("temp store");
            // The decoy is resumable, valid and absolute — and it is not the
            // target. Its presence is what makes "no directory substitution" a
            // real claim rather than a vacuous one: a bridge that fell back to
            // any resumable checkpoint would open a terminal at `decoy_dir`.
            store
                .insert_checkpoint(
                    1_700_000_300,
                    "proj_decoy",
                    &decoy,
                    &decoy_text,
                    None,
                    None,
                    "opencode",
                    "high",
                    "sleeping",
                    1_700_000_250,
                    Some("0x9"),
                    Some("foot"),
                    None,
                    crate::store::trigger::MANUAL,
                    None,
                )
                .expect("decoy checkpoint");
            if let (Some(id), Some(dir)) = (resolved.as_deref(), checkpoint_dir.as_deref()) {
                store
                    .insert_checkpoint(
                        1_700_000_400,
                        "proj_target",
                        id,
                        dir,
                        None,
                        None,
                        "opencode",
                        "high",
                        "sleeping",
                        1_700_000_350,
                        Some("0x8"),
                        Some("foot"),
                        None,
                        crate::store::trigger::MANUAL,
                        None,
                    )
                    .expect("target checkpoint");
            }
            drop(store);

            let d = descriptor_in(
                &sandbox,
                4,
                "opencode",
                "",
                WORKSPACE_LABEL,
                descriptor_context.as_deref(),
            );
            prop_assert_eq!(plat.actions(), 0, "nothing has acted before the attempt");

            let report = resume_action(&plat, &sandbox.db(), &d, argument.as_deref());
            let line = resume_line(&report);

            // ---- exactly one of the six documented outcomes ----
            const CASES: &[&str] = &[
                "missing target",
                "shape validation",
                "not resumable",
                "bad directory",
                "focused one live window",
                "opened one terminal",
            ];
            // The two executor refusals are told apart by *which* reason the
            // executor gave, which is the same thing the property distinguishes
            // them by; the expected reason itself is written out below.
            let observed = match &report {
                Err(ResumeRefused::NoTarget) => "missing target",
                Err(ResumeRefused::MalformedTarget { .. }) => "shape validation",
                Err(ResumeRefused::Executor { reason, .. }) if reason.contains("no checkpoint") => {
                    "not resumable"
                }
                Err(ResumeRefused::Executor { .. }) => "bad directory",
                Ok(ResumeDone::FocusedLive { .. }) => "focused one live window",
                Ok(ResumeDone::OpenedTerminal { .. }) => "opened one terminal",
            };
            prop_assert_eq!(
                CASES.iter().filter(|c| **c == observed).count(),
                1,
                "{}",
                observed
            );
            // The expected outcome comes from the generation choice, never from
            // a second reading of the bridge.
            let want = match case_shape {
                0 => "missing target",
                1 | 4 => "opened one terminal",
                2 => "shape validation",
                3 => "focused one live window",
                5 => "bad directory",
                _ => "not resumable",
            };
            prop_assert_eq!(observed, want, "{}", line);

            // ---- one presentable line, whatever happened (26.23, 26.25) ----
            prop_assert_eq!(line.lines().count(), 1, "{}", line);
            prop_assert!(line.ends_with(RESUME_READY), "{}", line);
            prop_assert!(!line.contains('\u{1b}'), "{}", line);

            match &report {
                Err(refused) => {
                    // `reached_executor()` is the oracle for the bridge's own
                    // refusals: `false` means the resolve-and-shape steps
                    // returned before `resume::resume` was ever called, so
                    // there is no executor side effect to look for (26.9,
                    // 26.12). Shapes 0 and 2 are exactly those two.
                    let bridge_owned = matches!(case_shape, 0 | 2);
                    prop_assert_eq!(refused.reached_executor(), !bridge_owned, "{}", line);
                    let want_class = match case_shape {
                        0 => "no target",
                        2 => "malformed target",
                        _ => "resume refused",
                    };
                    prop_assert_eq!(refused.class(), want_class);

                    // ---- zero executor side effects, corroborated ----
                    // The executor may have looked (collect, one store read);
                    // it never focused and never launched, which are the only
                    // two acting capabilities the trait exposes.
                    prop_assert!(plat.focused.borrow().is_empty(), "{}", line);
                    prop_assert!(plat.launched.borrow().is_empty(), "{}", line);
                    prop_assert_eq!(plat.actions(), 0, "{}", line);

                    // ---- zero agent spawns ----
                    // No harness binary was planted in this sandbox at all, and
                    // the resume path constructs no argv of its own — the
                    // launch that *would* carry one was never made, which the
                    // empty `launched` vector above already states (26.20).
                    prop_assert_eq!(sandbox.invocations(), 0, "{}", line);

                    // ---- no directory substitution ----
                    // A resumable checkpoint for another session exists at
                    // `decoy_dir`; the refusal names it nowhere and opened
                    // nothing there (26.16).
                    prop_assert!(
                        !line.contains(decoy_text.as_str()),
                        "a refusal named a substitute directory: {}",
                        line
                    );

                    // ---- and the line says what it must ----
                    // Exhaustive over the refusal type, so no refusal shape can
                    // slip through unasserted; which `case_shape` produced each
                    // one is already pinned by the `observed == want` check
                    // above.
                    match refused {
                        // Naming the missing target: what was not entered,
                        // what the chat does not record, and how to fix it.
                        ResumeRefused::NoTarget => {
                            prop_assert!(line.contains("no session id was entered"), "{}", line);
                            prop_assert!(line.contains("records no context session"), "{}", line);
                            prop_assert!(line.contains("sess_"), "{}", line);
                        }
                        ResumeRefused::MalformedTarget { entered } => {
                            prop_assert_eq!(entered.as_str(), malformed.as_str());
                            prop_assert!(line.contains(malformed.as_str()), "{}", line);
                            prop_assert!(line.contains("nothing was looked up"), "{}", line);
                        }
                        // The two executor refusals carry the executor's own
                        // reason. The expected text is written out here rather
                        // than read back off the refusal, so "carries the
                        // executor's reason" is compared against the reason.
                        ResumeRefused::Executor { target: named, reason } => {
                            let want_target = resolved
                                .as_deref()
                                .expect("an executor refusal had a resolved target");
                            prop_assert_eq!(named.as_str(), want_target);
                            let want_reason = if case_shape == 5 {
                                match bad_dir_shape {
                                    0 => "refusing non-absolute project dir".to_string(),
                                    1 => format!("project directory unavailable: {missing_text}"),
                                    _ => format!("project path is not a directory: {file_text}"),
                                }
                            } else {
                                format!("unknown session {want_target} (no checkpoint)")
                            };
                            prop_assert_eq!(reason.as_str(), want_reason.as_str());
                            prop_assert!(line.contains(want_reason.as_str()), "{}", line);
                            prop_assert!(line.contains(want_target), "{}", line);
                            // The two directory shapes the executor can name
                            // are named (26.16); the non-absolute one is
                            // refused by shape, and its reason says so.
                            if case_shape == 5 && bad_dir_shape == 1 {
                                prop_assert!(line.contains(missing_text.as_str()), "{}", line);
                            }
                            if case_shape == 5 && bad_dir_shape == 2 {
                                prop_assert!(line.contains(file_text.as_str()), "{}", line);
                            }
                            if case_shape == 5 && bad_dir_shape == 0 {
                                prop_assert!(line.contains("non-absolute"), "{}", line);
                            }
                        }
                    }
                }
                Ok(done) => {
                    // ---- success lines name the target and the operation ----
                    let want_target = resolved.as_deref().expect("a success had a target");
                    prop_assert_eq!(done.session_id(), want_target);
                    prop_assert!(line.contains(want_target), "{}", line);
                    prop_assert!(line.contains(done.operation()), "{}", line);

                    // ---- exactly one operation, and it is the right one ----
                    prop_assert_eq!(plat.actions(), 1, "{}", line);
                    match done {
                        ResumeDone::FocusedLive { .. } => {
                            let focused = plat.focused.borrow();
                            prop_assert_eq!(focused.len(), 1);
                            prop_assert_eq!(focused[0].as_str(), "0x1");
                            // 26.13, 28.7: a live target is focused, and no
                            // terminal is opened even though a resumable
                            // checkpoint exists for another session.
                            prop_assert!(plat.launched.borrow().is_empty(), "{}", line);
                        }
                        ResumeDone::OpenedTerminal { directory, .. } => {
                            let launched = plat.launched.borrow();
                            prop_assert_eq!(launched.len(), 1);
                            // 26.14, 26.16: the checkpoint's own directory,
                            // never the decoy's and never a fallback.
                            prop_assert_eq!(launched[0].directory.as_str(), good_text.as_str());
                            prop_assert_ne!(launched[0].directory.as_str(), decoy_text.as_str());
                            // 24.18, 26.20: a fixed argument vector with no
                            // command at all, so there is no shell and no
                            // agent process.
                            prop_assert!(launched[0].command.is_empty(), "{:?}", launched[0]);
                            prop_assert_eq!(directory.as_str(), good_text.as_str());
                            prop_assert!(line.contains(good_text.as_str()), "{}", line);
                            prop_assert!(plat.focused.borrow().is_empty(), "{}", line);
                        }
                    }
                    prop_assert_eq!(sandbox.invocations(), 0, "a resume spawns no agent");
                }
            }
        }
    }

    proptest! {
        // 120 cases: 3 harnesses × (binary installed or not) × (runtime
        // directory blocked or not) × 3 malformed-model shapes is 36
        // combinations, and a third of the cases spawn the fake harness once.
        #![proptest_config(ProptestConfig { cases: 120, ..ProptestConfig::default() })]

        // **Validates: Requirements 26.19**
        // Feature: pitwall-chat-and-brief-ticker, Property 27: Agent invocation validation order is preserved — For any invalid combination of session identifier, liveness, agent binary presence, model validity and project directory, the reported error is the one produced by the first failing stage of the existing order, and nothing downstream of that stage executes.
        #[test]
        fn prop27_agent_invocation_validation_order_is_preserved(
            harness_ix in 0usize..crate::agents::KNOWN.len(),
            install_binary in any::<bool>(),
            block_runtime_dir in any::<bool>(),
            bad_model_shape in 0usize..3,
            marker in word(),
            unknown_word in word(),
        ) {
            // **The scope decision, stated rather than glossed over.**
            //
            // Requirement 26.19 is conditional: *where* a Chat_Session
            // invocation runs an agent binary, it applies "the existing
            // validation order of session identifier shape, session liveness,
            // agent binary presence, model validity, and project directory".
            // That five-stage order is `assign::prepare`'s — and `assign`
            // spawns a worker, discovers binaries on the *live* `PATH`, and is
            // reached from `pitwall assign`, not from a chat. Chat never calls
            // it. Chat reaches an agent through exactly one function,
            // [`respond_in`], and reaches a workspace through exactly one
            // other, [`resume_action`], which starts no agent at all (26.20)
            // and is Property 26's subject.
            //
            // So this test is written over the order that actually governs a
            // chat's agent invocation, which [`respond_in`] documents and
            // performs: **delivery channel → binary → staged context → argv →
            // run**. Stating the property over `assign::prepare`'s order
            // instead would be testing a function no chat can call, on the
            // live `PATH`, and would prove nothing about this surface.
            //
            // The two stages of 26.19's list that do not appear in that order
            // are not missing — they live elsewhere on the chat path, and both
            // are asserted below rather than left implicit:
            //
            // - **session identifier shape and liveness** belong to the resume
            //   bridge, which runs no agent binary; the shape check precedes
            //   any lookup there (26.11) and Property 26 asserts it.
            // - **model validity** is a *capture-time* stage: 11.7 puts it in
            //   [`ChatDescriptor::capture`], so a descriptor carrying an
            //   invalid model does not exist and `respond_in` has no model
            //   stage to fail. That is asserted at the end of this test.
            let answer = format!("answer-{marker}");
            let sandbox = Sandbox::new("p27");
            let harness = crate::agents::KNOWN[harness_ix].id;
            let binary_name = crate::agents::KNOWN[harness_ix].binary;

            // Stage 2's input: the search path either holds this harness or is
            // empty. It always exists, so "not installed" is the absence of
            // one binary rather than the absence of a directory.
            std::fs::create_dir_all(&sandbox.bin_dir).expect("bin dir");
            if install_binary {
                counting_harness(&sandbox.bin_dir, binary_name, &sandbox.log, &answer);
            }

            // Stage 3's input: a regular file occupying the runtime directory's
            // own path, so `create_dir_all` refuses. Chosen over a permission
            // change because it fails identically for an unprivileged and a
            // root test runner.
            let blocked = sandbox.root.join("blocked-run");
            let runtime_dir = if block_runtime_dir {
                std::fs::write(&blocked, "not a directory").expect("blocking file");
                blocked.clone()
            } else {
                sandbox.runtime_dir.clone()
            };

            let plat = RecordingPlatform::quiet();
            let d = descriptor_in(&sandbox, 9, harness, "", WORKSPACE_LABEL, None);
            let observation = observe(&plat, &d, &sandbox.data_dir);
            let question = question_of(&format!("what is {marker} doing?"));

            // The first failing stage, taken from the generation choice. When
            // both stage 2 and stage 3 are broken, the order says stage 2.
            let want_stage = if !install_binary {
                2
            } else if block_runtime_dir {
                3
            } else {
                0
            };

            let outcome = respond_in(
                &d,
                &observation,
                &question,
                &runtime_dir,
                &sandbox.bin_dirs(),
                Duration::from_secs(20),
            );

            match want_stage {
                2 => {
                    // The binary stage reports, *even when the staging stage
                    // would also have failed* — which is the order claim.
                    let want = ChatError::HarnessNotInstalled {
                        harness: harness.to_string(),
                    };
                    prop_assert_eq!(outcome.as_ref().err(), Some(&want));
                    prop_assert_eq!(want.class(), "harness not installed");
                    // Nothing downstream of stage 2 ran: nothing was staged
                    // and nothing was spawned.
                    prop_assert!(sandbox.staged().is_empty(), "{:?}", sandbox.staged());
                    prop_assert_eq!(sandbox.invocations(), 0);
                }
                3 => {
                    let want = ChatError::ContextUnavailable;
                    prop_assert_eq!(outcome.as_ref().err(), Some(&want));
                    prop_assert_eq!(want.class(), "context unavailable");
                    // Nothing downstream of stage 3 ran: the argv was never
                    // built and the harness — which *is* installed in this
                    // branch — was never invoked.
                    prop_assert_eq!(sandbox.invocations(), 0);
                }
                _ => {
                    // Every stage passed, so the run happened exactly once and
                    // the harness's own answer came back. Without this branch
                    // the two above would be claims about a path that never
                    // works rather than about skipped work.
                    prop_assert_eq!(
                        outcome.as_ref().ok().map(String::as_str),
                        Some(answer.as_str())
                    );
                    prop_assert_eq!(sandbox.invocations(), 1);
                    prop_assert!(sandbox.staged().is_empty(), "{:?}", sandbox.staged());
                }
            }
            prop_assert_eq!(sandbox.invocations(), usize::from(want_stage == 0));

            // The blocked path is the observable evidence for stage 3: it is
            // still the regular file it was, so no document was staged there.
            // (`Sandbox::staged` looks at `sandbox.runtime_dir`, which is a
            // different path in this branch, so it would be vacuous here.)
            if block_runtime_dir {
                prop_assert!(blocked.is_file(), "the blocked runtime path is still a file");
                let held = std::fs::read_to_string(&blocked).unwrap_or_default();
                prop_assert_eq!(held.as_str(), "not a directory");
            }

            // ---- stage 1 is total, and unreachable from any descriptor ----
            // `capture` accepts only a `crate::agents::KNOWN` id and every one
            // of those has a `DELIVERY` row, so no descriptor that exists can
            // fail the delivery stage. That is a fact worth asserting rather
            // than a gap: the guard stays total, so adding a harness without a
            // verified private channel becomes a refusal, never a leak (§9.3).
            for known in crate::agents::KNOWN {
                prop_assert!(delivery_for(known.id).is_some(), "{}", known.id);
            }
            let not_a_harness = format!("harness-{unknown_word}");
            prop_assert!(delivery_for(&not_a_harness).is_none(), "{}", not_a_harness);
            let unknown = ChatDescriptor::capture(
                9,
                &not_a_harness,
                "",
                WORKSPACE_LABEL,
                None,
                0,
                &sandbox.project_dir(),
            );
            prop_assert!(unknown.is_err(), "an unknown harness must refuse at capture");

            // ---- model validity refuses at capture, before every stage ----
            let bad_model = match bad_model_shape {
                // No provider separator.
                0 => format!("no-slash-{marker}"),
                // Whitespace, which could otherwise split an argv element.
                1 => format!("prov/{marker} with spaces"),
                // Over the 128-character bound.
                _ => format!("prov/{}", "m".repeat(200)),
            };
            prop_assert!(!crate::summary::valid_model(&bad_model), "{}", bad_model);
            let refused = ChatDescriptor::capture(
                9,
                harness,
                &bad_model,
                WORKSPACE_LABEL,
                None,
                0,
                &sandbox.project_dir(),
            );
            prop_assert!(refused.is_err(), "an invalid model must refuse");
            prop_assert!(
                refused.err().unwrap_or_default().contains("model"),
                "the refusal must name the model stage"
            );
            // And nothing downstream of that stage can run, because there is
            // no descriptor to run `respond_in` with: the invocation count is
            // still exactly what the reachable stages above left it at.
            prop_assert_eq!(sandbox.invocations(), usize::from(want_stage == 0));
        }
    }

    // -----------------------------------------------------------------
    // Task 13.4 (Rust half) — Property 29
    // -----------------------------------------------------------------

    /// Fragments a human's own turn text can be assembled from. Several are
    /// made entirely of the characters Requirement 20.11 forbids the
    /// *renderer* to add — see the note inside the test about whose
    /// characters those are.
    const TURN_TEXT_FRAGMENTS: &[&str] = &[
        "the box is drawn",
        "\u{250c}\u{2500}\u{2500}\u{2510}",
        "\u{2502} bubble \u{2502}",
        "\u{2514}\u{2500}\u{2500}\u{2518}",
        "\u{2588}\u{2593}\u{2592}\u{2591}",
        "\u{2550}\u{2551}\u{256c}",
        "+---+ | pane | +---+",
        "  leading and trailing   ",
        "caf\u{00e9} \u{4e2d}\u{6587}",
        "",
    ];

    /// Frame characters outside the box-drawing and block-element ranges: the
    /// ASCII approximations a "bordered bubble" is usually drawn with, the
    /// geometric shapes a simulated widget uses, and the emoji variation
    /// selector that dresses one up.
    const FRAME_CHARS: &[char] = &[
        '|', '+', '-', '=', '_', '#', '*', '~', '\u{2b1b}', '\u{2b1c}', '\u{25a0}', '\u{25a1}',
        '\u{25ac}', '\u{25ad}', '\u{fe0f}',
    ];

    proptest! {
        // 256 cases: 2 roles × 10 fragments in 1..6 positions × 3 joiners ×
        // a generated epoch. Pure — no sandbox, no store, no harness.
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        // **Validates: Requirements 20.10, 20.11**
        // Feature: pitwall-chat-and-brief-ticker, Property 29: Conversation turns render as plain labelled lines — For any turn role, text and timestamp, the rendered turn begins with the role label and a timestamp, contains no box-drawing or border characters, and contains no simulated widget scaffolding.
        #[test]
        fn prop29_conversation_turns_render_as_plain_labelled_lines(
            is_you in any::<bool>(),
            epoch in -86_400i64..4_000_000_000i64,
            body in proptest::collection::vec(0usize..TURN_TEXT_FRAGMENTS.len(), 1..6),
            joiner in 0usize..3,
        ) {
            // **Whose characters are whose.** 20.11 forbids the *renderer*
            // from adding bordered chat bubbles, dashboard panes and
            // simulated widgets. It does not — and must not — forbid a human
            // from typing `\u{250c}\u{2500}\u{2500}\u{2510}` into a question:
            // censoring a person's own words would be a different and worse
            // behaviour than the one being prevented. So every assertion below
            // is about the characters the renderer *contributes*, never about
            // the characters that merely *appear*.
            //
            // That distinction is made structurally rather than by guesswork.
            // `render_turn` produces exactly:
            //
            //     line 0    := "<role label>" + "  " + "<clock>"
            //     line 1+n  := "  " + <the turn's own nth text line>
            //
            // so the scaffolding is line 0 plus one two-space indent per text
            // line, and nothing else. Each rendered line after the first is
            // checked character for character against its text line with that
            // indent, and the scaffolding is then collected and checked on its
            // own. A box-drawing character in the output is therefore provably
            // the human's; a box-drawing character in the scaffolding is a
            // failure. The last assertion in the test closes the loop by
            // requiring the human's own forbidden characters to have survived,
            // so this cannot pass by censoring them.
            let sep = match joiner {
                0 => "\n",
                1 => " ",
                _ => "\t",
            };
            let text = body
                .iter()
                .map(|ix| TURN_TEXT_FRAGMENTS[*ix])
                .collect::<Vec<&str>>()
                .join(sep);
            let role = if is_you { Role::You } else { Role::Pitwall };
            let turn = Turn::new(role, epoch, &text);

            // The deterministic non-colour palette, so no SGR sequence can sit
            // between a substring assertion and what it is looking for.
            let palette = Palette::plain();
            prop_assert!(!palette.is_coloured());
            let rendered = render_turn(&turn, palette);
            prop_assert!(!rendered.contains('\u{1b}'), "{:?}", rendered);

            // ---- the role label and a timestamp lead the line (20.10) ----
            let clock = format_clock_utc(turn.at_epoch());
            let head = format!("{}  {}", role.label(), clock);
            let rendered_lines: Vec<&str> = rendered.lines().collect();
            let first = *rendered_lines
                .first()
                .expect("a rendered turn has at least one line");
            prop_assert_eq!(first, head.as_str());
            prop_assert!(rendered.starts_with(role.label()), "{:?}", rendered);
            let other = if is_you { Role::Pitwall } else { Role::You };
            prop_assert!(!first.starts_with(other.label()), "{}", first);
            // A timestamp, and a well-formed one.
            prop_assert!(first.ends_with(" UTC"), "{}", first);
            let hhmm = clock.trim_end_matches(" UTC");
            let (hh, mm) = hhmm.split_once(':').expect("the clock is HH:MM UTC");
            prop_assert_eq!(hh.len(), 2);
            prop_assert_eq!(mm.len(), 2);
            prop_assert!(hh.parse::<u32>().expect("two digits") < 24, "{}", hh);
            prop_assert!(mm.parse::<u32>().expect("two digits") < 60, "{}", mm);
            // A negative epoch is clamped, so no turn presents a pre-epoch
            // clock.
            prop_assert!(turn.at_epoch() >= 0);

            // ---- the renderer adds exactly a two-space indent per line ----
            let text_lines: Vec<&str> = turn.text().lines().collect();
            prop_assert_eq!(rendered_lines.len(), text_lines.len() + 1);
            prop_assert!(rendered.ends_with('\n'), "{:?}", rendered);
            let mut scaffolding = String::from(first);
            for (rendered_line, text_line) in rendered_lines[1..].iter().zip(text_lines.iter()) {
                let want_line = format!("  {text_line}");
                prop_assert_eq!(*rendered_line, want_line.as_str());
                let indent = &rendered_line[..2];
                prop_assert_eq!(indent, "  ");
                scaffolding.push_str(indent);
                scaffolding.push('\n');
            }

            // ---- and that scaffolding carries no border and no widget ----
            for c in scaffolding.chars() {
                prop_assert!(
                    !('\u{2500}'..='\u{257f}').contains(&c),
                    "the renderer contributed box drawing {:?}",
                    c
                );
                prop_assert!(
                    !('\u{2580}'..='\u{259f}').contains(&c),
                    "the renderer contributed a block element {:?}",
                    c
                );
                prop_assert!(
                    !FRAME_CHARS.contains(&c),
                    "the renderer contributed a frame character {:?}",
                    c
                );
            }
            // The total form of the same claim: the only characters the
            // renderer contributes at all are the label, the clock and
            // whitespace. Any border, pane edge or widget dressing whatsoever
            // — including one nobody thought to enumerate above — would have
            // to be one of these.
            prop_assert!(
                scaffolding
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == ':' || c == ' ' || c == '\n'),
                "unexpected scaffolding {:?}",
                scaffolding
            );
            // A turn is not a header: the renderer adds no wordmark and no
            // divider of its own, so the conversation area is turns and
            // nothing else.
            prop_assert!(!scaffolding.contains(WORDMARK), "{:?}", scaffolding);

            // ---- what is presented is what was asserted ----
            let mut sink: Vec<u8> = Vec::new();
            print_turn(&mut sink, &turn, palette).expect("writing to a Vec cannot fail");
            let printed = String::from_utf8_lossy(&sink).into_owned();
            prop_assert_eq!(printed.as_str(), rendered.as_str());

            // ---- the human's own words survived ----
            // Without this, every assertion above would also hold for a
            // renderer that stripped the human's text, which is not the
            // behaviour 20.11 asks for.
            for line in &text_lines {
                prop_assert!(rendered.contains(*line), "{:?} was dropped", line);
            }
            let human_drew_a_box = turn
                .text()
                .chars()
                .any(|c| ('\u{2500}'..='\u{257f}').contains(&c));
            if human_drew_a_box {
                prop_assert!(
                    rendered
                        .chars()
                        .any(|c| ('\u{2500}'..='\u{257f}').contains(&c)),
                    "the human's own box-drawing characters must not be censored"
                );
            }
        }
    }
}
