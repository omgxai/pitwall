import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs.Commons
import qs.Ui

// Pitwall session timeline instrument. Collapsed: flag only. Expanded:
// header, AI summary ticker, session rail (duration bars, newest first),
// attached toast for the hovered/pinned session, settings behind gear.
// Data: state.json v3 via StateReader only. Actions: native activate/
// close, fixed-argv panel processes (resume/stop/config/models), explicit
// clicks only. No polling, no per-bar inference, no transcript storage.
Panel {
  id: root
  moduleName: "dev.pitwall"
  ipcTarget: "dev.pitwall"

  visible: stateReader.sessionCount > 0 || stateReader.resumable.length > 0
  implicitWidth: buttonHolder.implicitWidth
  implicitHeight: buttonHolder.implicitHeight

  property double nowMs: Date.now()
  property string selectedId: ""
  property string hoveredId: ""
  property bool showSettings: false
  property bool summaryExpanded: false
  property bool generating: false
  property string feedbackText: ""
  property bool feedbackAttention: false
  // Scroll_Rate: the one speed constant. Every pass duration derives from it
  // and measured geometry — never from the summary's character count (3.6).
  readonly property real tickerSpeedPxPerSec: 180
  // The pass offset lives on root, not on the track, because the panel can
  // tear its content down while a pass is paused. `paused` preserves progress
  // only while the item lives; the externalised offset survives teardown, so a
  // reopened panel continues the same pass from where it stopped (4.2, 4.5).
  // NaN means "no pass in flight yet": the first pass enters from the right.
  property real tickerOffset: NaN
  // Travel distance over Scroll_Rate, in milliseconds. No lower clamp: a floor
  // would make short briefs scroll slower than long ones (3.1, 3.3, 3.6).
  // Math.max(1, …) is not a speed floor, it only keeps the duration off zero,
  // where a NumberAnimation snaps to `to` instead of scrolling.
  function tickerDurationMs(fromX, contentW) {
    return Math.max(1, (fromX + contentW) / root.tickerSpeedPxPerSec * 1000)
  }
  // The installer places the binary in the user-local bin directory. Keep a
  // PATH fallback for development environments that provide another install.
  readonly property string pitwallBinary: {
    var home = String(Quickshell.env("HOME") || "")
    return home !== "" ? home + "/.local/bin/pitwall" : "pitwall"
  }
  // Local settings mirror (initialized from state echo, updated on change).
  property string cfgAgent: "opencode"
  property string cfgModel: ""
  property bool cfgSummary: true
  property var modelOptions: ["Agent default"]

  readonly property var liveSessions: {
    var arr = stateReader.sessions.slice()
    arr.sort(function(a, b) {
      var ea = (a.last_activity && Number(a.last_activity.epoch)) || 0
      var eb = (b.last_activity && Number(b.last_activity.epoch)) || 0
      return eb - ea
    })
    return arr
  }
  readonly property var resumable: stateReader.resumable
  readonly property bool stale: stateReader.isStale(nowMs)
  readonly property var summary: stateReader.summary
  readonly property var primary: stateReader.primarySession
  // Longest observed age across the rail (seconds, floor 60): log-scale anchor.
  readonly property double maxAge: {
    var m = 60
    var arr = stateReader.sessions
    for (var i = 0; i < arr.length; i++) {
      var a = Number((arr[i] && arr[i].age_secs) || 0)
      if (a > m) m = a
    }
    return m
  }

  readonly property bool needsAttention: {
    if (stale) return true
    var ss = stateReader.sessions
    for (var i = 0; i < ss.length; i++) {
      if (String((ss[i] || {}).state || "") === "stopped") return true
    }
    return false
  }

  // Duration fraction: logarithmic compression so one ancient session
  // never flattens the rest. Bounded [0.12, 1]. Unknown age → minimum.
  function barFrac(ageSecs) {
    var age = Math.max(0, Number(ageSecs) || 0)
    var f = Math.log(1 + age) / Math.log(1 + maxAge)
    return Math.min(1, Math.max(0.12, f))
  }

  function ageTextShort(secs) {
    var s = Math.max(0, Math.round(Number(secs) || 0))
    if (s < 90) return s + "s"
    var m = Math.floor(s / 60)
    if (m < 90) return m + "m"
    var h = Math.floor(m / 60)
    if (h < 48) return h + "h"
    return Math.floor(h / 24) + "d"
  }

  function tooltipText() {
    if (primary) {
      var s = String(primary.summary || stateReader.sessionLabel(primary))
      return stale ? s + " (stale data)" : s
    }
    return "Pitwall"
  }

  function announce(text, attention) {
    root.feedbackText = String(text || "")
    root.feedbackAttention = !!attention
    feedbackTimer.restart()
  }

  // --- native focus/close (M4 mechanism, unchanged semantics) ---
  function findToplevel(s) {
    var w = (s && s.window) || null
    if (!w) return null
    var addr = String(w.address || "")
    if (!/^0x[0-9a-fA-F]+$/.test(addr)) return null
    var wanted = []
    if (w.class) wanted.push(String(w.class))
    if (w.initial_class) wanted.push(String(w.initial_class))
    var model = ToplevelManager.toplevels
    var list = (model && model.values) || []
    var candidates = []
    for (var i = 0; i < list.length; i++) {
      var t = list[i]
      if (!t) continue
      if (wanted.indexOf(String(t.appId || "")) >= 0) candidates.push(t)
    }
    if (candidates.length === 1) return candidates[0]
    if (candidates.length > 1) {
      var title = String(w.title || "")
      for (var j = 0; j < candidates.length; j++) {
        if (title !== "" && String(candidates[j].title || "") === title) return candidates[j]
      }
    }
    return null
  }

  function focusSession(s) {
    var target = findToplevel(s)
    if (!target) {
      console.warn("pitwall", "Focus target gone or ambiguous")
      root.announce("Workspace target is no longer available.", true)
      return
    }
    target.activate()
    root.announce("Focused " + stateReader.sessionLabel(s) + ".", false)
    root.close()
  }

  function closeSession(s) {
    var target = findToplevel(s)
    if (!target) {
      console.warn("pitwall", "Close target gone or ambiguous")
      root.announce("Could not identify the original window.", true)
      return
    }
    target.close()
    root.announce("Closed " + stateReader.sessionLabel(s) + ".", false)
    root.close()
  }

  // --- fixed-argv panel processes (no shell, validated inputs) ---
  function runFixed(argv, onDone) {
    var proc = actionProcComponent.createObject(root, { cmd: argv, done: onDone })
    if (!proc) {
      console.warn("pitwall", "action spawn failed")
      return
    }
    proc.running = true
  }

  function resumeCheckpoint(sessionId) {
    var sid = String(sessionId || "")
    if (!/^sess_[0-9a-f]{16}$/.test(sid)) {
      console.warn("pitwall", "Refusing resume with invalid session id", sid)
      root.announce("Could not resume: invalid session.", true)
      return
    }
    runFixed([root.pitwallBinary, "resume", "--session-id", sid], function(code) {
      if (code !== 0) {
        console.warn("pitwall", "resume exited", code, "for", sid)
        root.announce("Resume failed. The workspace target may be gone.", true)
      } else {
        root.announce("Resume started.", false)
      }
    })
  }

  function stopSession(s) {
    // SIGTERM only, numeric pid only, explicit click only. Never SIGKILL,
    // never a shell. The session root owns the tree; the terminal owns
    // teardown policy from there.
    var pid = Number(s && s.root_pid)
    if (!isFinite(pid) || pid <= 1 || Math.floor(pid) !== pid) {
      console.warn("pitwall", "Refusing stop with invalid pid")
      root.announce("Could not stop: workspace target is unavailable.", true)
      return
    }
    runFixed(["/usr/bin/kill", "-s", "TERM", String(pid)], function(code) {
      if (code !== 0) {
        console.warn("pitwall", "stop exited", code, "for pid", pid)
        root.announce("Stop failed. The session may have already ended.", true)
      } else {
        root.announce("Stop requested.", false)
      }
      stateReader.refresh()
    })
  }

  function elapsedFor(s) {
    if (!s || !s.last_activity) return ""
    return stateReader.ageText(s.last_activity.epoch, nowMs)
  }

  function elapsedForCheckpoint(c) {
    if (!c) return ""
    return stateReader.ageText(c.created_at, nowMs)
  }

  function confReason(kind, conf, role) {
    if (kind !== "unknown") return kind + " · " + conf + " confidence"
    if (role === "app") return "Not a terminal — agent detection does not apply"
    if (conf === "low") return "Agent identity unavailable — terminal context only"
    return "Agent identity unavailable"
  }

  onOpenedChanged: if (opened) {
    nowMs = Date.now()
    selectedId = ""
    hoveredId = ""
    showSettings = false
    summaryExpanded = false
    stateReader.refresh()
    root.refreshNow()
    Qt.callLater(function() { keyCatcher.forceActiveFocus() })
  }

  StateReader {
    id: stateReader
    statePath: String(root.setting("statePath", ""))
    staleAfterSec: Number(root.setting("staleAfterSec", 900)) || 900
    onRecordChanged: {
      // Seed local settings mirror from the state echo (user edits below
      // update the mirror immediately; echo is the initial source).
      if (record && record.config) {
        if (record.config.agent) root.cfgAgent = String(record.config.agent)
        root.cfgModel = record.config.model ? String(record.config.model) : ""
        root.cfgSummary = record.config.summary_enabled !== false
      }
    }
  }

  Timer {
    interval: 30000
    running: root.opened
    repeat: true
    onTriggered: root.nowMs = Date.now()
  }

  Timer {
    id: feedbackTimer
    interval: 4500
    repeat: false
    onTriggered: root.feedbackText = ""
  }

  // One-shot action runner: fixed argv, exit-visible, no shell.
  Component {
    id: actionProcComponent
    Process {
      property var cmd: []
      property var done: null
      command: cmd
      stdout: StdioCollector {}
      stderr: StdioCollector {}
      onExited: function(code) {
        if (done) done(code)
        destroy()
      }
    }
  }

  // Settings data loaders (user-gesture triggered only, never polled).
  Process {
    id: modelsProc
    property var onDone: null
    stdout: StdioCollector { id: modelsOut }
    stderr: StdioCollector {}
    onExited: function(code) {
      var lines = String(code === 0 ? modelsOut.text : "").split("\n")
      var opts = ["Agent default"]
      for (var i = 0; i < lines.length; i++) {
        var t = lines[i].trim()
        if (t !== "") opts.push(t)
      }
      root.modelOptions = opts
      if (onDone) { onDone(); onDone = null }
    }
  }

  function refreshModels(agent, then) {
    modelsProc.onDone = then || null
    modelsProc.command = [root.pitwallBinary, "models", "--agent", String(agent || "opencode")]
    modelsProc.running = true
  }

  property bool refreshing: false

  // Workspace refresh (explicit open/manual only — never polled, never
  // AI). Reuses the existing snapshot path; the watched state file
  // updates the rail when the write lands.
  function refreshNow() {
    if (root.refreshing) return
    root.refreshing = true
    refreshProc.running = true
  }

  Process {
    id: refreshProc
    command: [root.pitwallBinary, "snapshot"]
    stdout: StdioCollector {}
    stderr: StdioCollector {}
    onExited: function(code) {
      root.refreshing = false
      refreshButton.rotation = 0
      if (code !== 0) {
        console.warn("pitwall", "snapshot refresh exited", code)
        root.announce("Refresh failed. Showing the last known workspace state.", true)
      } else {
        root.announce("Workspace state refreshed.", false)
      }
      stateReader.refresh()
    }
  }

  Process {
    id: summaryProc
    stdout: StdioCollector {}
    stderr: StdioCollector {}
    onExited: function(code) {
      root.generating = false
      if (code !== 0) {
        console.warn("pitwall", "summarize exited", code)
        root.announce("Summary unavailable. Your workspace was not changed.", true)
      } else {
        root.announce("Summary updated.", false)
      }
      stateReader.refresh()
    }
  }

  function generateSummary() {
    if (root.generating) return
    var args = [root.pitwallBinary, "summarize", "--agent", root.cfgAgent]
    if (root.cfgModel !== "") {
      args.push("--model")
      args.push(root.cfgModel)
    }
    summaryProc.command = args
    root.generating = true
    summaryProc.running = true
  }

  Item {
    id: buttonHolder
    anchors.fill: parent
    implicitWidth: button.implicitWidth
    implicitHeight: button.implicitHeight

    // Collapsed: flag mark only (no permanent wordmark).
    BarIconButton {
      id: button
      anchors.fill: parent
      bar: root.bar
      active: root.needsAttention
      tooltipText: root.tooltipText()
      onPressed: function(buttonCode) {
        if (buttonCode === Qt.RightButton) {
          stateReader.refresh()
          root.nowMs = Date.now()
        } else {
          root.toggle()
        }
      }

      iconComponent: Component {
        Image {
          width: Style.bar.iconCanvas
          height: Style.bar.iconCanvas
          source: "flag.svg"
          fillMode: Image.PreserveAspectFit
          smooth: false
          mipmap: false
        }
      }
    }

    // Unread badge: attention + completion only (stateReader.unreadCount).
    // No badge at zero. Text-only overlay, theme tint, no animation.
    Text {
      visible: stateReader.unreadCount > 0
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.rightMargin: -Style.space(2)
      anchors.topMargin: -Style.space(4)
      textFormat: Text.PlainText
      text: stateReader.unreadCount > 9 ? "9+" : String(stateReader.unreadCount)
      color: Color.accent
      font.family: Style.font.family
      font.pixelSize: Style.font.caption
      font.bold: true
      renderType: Text.NativeRendering
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(360))
    contentHeight: panel.fittedContentHeight(
      headerItem.height + railColumn.implicitHeight + Style.space(8), Style.space(560))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: {
        if (root.selectedId !== "") {
          root.selectedId = ""
          root.hoveredId = ""
        } else {
          root.close()
        }
      }
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onActivateRequested: {
        if (root.selectedSession()) root.focusSession(root.selectedSession())
        else if (root.primary) root.focusSession(root.primary)
      }

      Column {
        id: panelColumn
        anchors.fill: parent
        spacing: Style.space(8)

        // ---- fixed header (outside the scroll: stable hitboxes) ----
        Item {
          id: headerItem
          width: parent.width
          height: headerRow.implicitHeight

          Row {
            id: headerRow
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(6)

            Image {
              id: flagMark
              width: Style.space(14)
              height: Style.space(14)
              anchors.verticalCenter: parent.verticalCenter
              source: "flag.svg"
              fillMode: Image.PreserveAspectFit
              smooth: false
              mipmap: false
              visible: status !== Image.Error
            }

            // Single wordmark (split-color, palette-bound).
            Row {
              anchors.verticalCenter: parent.verticalCenter
              spacing: 0

              Text {
                textFormat: Text.PlainText
                text: "PIT"
                color: Color.foreground
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                font.bold: true
                renderType: Text.NativeRendering
              }

              Text {
                textFormat: Text.PlainText
                text: "WALL"
                color: Color.accent
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                font.bold: true
                renderType: Text.NativeRendering
              }

              Text {
                textFormat: Text.PlainText
                text: " \u00b7 " + stateReader.sessionCount
                color: Color.muted
                font.family: Style.font.family
                font.pixelSize: Style.font.bodySmall
                font.bold: true
                renderType: Text.NativeRendering
              }
            }
          }

          PanelActionButton {
            id: refreshButton
            anchors.right: gearButton.left
            anchors.rightMargin: Style.space(2)
            anchors.verticalCenter: parent.verticalCenter
            iconText: String.fromCodePoint(0xF0450)
            tooltipText: "Refresh workspace state (no AI)"
            focusable: true
            onClicked: root.refreshNow()

            NumberAnimation on rotation { // reset by onExited (see refreshProc)
              running: root.refreshing
              loops: Animation.Infinite
              from: 0
              to: 360
              duration: 900
              easing.type: Easing.OutCubic
            }
          }

          PanelActionButton {
            id: gearButton
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            iconText: String.fromCodePoint(0xF013)
            tooltipText: "Pitwall settings"
            focusable: true
            onClicked: {
              root.showSettings = !root.showSettings
              if (root.showSettings) root.refreshModels(root.cfgAgent)
            }
          }
        }

        Flickable {
        id: railFlick
        width: parent.width
        height: Math.min(railColumn.implicitHeight,
          Math.max(200, panel.contentHeight - headerItem.height - panelColumn.spacing))
        contentWidth: width
        contentHeight: railColumn.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        interactive: contentHeight > height
        ScrollBar.vertical: ScrollBar {
          policy: ScrollBar.AsNeeded
          contentItem: Rectangle {
            implicitWidth: Style.space(6)
            radius: width / 2
            color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.28)
          }
        }

           Column {
             id: railColumn
          width: railFlick.width
             spacing: Style.space(10)

             Text {
               visible: root.feedbackText !== ""
               width: parent.width
               textFormat: Text.PlainText
               text: root.feedbackText
               color: root.feedbackAttention ? Color.urgent : Color.muted
               font.family: Style.font.family
               font.pixelSize: Style.font.caption
               wrapMode: Text.Wrap
               maximumLineCount: 2
               elide: Text.ElideRight
               renderType: Text.NativeRendering
             }

           // ---- settings view (replaces rail while open) ----
          Column {
            visible: root.showSettings
            width: parent.width
            spacing: Style.space(8)

            PanelSectionHeader {
              width: parent.width
              text: "AI SUMMARY"
            }

            Dropdown {
              width: parent.width
              label: "Agent"
              value: root.cfgAgent
              options: ["opencode", "claude", "codex"]
              onChanged: function(v) {
                root.cfgAgent = v
                root.cfgModel = ""
                runFixed([root.pitwallBinary, "config", "set", "agent", v], function(code) {
                  if (code !== 0) console.warn("pitwall", "config set agent failed", code)
                  root.refreshModels(v)
                })
              }
            }

            Dropdown {
              width: parent.width
              label: "Model"
              value: root.modelValue()
              options: root.modelOptions
              onChanged: function(v) {
                var m = (v === "Agent default") ? "" : v
                root.cfgModel = m
                runFixed([root.pitwallBinary, "config", "set", "model", m], function(code) {
                  if (code !== 0) console.warn("pitwall", "config set model failed", code)
                })
              }
            }

            Toggle {
              width: parent.width
              label: "Summary"
              description: "Show the AI workspace summary"
              checked: root.cfgSummary
              onClicked: {
                root.cfgSummary = !root.cfgSummary
                runFixed([root.pitwallBinary, "config", "set", "summary_enabled", root.cfgSummary ? "true" : "false"], function(code) {
                  if (code !== 0) console.warn("pitwall", "config set summary failed", code)
                })
              }
            }
          }

          // ---- workspace view ----
          Column {
            visible: !root.showSettings
            width: parent.width
            spacing: Style.space(10)

            // ---- AI brief card (one bounded surface) ----
            Column {
             id: briefSurface
             visible: root.cfgSummary
             width: parent.width
             spacing: Style.space(4)

              // Hover pause covers the whole AI_Brief_Surface — caption row,
              // ticker card and status line — not just the text row (4.1).
              // A HoverHandler is not an Item, so Column does not position it.
              HoverHandler {
                id: briefHover
              }

              // One bounded card carries the whole AI_Brief_Surface: caption
              // row, single-line ticker, pass-progress underline and the
              // status line live inside this one border, so the brief never
              // reads as stacked lines of equal weight (6.5, 7.3). The accent
              // border is what marks the brief out from the rail below it.
              Rectangle {
                id: briefCard
                width: parent.width
                height: briefCardColumn.implicitHeight + Style.space(12)
                radius: Style.space(3)
                color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.045)
                border.width: 1
                border.color: Qt.rgba(Color.accent.r, Color.accent.g, Color.accent.b, 0.35)

                Column {
                  id: briefCardColumn
                  anchors.left: parent.left
                  anchors.right: parent.right
                  anchors.top: parent.top
                  anchors.leftMargin: Style.space(6)
                  anchors.rightMargin: Style.space(6)
                  anchors.topMargin: Style.space(6)
                  spacing: Style.space(4)

                  // (a) caption row: flag mark, label, current indicator.
                  // ● means the cached brief is current, ○ that it is not
                  // (6.1) — the word beside it in the status line carries the
                  // meaning, the glyph never carries it alone.
                  Row {
                    width: parent.width
                    spacing: Style.space(4)

                    Image {
                      width: Style.space(11)
                      height: Style.space(11)
                      anchors.verticalCenter: parent.verticalCenter
                      source: "flag.svg"
                      fillMode: Image.PreserveAspectFit
                      smooth: false
                      mipmap: false
                      visible: status !== Image.Error
                    }

                    Text {
                      anchors.verticalCenter: parent.verticalCenter
                      textFormat: Text.PlainText
                      text: "AI BRIEF"
                      color: Color.muted
                      font.family: Style.font.family
                      font.pixelSize: Style.font.caption
                      font.bold: true
                      renderType: Text.NativeRendering
                    }

                    Text {
                      anchors.verticalCenter: parent.verticalCenter
                      textFormat: Text.PlainText
                      text: root.summary && root.summary.status === "ready" ? "●" : "○"
                      color: root.summary && root.summary.status === "ready" ? Color.accent : Color.muted
                      font.family: Style.font.family
                      font.pixelSize: Style.font.caption
                      renderType: Text.NativeRendering
                    }
                  }

                  // (b) the single-line ticker viewport plus its expand
                  // affordance. `visible` keys on the root function, never on
                  // a child's `visible`: QQuickItem.visible reads *effective*
                  // visibility, so a parent keyed on a child would latch off
                  // and never come back.
                  Item {
                    id: tickerLine
                    visible: root.summaryShort() !== ""
                    width: parent.width
                    height: tickerViewport.height

                    Item {
                      id: tickerViewport
                      visible: root.summaryShort() !== ""
                      anchors.left: parent.left
                      anchors.right: expandGlyph.left
                      anchors.rightMargin: Style.space(4)
                      height: root.summaryExpanded ? expandedSummary.implicitHeight + Style.space(10) : marqueeText.implicitHeight + Style.space(10)
                      clip: true
                      // Recompute the pass duration from the new travel distance
                      // (3.4). Restarting keeps the current x, so a resize does not
                      // throw away where the brief had got to.
                      onWidthChanged: if (tickerPass && tickerViewport.visible) tickerPass.restart()

                      Item {
                        id: tickerTrack
                        visible: !root.summaryExpanded
                        y: Style.space(5)
                        // Measured natural width of the complete brief (3.2). The
                        // viewport clips; travel exposes the tail (1.1, 1.3).
                        width: marqueeText.implicitWidth
                        height: marqueeText.implicitHeight

                        // Start a fresh pass with the first character at the right
                        // edge (2.2, 2.4, 5.1). `from` reads tickerTrack.x, so x has
                        // to be moved before the restart, not just the mirror.
                        function startPassFromRightEdge() {
                          root.tickerOffset = tickerViewport.width
                          tickerTrack.x = tickerViewport.width
                          if (tickerViewport.visible)
                            tickerPass.restart()
                        }

                        // Imperative, not a binding on x: the animation must be free
                        // to drive x, and a binding would also fight onXChanged.
                        Component.onCompleted: {
                          x = isNaN(root.tickerOffset) ? tickerViewport.width : root.tickerOffset
                          // Re-arm explicitly so `from` is read after x is restored,
                          // whatever order completion and the `running` binding run in.
                          if (tickerViewport.visible)
                            tickerPass.restart()
                        }
                        // Cheap mirror, no timer. Nothing binds x to tickerOffset, so
                        // this cannot cycle: the write is one-way, track -> root.
                        onXChanged: root.tickerOffset = x

                        Text {
                          id: marqueeText
                          textFormat: Text.PlainText
                          text: root.summaryShort()
                          width: implicitWidth
                          height: implicitHeight
                          elide: Text.ElideNone
                          color: Color.foreground
                          font.family: Style.font.family
                          font.pixelSize: Style.font.bodySmall
                          renderType: Text.NativeRendering
                          // A new brief is read from its start, never from its middle
                          // (5.1). QQuickText lays out before it emits textChanged,
                          // so implicitWidth is already the new measured width here
                          // and the restart picks up the new duration (5.2, 5.3).
                          onTextChanged: tickerTrack.startPassFromRightEdge()
                        }

                        // One pass per animation run, restarted explicitly, rather
                        // than loops: Animation.Infinite. `from`/`to`/`duration` are
                        // baked in when a run starts and ignored until it restarts,
                        // so an infinitely looping run would keep replaying the
                        // geometry of its first pass: it could neither resume from a
                        // held offset (4.5) nor recompute duration after a resize
                        // (3.4). An explicit restart re-reads all three.
                        NumberAnimation on x {
                          id: tickerPass
                          from: tickerTrack.x                     // resumes an interrupted pass (4.5)
                          to: -marqueeText.implicitWidth          // last character clears the left edge (2.3)
                          duration: root.tickerDurationMs(tickerTrack.x, marqueeText.implicitWidth)
                          easing.type: Easing.Linear              // (3.5)
                          running: tickerViewport.visible         // empty brief hides the viewport and stops it (5.4)
                          // Gated on `running` because Qt refuses setPaused() on a
                          // stopped animation; the gate also re-applies the pause
                          // after any restart(), which clears it (4.1, 4.3, 4.4).
                          paused: tickerPass.running && (!root.opened || root.summaryExpanded || briefHover.hovered)
                          onFinished: tickerTrack.startPassFromRightEdge()
                        }
                      }

                      Text {
                        id: expandedSummary
                        visible: root.summaryExpanded
                        x: Style.space(5)
                        y: Style.space(5)
                        width: parent.width - Style.space(10)
                        textFormat: Text.PlainText
                        wrapMode: Text.Wrap
                        text: root.summaryShort()
                        color: Color.foreground
                        font.family: Style.font.family
                        font.pixelSize: Style.font.bodySmall
                        renderType: Text.NativeRendering

                      }
                    }

                    // Expand affordance. Both codepoints are the chevrons the
                    // rail group headers already ship, so no unverified glyph
                    // enters the plugin.
                    Text {
                      id: expandGlyph
                      anchors.right: parent.right
                      anchors.verticalCenter: parent.verticalCenter
                      textFormat: Text.PlainText
                      text: root.summaryExpanded ? String.fromCodePoint(0xF0140) : String.fromCodePoint(0xF0142)
                      color: Color.muted
                      font.family: Style.font.family
                      font.pixelSize: Style.font.bodySmall
                      renderType: Text.NativeRendering
                    }

                    MouseArea {
                      anchors.fill: parent
                      acceptedButtons: Qt.LeftButton
                      cursorShape: Qt.PointingHandCursor
                      onClicked: root.summaryExpanded = !root.summaryExpanded
                    }
                  }

                  // (c) pass-progress underline. A width *binding* driven by
                  // the ticker's own x — not a second animation — so the
                  // ≤2 concurrent non-ticker animation budget is untouched
                  // (7.11).
                  Rectangle {
                    visible: root.summaryShort() !== "" && !root.summaryExpanded
                    width: parent.width
                    height: Style.space(2)
                    radius: height / 2
                    color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.12)

                    Rectangle {
                      // Fraction of the pass already travelled:
                      //   1 - (x + contentW) / (viewportW + contentW)
                      // 0 with the first character at the right edge,
                      // 1 once the last character has cleared the left edge.
                      // Reads only geometry that is set from above (viewport
                      // width, measured text width, animated x); nothing here
                      // feeds back into any of them, so there is no loop.
                      width: {
                        var contentW = marqueeText.implicitWidth
                        var travel = tickerViewport.width + contentW
                        if (!(travel > 0)) return 0
                        var f = 1 - (tickerTrack.x + contentW) / travel
                        return parent.width * Math.max(0, Math.min(1, f))
                      }
                      height: parent.height
                      radius: parent.radius
                      color: Color.accent
                    }
                  }

                  // (d) status line: one caption-weight line beside the
                  // generate control, inside the same card — never a second
                  // equal-weight stacked line (6.5). Covers ready (empty),
                  // stale, error, unavailable, absent and in-progress
                  // (6.1–6.4, 6.6); while a run is in progress the cached
                  // brief above stays on screen (6.6).
                  Item {
                    id: briefStatusRow
                    visible: root.briefStatusText() !== "" || root.briefNeedsGenerate()
                    width: parent.width
                    height: Math.max(briefStatusLabel.implicitHeight,
                      root.briefNeedsGenerate() ? generateBriefButton.implicitHeight : 0)

                    Text {
                      id: briefStatusLabel
                      anchors.left: parent.left
                      anchors.right: root.briefNeedsGenerate() ? generateBriefButton.left : parent.right
                      anchors.rightMargin: Style.space(6)
                      anchors.verticalCenter: parent.verticalCenter
                      textFormat: Text.PlainText
                      elide: Text.ElideRight
                      maximumLineCount: 1
                      text: root.briefStatusText()
                      color: root.briefStatusAttention() ? Color.urgent : Color.muted
                      font.family: Style.font.family
                      font.pixelSize: Style.font.caption
                      renderType: Text.NativeRendering
                    }

                    Button {
                      id: generateBriefButton
                      visible: root.briefNeedsGenerate()
                      anchors.right: parent.right
                      anchors.verticalCenter: parent.verticalCenter
                      text: "Generate summary"
                      onClicked: root.generateSummary()
                    }
                  }

                  // Needs-attention line: only when the cached summary
                  // explicitly contains such a section. Never invented, and
                  // caption weight so it stays inside the card's hierarchy.
                  Text {
                    visible: root.attentionText() !== ""
                    width: parent.width
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    maximumLineCount: 1
                    text: "! " + root.attentionText()
                    color: Color.urgent
                    font.family: Style.font.family
                    font.pixelSize: Style.font.caption
                    renderType: Text.NativeRendering
                  }
                }
              }
            }

            // ---- grouped workspace rail ----
            // Semantic priority first (tier order fixed), recency within.
            // Groups key on shared project id; unprojected entries stand
            // alone under their own session id. Identity untouched:
            // grouping is presentation over stable sess_*/proj_* ids.
            // Collapse state is per group key; geometry stays in-flow.
            // Single-row group header: [tier icon] name · count [unread] [chevron].
            // The icon carries the tier; no separate caption row exists.
            Repeater {
              model: root.railGroups
              delegate: Column {
                width: railColumn.width
                spacing: Style.space(4)

                 Item {
                   width: parent.width
                   height: Style.font.bodySmall + Style.space(4)

                   Rectangle {
                     anchors.fill: parent
                     color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.025)
                     radius: Style.space(2)
                   }

                   Rectangle {
                     anchors.left: parent.left
                     anchors.right: parent.right
                     anchors.bottom: parent.bottom
                     height: 1
                     color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.12)
                   }

                   Row {
                    anchors.left: parent.left
                    anchors.right: toggleGlyph.left
                    anchors.rightMargin: Style.space(4)
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: Style.space(6)

                    Text {
                      textFormat: Text.PlainText
                      anchors.verticalCenter: parent.verticalCenter
                      text: root.tierIcon(modelData.tier)
                      color: Qt.darker(Color.foreground, 1.4)
                      font.family: Style.font.family
                      font.pixelSize: Style.font.caption
                      renderType: Text.NativeRendering
                    }

                    Text {
                      textFormat: Text.PlainText
                      anchors.verticalCenter: parent.verticalCenter
                      elide: Text.ElideRight
                      text: modelData.label
                      color: Color.foreground
                      font.family: Style.font.family
                      font.pixelSize: Style.font.bodySmall
                      renderType: Text.NativeRendering
                    }

                    Text {
                      visible: modelData.unread > 0
                      textFormat: Text.PlainText
                      anchors.verticalCenter: parent.verticalCenter
                      text: "\u25cf" + modelData.unread
                      color: Color.accent
                      font.family: Style.font.family
                      font.pixelSize: Style.font.caption
                      renderType: Text.NativeRendering
                    }
                  }

                  Text {
                    id: toggleGlyph
                    textFormat: Text.PlainText
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    text: modelData.collapsed ? String.fromCodePoint(0xF0142) : String.fromCodePoint(0xF0140)
                    color: Qt.darker(Color.foreground, 1.4)
                    font.family: Style.font.family
                    font.pixelSize: Style.font.bodySmall
                    renderType: Text.NativeRendering
                  }

                  MouseArea {
                    anchors.fill: parent
                    acceptedButtons: Qt.LeftButton
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.toggleGroup(modelData.key)
                  }
                }

                Repeater {
                  model: (modelData.kind === "group" && !modelData.collapsed) ? modelData.entries : []
                  delegate: SessionBar {
                    width: railColumn.width
                    entry: modelData.ref
                    resumable: modelData.hist
                    frac: modelData.hist ? 0.25 : root.barFrac(modelData.ref.age_secs)
                    selected: root.selectedId === modelData.selId
                    dimmed: root.selectedId !== "" && root.selectedId !== modelData.selId
                    showCard: root.selectedId === modelData.selId
                    detailText: modelData.hist ? root.detailFor(modelData.ref, true) : root.detailFor(modelData.ref, false)
                    notifs: root.notifsFor(modelData.selId)
                    onNotifClicked: function(nid) { root.markNotifRead(nid) }
                    canFocus: !modelData.hist
                    canStop: !modelData.hist
                    canClose: !modelData.hist
                    canResume: modelData.hist
                    canAssign: !modelData.hist
                    resumeTooltip: modelData.hist ? root.resumeTooltipFor(modelData.ref) : ""
                    onClicked: {
                      root.selectedId = (root.selectedId === modelData.selId) ? "" : modelData.selId
                    }
                    onHovered: function(h) {
                      root.hoveredId = h ? modelData.selId : ""
                    }
                    onFocusRequested: root.focusSession(modelData.ref)
                    onStopRequested: root.stopSession(modelData.ref)
                    onCloseRequested: root.closeSession(modelData.ref)
                    onResumeRequested: root.resumeCheckpoint(String(modelData.ref.session_id || ""))
                  }
                }
              }
            }
          }
        }
      }
      }
    }
  }

  // ---- grouped rail model ----
  // Tier order is fixed (semantic priority); recency rules inside each
  // tier via the pre-sorted live/resumable arrays. Groups key on shared
  // project id, singletons on their own id. Collapsed state is a plain
  // object keyed by group key (reassigned wholesale so bindings fire).
  // Groups start collapsed (expandedGroups empty); headers toggle.
  // Collapsed default keeps the rail a project map, not a process list.
  readonly property var tierOrder: ["pitwall-native", "agents", "workspace", "system"]
  property var expandedGroups: ({})
  function toggleGroup(key) {
    var next = {}
    for (var k in expandedGroups) next[k] = expandedGroups[k]
    next[key] = !next[key]
    expandedGroups = next
  }

  // Tier icons: verified Nerd codepoints only (coverage table in the
  // plugin README). Glyph never carries meaning alone — tier word follows.
  // Tier icon by tier key (verified Nerd codepoints; word removed —
  // the project label beside it carries the name).
  function tierIcon(tier) {
    if (tier === "pitwall-native") return String.fromCodePoint(0xF024)
    if (tier === "agents") return String.fromCodePoint(0xF007)
    if (tier === "workspace") return String.fromCodePoint(0xF07B)
    return String.fromCodePoint(0xF0AD)
  }

  function groupDisplayName(entries) {
    if (entries.length === 0) return ""
    var ref0 = entries[0].ref
    var name = ref0.project_name || (ref0.project && ref0.project.name) || "session"
    if (entries.length === 1) return String(name)
    return String(name) + " \u00b7 " + entries.length
  }

  readonly property var railGroups: {
    var out = []
    var live = liveSessions
    var hist = resumable.slice(0, 5)
    var groups = {}
    var order = []
    var i, s, gkey
    for (i = 0; i < live.length; i++) {
      s = live[i] || {}
      gkey = String(s.group || s.id || ("live-" + i))
      if (!groups[gkey]) {
        groups[gkey] = { key: gkey, tier: String(s.tier || "workspace"), live: [], hist: [] }
        order.push(gkey)
      }
      groups[gkey].live.push(s)
    }
    for (i = 0; i < hist.length; i++) {
      var c = hist[i] || {}
      gkey = String(c.group || ("hist-" + String(c.session_id || i)))
      if (!groups[gkey]) {
        groups[gkey] = { key: gkey, tier: String(c.tier || "workspace"), live: [], hist: [] }
        order.push(gkey)
      }
      groups[gkey].hist.push(c)
    }
    // Unread inbox indexed once per rebuild (small arrays by contract).
    var notifs = stateReader.notifications
    function groupUnread(gkey, entries) {
      var n = 0
      for (var ni = 0; ni < notifs.length; ni++) {
        var nb = notifs[ni] || {}
        var sid = String(nb.session_id || "")
        var matched = false
        for (var ei = 0; ei < entries.length && !matched; ei++) {
          var er = entries[ei].ref || {}
          if (String(er.id || "") !== "" && String(er.id) === sid) matched = true
          else if (String(er.session_id || "") !== "" && String(er.session_id) === sid) matched = true
        }
        if (matched || (String(nb.project_id || "") !== "" && String(nb.project_id) === gkey)) n++
      }
      return n
    }
    var t, k, g, entries, collapsed
    for (var ti = 0; ti < tierOrder.length; ti++) {
      t = tierOrder[ti]
      for (var oi = 0; oi < order.length; oi++) {
        k = order[oi]
        g = groups[k]
        if (g.tier !== t) continue
        entries = []
        for (var li = 0; li < g.live.length; li++) {
          entries.push({ kind: "live", hist: false, ref: g.live[li], selId: String(g.live[li].id || "") })
        }
        for (var hi = 0; hi < g.hist.length; hi++) {
          entries.push({ kind: "hist", hist: true, ref: g.hist[hi], selId: "r:" + String(g.hist[hi].session_id || "") })
        }
        if (entries.length === 0) continue
        collapsed = !expandedGroups[g.key]
        out.push({ kind: "group", key: g.key, tier: t, label: groupDisplayName(entries),
          collapsed: collapsed, entries: collapsed ? [] : entries,
          unread: groupUnread(g.key, entries) })
      }
    }
    return out
  }

  function entryHasUnread(selId) {
    var notifs = stateReader.notifications
    var sid = (selId.charAt(0) === "r") ? selId.slice(2) : selId
    for (var i = 0; i < notifs.length; i++) {
      if (String((notifs[i] || {}).session_id || "") === sid) return true
    }
    return false
  }

  // Notifications scoped to one entry id (live or r:-prefixed).
  // Number([]) guards the id path: read() refuses id <= 0.
  function markNotifRead(id) {
    var nid = Number(id) || 0
    if (nid <= 0) return
    runFixed([root.pitwallBinary, "notifications", "read", String(nid)], function(code) {
      if (code !== 0) {
        console.warn("pitwall", "notification read exited", code)
        root.announce("Could not mark notification read.", true)
      } else {
        root.announce("Notification marked read.", false)
      }
      stateReader.refresh()
    })
  }

  function notifsFor(selId) {
    var notifs = stateReader.notifications
    var sid = (selId.charAt(0) === "r") ? selId.slice(2) : selId
    var out = []
    for (var i = 0; i < notifs.length; i++) {
      var nb = notifs[i] || {}
      if (String(nb.session_id || "") === sid) out.push(nb)
    }
    return out
  }

  // ---- selection model: pinned clicks only. Hover highlights bars
  // (SessionBar hovered flag) but never opens, moves, or steals a card.
  function selectedSession() {
    return findEntry(selectedId)
  }

  function findEntry(id) {
    if (id === "") return null
    if (id.charAt(0) === "r") {
      var want = id.slice(2)
      for (var i = 0; i < resumable.length; i++) {
        if (String(resumable[i].session_id) === want) return resumable[i]
      }
      return null
    }
    for (var j = 0; j < liveSessions.length; j++) {
      if (String(liveSessions[j].id) === id) return liveSessions[j]
    }
    return null
  }

  // Detail text for a pinned card. Live sessions: deterministic detail
  // (never AI-inferred). Resumable: checkpoint facts. Caller passes the
  // entry directly, so hover state can never reroute content.
  function detailFor(s, isResumable) {
    if (!s) return ""
    if (!isResumable) {
      // Live session: deterministic detail (never AI-inferred).
      // State words describe the observation only: active = running now,
      // idle = sleeping, waiting = stopped job present, unknown = unclear.
      // Sleeping/idle never implies useless or safe-to-kill.
      var parts = []
      var sstate = String(s.state || "unknown")
      var stateWord = sstate === "running" ? "active"
        : sstate === "sleeping" ? "idle"
        : sstate === "stopped" ? "waiting" : "unknown"
      parts.push("state  " + stateWord)
      var a = s.agent || {}
      parts.push("agent  " + String(a.kind || "unknown") + " · " + String(a.confidence || "unknown"))
      var w = s.window || null
      var win = w ? String(w.class || "?") : "no window"
      var ws = (w && w.workspace) ? " · workspace " + w.workspace : ""
      parts.push("window  " + win + ws)
      var p = s.project || null
      if (p) {
        var git = p.branch ? p.branch : "no branch"
        if (p.git_clean === true) git += " · clean"
        else if (p.git_clean === false) git += " · dirty"
        parts.push("project  " + String(p.name || "?") + " (" + git + ")")
      }
      parts.push(confReason(String(a.kind || "unknown"), String(a.confidence || "unknown"), String(s.role || "unknown")))
      return parts.join("\n")
    }
    // Resumable checkpoint.
    var bits = []
    bits.push("checkpoint " + stateReader.ageText(s.created_at, nowMs))
    if (s.branch) bits.push("branch " + s.branch)
    if (s.note) bits.push(String(s.note))
    else bits.push("no note")
    return bits.join("\n")
  }

  function resumeTooltipFor(s) {
    var dir = s ? String(s.project_dir || "") : ""
    if (dir === "") return "Resume unavailable"
    return "Resume: open terminal at " + dir
  }

  // The viewport clips; the marquee track carries the complete brief text and
  // horizontal motion exposes the tail. No truncation, no filler.
  function summaryShort() {
    if (!summary) return ""
    if (summary.status === "error") return ""
    return String(summary.text || "").replace(/\s+/g, " ").trim()
  }

  // Status line for the brief card. Every string is either a fixed panel
  // string or a value the Summary_Record itself carries (its status word and
  // its own failure message) — the panel adds no interpretation of the brief
  // (6.7). `ready` returns empty: the ● indicator alone says "current".
  function briefStatusText() {
    if (root.generating) return "Generating…"
    if (!summary) return "No brief yet"
    var st = String(summary.status || "")
    if (st === "ready") return ""
    if (st === "stale") return "Outdated"
    var msg = String(summary.message || "")
    return msg !== "" ? "Unavailable — " + msg : "Unavailable"
  }

  // Stale, error and unavailable are the states the human has to act on;
  // generating and absent are neutral.
  function briefStatusAttention() {
    if (root.generating || !summary) return false
    var st = String(summary.status || "")
    return st === "stale" || st === "error" || st === "unavailable"
  }

  // The generate control accompanies every non-current state — stale, error,
  // unavailable and absent (6.2, 6.3, 6.4) — and stands down while a run is
  // already in progress so one click cannot start two.
  function briefNeedsGenerate() {
    if (root.generating) return false
    if (!summary) return true
    return String(summary.status || "") !== "ready"
  }

  // "Needs attention" line, only when the cached summary explicitly
  // contains such a section. Otherwise absent (never invented).
  function attentionText() {
    if (!summary || summary.status !== "ready") return ""
    var m = String(summary.text || "").match(/needs attention:?([^\n]*)/i)
    if (!m) return ""
    var line = m[1].trim().replace(/\s+/g, " ")
    if (line === "") return ""
    return line.length > 140 ? line.slice(0, 137) + "…" : line
  }

  function modelValue() {
    if (root.cfgModel !== "") {
      for (var i = 0; i < modelOptions.length; i++) {
        if (modelOptions[i] === root.cfgModel) return root.cfgModel
      }
    }
    return "Agent default"
  }

}
