# `dev.pitwall` — Omarchy bar-widget plugin (M3)

Native Omarchy panel for Pitwall. Renders the M2 `state.json` artifact;
all detection lives in the Rust daemon/CLI, never in QML.

## Components

| File | Role |
|---|---|
| `manifest.json` | Plugin contract (`bar-widget`, id `dev.pitwall`) |
| `SessionBar.qml` | Timeline bar: label + duration track + history segments |
| (removed M5f) | `ActivityStrip`, `SessionHero`, `SessionRow`, `ResumableRow`, `StateDot` — replaced by the rail + toast |
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
  Targeting failures stay visible in the open panel as a short attention
  message; successful explicit actions use the same compact feedback line.
- Actions resolve the installed user-local binary at
  `~/.local/bin/pitwall` instead of depending on the Quickshell process PATH;
  a PATH-name fallback remains for development environments without `HOME`.
  (`hyprctl dispatch` is unusable from shell widgets in this environment —
  its Lua shorthand rejects multi-token calls; native activation is also
  the first-party mechanism.)
- Glyphs verified present in the installed font via `fc-query` charset:
  U+25CF/25CB (dots), U+F034E (focus). No other codepoints assumed.

## Identity

The panel header carries the Pitwall pixel checkered flag: original
16x16 artwork (`assets/flag.svg`, MIT, same as repo — pole + 6x5
2px checker cells in Omarchy default foreground/muted, transparent
background, `crispEdges`). `assets/flag-64.png` is the raster export
for README/GitHub/favicon use. The runtime copy at
`plugin/dev.pitwall/flag.svg` is derived from `assets/` at install
time (single source; do not edit the copy). Rendered at 14px with
`smooth: false`; if the asset fails to load, the PITWALL wordmark
alone carries the identity (no second mark, no emoji). Verified crisp
at 16px via pixel dump (pure 2px cells, no blending).

## Design notes

- No gauge: evaluated process-count and freshness meters; both either
  imply false semantics (more procs ≠ healthier) or duplicate the
  activity strip. Per project rule, a clean panel beats a meaningless
  gauge. Revisit only with a genuinely new quantity in state.json.
- ActivityStrip encodes recency (height) × state (color) from real
  `last_activity` epochs only. Never labeled CPU/productivity/health.
- Pulse: 1400ms, opacity 1.0↔0.65, running-state only, dead otherwise.
- Focus uses native `Toplevel.activate()` (app-id + title tiebreak,
  refuse-on-ambiguous) — no subprocess. Note on `hyprctl dispatch`: only
  the *bare multi-token* form is broken in this environment (its Lua
  shorthand rejects even `exec echo hi`); the Omarchy Lua form
  (`hl.dsp.focus({window=...})`, cf. `omarchy-hyprland-focus-app`)
  exists and works. `Toplevel.activate()` remains preferred: native,
  zero-subprocess, same mechanism as first-party widgets.
- Resume (M4): live rows focus natively; vanished checkpoints resume via
  fixed-form `pitwall resume --session-id <id>` (validated
  `sess_[0-9a-f]{16}`, exit-visible `Process`, no shell). Never starts
  an agent; refusals warn and leave the panel stable.

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
# keyboard toggle (add to your Omarchy bindings; plugins must not bind
# global keys themselves):
#   omarchy-shell dev.pitwall toggle
```

Needs `state.json` present: run `pitwall snapshot` (or wait for the timer).

> Hot-reload caveat (verified live): the shell reuses already-compiled
> components when the entry URL is unchanged, so QML *content* edits may
> not take effect until `omarchy-restart-shell` (lock-guarded: refuses
> while the session is locked). Manifest/metadata-only edits apply live.
> During M4 development, hot-reload served stale components across
> reloads and even a disable/enable cycle — only a shell restart loaded
> the new code. Always screenshot-verify after QML changes.
