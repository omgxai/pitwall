# Changelog

All notable changes to Pitwall are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/); versioning follows
[SemVer](https://semver.org/) once the first release is tagged.

## [Unreleased]

### Added
- M3 Omarchy panel: `plugin/dev.pitwall/` (manifest, Widget, StateReader,
  SessionHero, SessionRow, ActivityStrip, StateDot, README). Kit-only QML,
  FileView-driven state.json, native `Toplevel.activate()` Focus,
  animation budget enforced, screenshot-verified on live desktop.
- M2 local continuity cache: `store` module on `rusqlite` (bundled, sole
  dependency), `meta`/`observations`/`sessions` schema v1, hash-gated
  writes, newest-100 pruning, corrupt-quarantine + newer-version refusal,
  `state.json` artifact (state schema v1, scrubbed), `pitwall snapshot`,
  P1 project-dir normalization, observer self-exclusion, systemd unit +
  timer (shipped disabled), ADR-007, SECURITY persistence boundary.
- M1 workspace discovery: `platform` trait + Linux impl, `collector` with
  confidence states and stable `proj_`/`sess_` IDs, `pitwall status [--json]`
  (JSON schema v1), 23 unit tests with fixtures/mocks.
- M0 project foundation: Rust crate scaffold (`pitwall` CLI + `pitwall_lib`),
  MIT license, README, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, ROADMAP,
  CHANGELOG, `.gitignore`, GitHub templates, CI skeleton, ADRs 001–006,
  `.pitwall/STATE.md`, systemd unit + install script stubs.
