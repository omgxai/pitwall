# Changelog

All notable user-facing changes to Pitwall are documented here.

Pitwall is currently pre-release, so this changelog focuses on meaningful changes to the product rather than every internal implementation detail.

## [Unreleased]

### Added

- Pitwall Chat for having a focused conversation about the current workspace.
- A clearer AI Brief surface for understanding what is happening across active sessions.
- Improved workspace summary freshness detection.
- Better session activity and notification feedback.

### Improved

- The Pitwall panel now provides clearer visibility into active AI sessions and projects.
- Workspace summaries use bounded, structured context.
- Notifications distinguish between something that happened and what it means.
- Session actions and failures provide clearer feedback.

### In progress

- Further verification and refinement of Pitwall Chat on live Omarchy environments.
- Continued reliability and usability improvements.
- Broader support for AI coding workflows.

## [0.1.0] - 2026-09-13

### Added

- First public Pitwall release.
- Omarchy desktop integration with the native Pitwall panel.
- Workspace and AI-agent session discovery.
- Project and session grouping.
- AI-generated workspace summaries.
- Session checkpoints and resume support.
- Workspace notifications.
- Explicit session actions including focus, stop, resume and close.
- `pitwall doctor` for checking the local installation.
- User-local installation and uninstall support.
- Configurable AI agent and model settings.
- Local SQLite continuity cache.
- Scrubbed and bounded AI context.
- MIT open-source licensing.
- Pitwall pixel-art identity and UI assets.

### Improved

- Session detection and state handling.
- Panel layout, interaction and visual hierarchy.
- Reliability when sessions appear, disappear or stop.
- Feedback for refresh, summary, resume, stop, close and workspace-target failures.
- Local-first behaviour with no required cloud service or Pitwall account.

## Earlier development

Pitwall was developed through a series of internal milestones covering workspace discovery, local continuity, the Omarchy panel, checkpoints, AI summaries, notifications and workspace interaction.

Detailed implementation history remains available in the Git history and architecture documentation rather than being reproduced here.

---

[Unreleased]: https://github.com/omgxai/pitwall/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/omgxai/pitwall/releases/tag/v0.1.0
