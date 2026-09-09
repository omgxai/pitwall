# Security Policy

## Supported versions

Pitwall is pre-release (v0.x). Security fixes target the latest `main`
until the first tagged release; a version table will be published at v0.1.0.

## Report a vulnerability

- **Do not open a public issue for a suspected vulnerability.**
- Report privately to the maintainer contact published with the public
  repository (to be added before first public push).
- Include: affected version/commit, reproduction steps, impact assessment.
- Expect acknowledgement within 72 hours and a remediation plan within 7 days.

## Security principles (architectural, non-negotiable)

1. **Local-first.** Workspace data stays on-device in SQLite. No cloud
   backend, no telemetry, no mandatory accounts.
2. **Secrets never travel.** API keys and tokens are never committed, logged,
   embedded in prompts, or exposed through summaries or UI output.
3. **Minimal LLM context.** The optional OpenRouter summarizer receives only
   the minimum fields needed (project name, branch, process state, short
   activity line) — never file contents or credentials.
4. **Human approval is the security boundary.** Agents may request; only the
   human approves (future approval layer). No autonomous privileged actions.
5. **Safe action layer.** MVP actions are limited to focusing/opening
   terminals and projects.

## Secret hygiene for contributors

- Store local keys in env vars or the OS keyring, never in files or shell
  history shared with the repo.
- CI includes a secret-scan step (gitleaks-style pattern check) that fails
  the build on suspected committed credentials.

## Persistence boundary (M2, binding)

Pitwall observes full process command lines in memory to classify agents
(`status --json` may show them ephemerally for local debugging). The
following MUST NOT enter SQLite history or `state.json`:

- full argv / command-line strings (only the process **name** is persisted)
- agent evidence strings containing command text (evidence stays in the
  live CLI output only)
- environment variables (never read, never stored)
- API keys, tokens, passwords, secrets of any kind
- command history or terminal output

Persisted per session (only): project id/dir/name, git repo state
(is_repo/branch/clean), session id, agent kind + confidence, aggregate
state, process count, activity epoch, window address/class/title/workspace,
collection time, hostname. Per process: name, state, start time, cwd.

Enforcement: unit tests assert secret-bearing fixtures never reach database
bytes (`full_cmdline_never_reaches_sqlite`) or the state artifact
(`state_artifact_is_separate_versioned_and_scrubbed`). CI secret-scan
covers the repository itself.

## M4 action boundary

Resume executes only two fixed-form operations, both user-initiated:

- Level 1 — focus a live window by validated `0x…` address (native
  compositor activation; CLI equivalent uses the first-party Lua dispatch
  shape with argv passing, never a shell).
- Level 2 — open one terminal at a validated absolute existing directory
  (`xdg-terminal-exec --dir`, fixed argv, no shell, no interpolation).

Refusals (malformed id, unknown session, missing/non-dir path, launch
failure) exit non-zero with a reason and never fall back to another
target. There is no agent-start path and no arbitrary-command path;
checkpoint notes are length-capped labels, never interpreted.

## M5c ephemeral terminal context

Terminal first/last lines sampled for AI summaries are EPHEMERAL, never
history: they exist only inside one bounded JSON document under
`/run/user/$UID/pitwall/` (0700 dir, O_EXCL 0600 unpredictable file),
are scrubbed before writing (secret-shape matrix incl. PEM/Bearer/bare
forms; structural bans on env/argv/transcripts hold first), travel to
the agent only via `-f` file attachment (never argv, never shell), and
are unlinked on every exit path (explicit close + Drop guard; tmpfs
backing). SQLite/state.json/logs never receive terminal text (only the
returned interpretation may later be cached per M5 design, never the
evidence). The agent is invoked with fixed argv and a static
interpret-only instruction; tool-call payloads in its output are
ignored, never chained.
