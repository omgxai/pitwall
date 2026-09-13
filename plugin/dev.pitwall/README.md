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
- AI brief ticker: renders the **complete** brief with `elide:
  Text.ElideNone` — no truncation marker, no filler, nothing clipped.
  Motion is right-to-left only; a pass starts with the first character at
  the viewport's right edge (`x = viewport.width`) and animates to
  `-marqueeText.implicitWidth`, so the last character clears the left edge
  before `onFinished` starts a fresh pass from the right. Duration is
  `(x + contentWidth) / 180 px per second` from measured geometry
  (`tickerDurationMs`), never from character count, and there is no lower
  clamp — a floor would make short briefs crawl. Linear easing. The pass
  offset lives on `root`, not on the track, so a panel content teardown
  while paused does not lose progress; pause is the animation's own
  `paused` state (closed panel, expanded brief, or hover over the whole
  brief surface), so resuming continues from the held offset instead of
  restarting. A changed summary abandons the pass in flight and restarts
  from the right; an empty brief hides the viewport and stops the
  animation. The pass-progress underline is a width *binding* on the
  ticker's own `x`, not a second animation, so the ≤2 concurrent budget
  above is untouched.
- Open Chat control: `Open Chat` runs the CLI with fixed argv only —
  `[pitwall, "chat"]`, or `[pitwall, "chat", "--session", <sess id>]` when
  a **live** session is pinned. A pinned resumable is a vanished session
  and cannot be a chat context; an invalid session id refuses instead of
  silently falling back to workspace context. The hint line states which
  of the two will happen before the click. Rendering, hovering or
  activating it invokes no inference and no model call.
- Glyphs: no codepoint enters the plugin until it is verified present in the
  installed font (`fc-query` charset) or already proven by the shipped,
  screenshot-verified panel. Current coverage — Nerd PUA: U+F013 gear,
  U+F024 flag (`pitwall-native` tier, also the Chat Sessions region),
  U+F007 user (`agents`), U+F07B folder (`workspace`), U+F0AD wrench
  (`system`), U+F0450 refresh, U+F034E focus action, U+F0140/U+F0142
  chevrons. BMP: U+25CF/25CB dots, U+2713, U+2022, U+2715, U+25A0, U+25B6,
  U+25B4/25BE, U+00B7, U+2014, U+2026, U+2192. Nothing outside this set is
  assumed, and no colour emoji is used.

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
- Chat entries (M8): chat sessions arrive in `state.json` as ordinary
  sessions carrying an additive `chat` object. They are held out of the
  tier walk and appended as one `Chat Sessions` group *after* every
  agent/workspace group — their tier is `pitwall-native`, which would
  otherwise place them first, the opposite of the intended prominence — and
  the whole region renders one weight step down (`caption: true`). Same
  single-row bar form as every other entry, carrying the literal `Pitwall
  Chat`, the number, harness, model (or the resolved agent-default label)
  and context label, closed by the observed state word. Focus is the only
  action offered, and activity comes from `state.json` alone. No entries,
  no header: the region is absent when no session carries chat fields. A
  malformed `chat` value degrades that entry to an ordinary session rather
  than breaking the panel. The Rust core decides what is a chat (title
  grammar plus a corroborating process-tree lease — see
  `docs/adr/ADR-009-chat-identity.md`); QML never infers it.
- M8 verification status: the ticker correction, the chat region and the
  Open Chat control were written without QML tooling — no `qmllint`, no
  `qmltestrunner`, and no live shell — so **nothing in M8 has been
  screenshot-verified or run**. The hot-reload caveat below applies in
  full: restart the shell and screenshot-verify before trusting any of
  it. Behaviour described above is the code's intent, not an observation.

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
