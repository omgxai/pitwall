# ADR-009: Chat session identity and native terminal launch (M8)

- **Status:** Accepted (M8). Implemented in `src/chat.rs`, `src/collector.rs`,
  `src/platform/linux.rs`, `src/main.rs`. **Not yet verified on an Omarchy
  runtime** — see "Verification status" below.
- **Context:** `pitwall chat` runs in a real terminal window, and Pitwall has
  to recognise that window as one of its own so the chat appears in its own
  panel. The obvious mechanism — a distinguishing window class / app-id — is
  unavailable to us: Omarchy lets the user choose the emulator (Foot default,
  Alacritty / Ghostty / Kitty selectable), so the class of a chat window is
  whatever the user's emulator sets. Passing `--app-id` or `--class` would
  name and require a specific emulator, which the Omarchy-native launch
  discipline forbids (ADR-003). `--title` is likewise not portably honoured
  across those emulators. So the window's *own* identity cannot carry ours.
- **Decision:**
  - **One launch path, emulator-agnostic.** Every native terminal — resume's
    and chat's — goes through the single `xdg-terminal-exec` invocation in
    `platform::linux::launch_terminal`, fixed argv, no shell, stdio null,
    `spawn()`. Command pass-through uses the abstraction's `--` end-of-options
    marker. No emulator is named, none is required to be installed, and
    nothing in `src/` reads a terminal's identity from the environment (there
    is no `TERM`, `TERM_PROGRAM`, `$TERMINAL` or emulator-specific env read
    anywhere).
  - **`pitwall chat` relaunches itself.** When stdout is not a TTY (the panel
    runs it from a fixed-argv `Process`) it opens one terminal running the same
    subcommand and exits 0. When stdout is a TTY it runs the chat in the
    foreground of that terminal. No daemon, no background process.
  - **Signal A — the chat titles itself.** The running chat emits OSC 2 with a
    reserved grammar and re-asserts it after every harness run:

    ```text
    Pitwall Chat NNN · <harness> · <model|agent default> · <context label>
    ```

    Parsed strictly (`chat::parse_title`): at most 200 characters, no control
    character anywhere, exactly four fields split on ` · ` (so no field may
    contain the separator), `Pitwall Chat ` followed by exactly three ASCII
    digits in `001..=999`, and a harness that is a known agent id.
  - **Signal B — a chat-number lease.** Number allocation creates
    `chat-NNN.lease` with `O_EXCL`, mode `0600`, in the runtime directory,
    containing the owner pid. Released explicitly at `/exit` and by `Drop` on
    every other path out of the command.
  - **Discovery requires both signals to corroborate each other.**
    `collector::chat_facts_for` upgrades a window to a chat only when the title
    parses, *and* the number in that title names a live lease, *and* the
    lease's owner pid is in that window's process tree, *and* that pid is a
    `pitwall chat` process. Any one of those failing means "not a chat": the
    window keeps the role, agent evidence and confidence it already had, and no
    partial chat entry is ever emitted.
- **Consequences:**
  - **A title alone is not an identity.** Printing the grammar into any
    terminal does not produce a chat entry, because the corroborating lease and
    process-tree signals cannot be forged by writing text.
  - **Server-mode terminals stay separate.** Foot/Kitty in server mode can put
    one pid in several windows' trees; the number↔lease↔pid binding means only
    the window whose title claims that number is upgraded.
  - **A chat started by hand inside a tmux pane is not discovered as a chat.**
    Its process hangs off the tmux server rather than the window client, and
    tmux owns the title, so neither signal holds. It appears as the ordinary
    terminal session it is. Chat launched by Pitwall is a direct child of the
    terminal, so this affects manual tmux use only. Inventing a chat that
    cannot be observed is not an option we take.
  - **The context session id is not observable.** The title grammar carries the
    context *label*, not the session id, and the collector has no other source
    for it, so `ChatFacts::context_session_id` is always `None`. Widening the
    grammar would be a grammar change, not a collector change.
  - **No new state is persisted.** Chat facts reaching `state.json` are the
    number, harness, model, context label and start epoch. No conversation
    text, pid, window address, argv or environment value — those are not fields
    on the type that carries them.
  - **Residue is bounded, not silently handled.** Rust's standard library has
    no signal handling and M8 adds no dependency for it, so a `SIGINT` during a
    harness run can leave one pid-tagged `0600` ephemeral context file in the
    runtime directory (tmpfs). The next chat startup sweeps orphans whose owner
    pid is gone; logout clears the rest. `/exit` and closing the window are
    clean paths.
  - **A known defect in the launch argv is deliberately preserved.**
    `launch_terminal` passes `--dir` and the directory as two argv elements.
    The reference `xdg-terminal-exec` accepts only the joined `--dir=<dir>`
    form and discards a bare `--dir`, after which the directory is taken as the
    command to run — established by executing that project's option parser in
    isolation, not by observing our own launch. Correcting it would change
    `resume`'s argv, which M8 is required to keep byte-identical, so the shape
    is kept and the discrepancy recorded instead. **Unverified on the target
    machine**: a downstream patch or a `$TERMINAL` that points straight at an
    emulator could mask it either way. Resolving it is a decision, not a
    silent code change.
- **Verification status:** the mechanism above is implemented and reasoned
  from the source, and every part of it that is pure — title composition and
  parsing, lease allocation, the corroboration rule, argv construction — has
  unit and property tests. **None of it has been observed on an Omarchy
  runtime.** M8 was written on macOS with no Rust toolchain and no QML tooling,
  so nothing was compiled, no test was executed, and no chat window was ever
  opened. Specifically still to be confirmed on the target machine: that
  `xdg-terminal-exec` is present and forwards a command after `--`; the `--dir`
  argv finding above; that OSC 2 reaches the compositor title in the user's
  emulator; that a chat window is discovered and appears in the panel; and
  that codex consumes the bounded document on stdin alongside its prompt
  argument (documented upstream, not observed live — if it does not, the fix is
  to drop codex from the chat delivery table, never to move the document into
  argv).
