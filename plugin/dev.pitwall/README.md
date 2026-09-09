# `dev.pitwall` — Omarchy bar-widget plugin (M3)

Native Omarchy panel for Pitwall. Renders the M2 `state.json` artifact;
all detection lives in the Rust daemon/CLI, never in QML.

## Components

| File | Role |
|---|---|
| `manifest.json` | Plugin contract (`bar-widget`, id `dev.pitwall`) |
| `Widget.qml` | Entry point: bar indicator + popup, Focus action |
| `StateReader.qml` | Watched `FileView` → validated `record` (null on missing/malformed) |
| `SessionHero.qml` | Primary session hero (`PanelHero` + Focus button) |
| `SessionRow.qml` | Compact 28px session row + Focus button |
| `ActivityStrip.qml` | Recency segments (static rectangles, no Canvas) |
| `StateDot.qml` | State dot (●/○ shape + color + label elsewhere; pulse only while running) |

## Design rules (binding)

- Kit only: `qs.Commons` (`Style`, `Color`, `Util`), `qs.Ui`
  (`Panel`, `WidgetButton`, `KeyboardPanel`, `PanelHero`,
  `PanelSectionHeader`, `PanelSeparator`, `PanelActionButton`).
- Zero hardcoded theme colors; `Style.font.family` everywhere
  (JetBrainsMono Nerd Font, system-supplied).
- Animation budget: ≤2 concurrent, ≤200ms one-shots, `OutCubic`,
  nothing while closed, pulse only while a session runs.
- Focus uses `Toplevel.activate()` matched by app-id (+ exact title to
  break ties); refuses ambiguous/gone targets. Never constructs commands.
  (`hyprctl dispatch` is unusable from shell widgets in this environment —
  its Lua shorthand rejects multi-token calls; native activation is also
  the first-party mechanism.)
- Glyphs verified present in the installed font via `fc-query` charset:
  U+25CF/25CB (dots), U+F034E (focus). No other codepoints assumed.

## Design notes

- No gauge: evaluated process-count and freshness meters; both either
  imply false semantics (more procs ≠ healthier) or duplicate the
  activity strip. Per project rule, a clean panel beats a meaningless
  gauge. Revisit only with a genuinely new quantity in state.json.
- ActivityStrip encodes recency (height) × state (color) from real
  `last_activity` epochs only. Never labeled CPU/productivity/health.
- Pulse: 1400ms, opacity 1.0↔0.65, running-state only, dead otherwise.

## Dev workflow

```bash
# install/edit loop (no root, no /usr/share changes)
rm -rf ~/.config/omarchy/plugins/dev.pitwall
cp -r plugin/dev.pitwall ~/.config/omarchy/plugins/dev.pitwall
omarchy plugin enable dev.pitwall --section right   # first time only
# shell hot-reloads on plugin change; watch for errors:
journalctl --user -t omarchy-shell --since "1 minute ago" | grep -i pitwall
# drive the panel without clicking:
omarchy-shell dev.pitwall toggle
```

Needs `state.json` present: run `pitwall snapshot` (or wait for the timer).
