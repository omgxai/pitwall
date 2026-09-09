# ADR-005: OpenRouter integration

- **Status:** Accepted (M0; implementation deferred to M5).
- **Context:** MVP needs concise workspace summaries; the user brings their
  own AI provider.
- **Decision:** OpenRouter as the single optional LLM provider. BYO API key
  from env var or OS keyring only. Deterministic local summaries are the
  default; LLM calls are opt-in, debounced, cached, and send minimal fields
  only (project, branch, process state, activity line — never file contents
  or secrets).
- **Consequences:** Pitwall is fully useful with no key. Key handling code
  must pass secret-scan CI and never log or persist raw keys.
