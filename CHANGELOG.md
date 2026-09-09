# Changelog

All notable changes to Pitwall are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/); versioning follows
[SemVer](https://semver.org/) once the first release is tagged.

## [Unreleased]

### Added
- M1 workspace discovery: `platform` trait + Linux impl, `collector` with
  confidence states and stable `proj_`/`sess_` IDs, `pitwall status [--json]`
  (JSON schema v1), 23 unit tests with fixtures/mocks.
- M0 project foundation: Rust crate scaffold (`pitwall` CLI + `pitwall_lib`),
  MIT license, README, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, ROADMAP,
  CHANGELOG, `.gitignore`, GitHub templates, CI skeleton, ADRs 001–006,
  `.pitwall/STATE.md`, systemd unit + install script stubs.
