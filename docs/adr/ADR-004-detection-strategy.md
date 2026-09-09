# ADR-004: Workspace detection strategy

- **Status:** Accepted (M0, mechanism verified; implementation in M1).
- **Context:** Need terminals, shells, agent processes, working directories,
  and git context with no polling storms.
- **Decision:** Join three native sources: (1) `hyprctl clients -j` for
  terminal windows/PIDs (incl. the `org.omarchy.agent` app-id convention),
  (2) `/proc` (`cwd`, `cmdline`, `stat`) for process truth, (3) `git`
  (branch/status) per detected working directory. Drive refresh from the
  Hyprland event socket with a slow (10–30s) debounced fallback poll.
- **Consequences:** No daemons to inject, no shell hooks required for MVP.
  Agent identity is heuristic (cmdline/app-id) and must be labeled as
  detected, never authoritative.
