# Contributing to Pitwall

Thanks for your interest. Pitwall is MIT-licensed and contributor-friendly.

## Ground rules

1. **Small, verifiable milestones.** One phase at a time; every PR must state
   what was tested.
2. **Local-first, low weight.** No cloud deps, no telemetry, no heavy runtimes.
   Justify any new dependency in the PR description.
3. **Never commit secrets.** API keys, tokens, and keyrings are rejected by CI
   scanning. Use env vars / OS keyring locally.
4. **Omarchy-native UI.** Calm, fast, unobtrusive. No dashboard clutter.

## Workflow

1. Fork, branch from `main` (`feat/<short-name>`).
2. Keep commits small and conventional:
   `feat: …`, `fix: …`, `docs: …`, `chore: …`.
3. Before pushing:
   ```bash
   cargo fmt --check && cargo test && cargo clippy -- -D warnings
   ```
4. Update `.pitwall/STATE.md` if your change completes or moves a milestone.
5. Open a PR using the template; link any related issue and ADR.

## Reporting issues

Use the bug report / feature request templates. Include: Omarchy version,
`hyprctl version` output, `pitwall --version`, and steps to reproduce.
Never paste secrets or API keys into issues.

## Code of Conduct

By participating you agree to uphold [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
