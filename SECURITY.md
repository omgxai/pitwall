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
