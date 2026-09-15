# Security Policy

Pitwall is a local-first open-source project, and security and privacy are important because Pitwall observes activity in a developer's workspace.

## Supported versions

Pitwall is currently pre-release (`v0.x`).

Until the first stable release, security fixes are focused on the latest `main` branch. A supported-version policy will be published with the first stable release.

## Reporting a vulnerability

**Please do not open a public GitHub issue for a suspected security vulnerability.**

If GitHub private vulnerability reporting is enabled for this repository, use:

**GitHub → Security → Advisories → Report a vulnerability**

This keeps the report private while the issue is investigated.

If private vulnerability reporting is not available, please contact the project maintainer through the contact information on the maintainer's GitHub profile and do not disclose the vulnerability publicly.

When reporting a vulnerability, please include:

- the affected version or commit
- steps to reproduce the issue
- the expected and observed behaviour
- the potential security or privacy impact
- any useful logs or proof of concept

Please remove passwords, API keys, tokens and other sensitive information before submitting a report.

We aim to acknowledge valid reports within 72 hours and provide an initial remediation plan within 7 days.

## Security principles

Pitwall is designed around the following principles.

### Local-first

Workspace data is kept on the user's machine.

Pitwall does not require a cloud backend, mandatory account or telemetry service.

### Secrets stay private

Pitwall should never intentionally persist or expose:

- API keys
- access tokens
- passwords
- private keys
- environment variables
- command history
- terminal transcripts
- other credentials or secrets

Contributors must never commit secrets to the repository.

### Minimal AI context

AI features are optional.

When Pitwall creates an AI summary, it uses a bounded and sanitized representation of workspace state rather than sending the entire workspace to an AI provider.

Pitwall does not require a Pitwall-owned AI provider key.

### Human control

Pitwall is intended to help people understand and manage AI-assisted development, not silently take control of their computer.

Actions that affect the workspace should be explicit and constrained.

### Safe execution

Operations that interact with the user's desktop or workspace should use validated inputs and fixed execution paths rather than arbitrary shell commands.

Unexpected or invalid targets should fail safely rather than silently falling back to another target.

## Contributor security

Please:

- keep credentials out of source code and configuration files
- use environment variables or the operating system's credential store for local development
- never include secrets in issues, pull requests or documentation
- check logs and screenshots for sensitive information before sharing them
- report security-sensitive bugs privately

CI includes automated checks intended to catch accidentally committed credentials, but contributors are responsible for checking their own changes as well.

## Privacy boundary

Pitwall may temporarily inspect information from running processes and desktop windows in order to understand the current workspace.

This information is intentionally bounded.

Persistent workspace state should contain only the information needed to identify and describe sessions and projects. Full command lines, credentials, environment variables and terminal transcripts should not become part of persistent history.

Temporary data used for AI interpretation should be bounded, scrubbed and removed when it is no longer needed.

## Security changes

Security-related changes should include appropriate tests and documentation.

If a change affects what Pitwall observes, stores, sends to an AI agent, or can do to the user's workspace, the security and privacy implications should be considered as part of the change.

For more information about contributing, see [CONTRIBUTING.md](CONTRIBUTING.md).
