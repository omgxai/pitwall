# ADR-003: Omarchy integration mechanism

- **Status:** Accepted (M0, verified on Omarchy 4.0.0.alpha).
- **Context:** Evaluated waybar modules, tray apps, Electron, and separate
  windows. Omarchy 4 runs a single Quickshell shell hosting the bar; panels
  and widgets load as plugins.
- **Decision:** Ship a third-party **bar-widget plugin** (`dev.pitwall`:
  `manifest.json` with `kinds: ["bar-widget"]`) installed under
  `~/.config/omarchy/plugins/`, enabled/positioned via
  `omarchy plugin enable` / `omarchy bar put`. The QML renders only; all
  logic lives in the Rust daemon.
- **Consequences:** Feels native, survives shell restarts, near-zero shell
  overhead. Widget must tolerate a missing/stale snapshot file gracefully.
  Reference patterns: first-party `omarchy.agents`, `omarchy.active-window`.
