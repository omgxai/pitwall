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
      return
    }
    target.activate()
    root.close()
  }

  function closeSession(s) {
    var target = findToplevel(s)
    if (!target) {
      console.warn("pitwall", "Close target gone or ambiguous")
      return
    }
    target.close()
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
      return
    }
    runFixed(["pitwall", "resume", "--session-id", sid], function(code) {
      if (code !== 0) console.warn("pitwall", "resume exited", code, "for", sid)
    })
  }

  function stopSession(s) {
    // SIGTERM only, numeric pid only, explicit click only. Never SIGKILL,
    // never a shell. The session root owns the tree; the terminal owns
    // teardown policy from there.
    var pid = Number(s && s.root_pid)
    if (!isFinite(pid) || pid <= 1 || Math.floor(pid) !== pid) {
      console.warn("pitwall", "Refusing stop with invalid pid")
      return
    }
    runFixed(["/usr/bin/kill", "-s", "TERM", String(pid)], function(code) {
      if (code !== 0) console.warn("pitwall", "stop exited", code, "for pid", pid)
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
    modelsProc.command = ["pitwall", "models", "--agent", String(agent || "opencode")]
    modelsProc.running = true
  }

  Process {
    id: summaryProc
    stdout: StdioCollector {}
    stderr: StdioCollector {}
    onExited: function(code) {
      root.generating = false
      if (code !== 0) console.warn("pitwall", "summarize exited", code)
      stateReader.refresh()
    }
  }

  function generateSummary() {
    if (root.generating) return
    var args = ["pitwall", "summarize", "--agent", root.cfgAgent]
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
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(360))
    contentHeight: panel.fittedContentHeight(column.implicitHeight, Style.space(560))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onActivateRequested: {
        if (root.selectedSession()) root.focusSession(root.selectedSession())
        else if (root.primary) root.focusSession(root.primary)
      }

      Flickable {
        id: railFlick
        anchors.fill: parent
        contentWidth: width
        contentHeight: column.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        interactive: contentHeight > height
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

        Column {
          id: column
          width: railFlick.width
          spacing: Style.space(10)

          // ---- fixed header: identity left, gear right ----
          Item {
            width: parent.width
            height: Math.max(flagMark.height, gearButton.height)

            Row {
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

              Text {
                textFormat: Text.PlainText
                anchors.verticalCenter: parent.verticalCenter
                // Split wordmark: base + accent tail, palette-bound.
                text: "PIT"
                color: Color.foreground
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                font.bold: true
                renderType: Text.NativeRendering
              }

              Text {
                textFormat: Text.PlainText
                anchors.verticalCenter: parent.verticalCenter
                text: "WALL"
                color: Color.accent
                font.family: Style.font.family
                font.pixelSize: Style.font.body
                font.bold: true
                renderType: Text.NativeRendering
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
                runFixed(["pitwall", "config", "set", "agent", v], function(code) {
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
                runFixed(["pitwall", "config", "set", "model", m], function(code) {
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
                runFixed(["pitwall", "config", "set", "summary_enabled", root.cfgSummary ? "true" : "false"], function(code) {
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

            // ---- AI summary ticker ----
            Column {
              visible: root.cfgSummary
              width: parent.width
              spacing: Style.space(4)

              PanelSectionHeader {
                width: parent.width
                text: "AI SUMMARY"
              }

              Item {
                width: parent.width
                height: summaryBody.implicitHeight
                clip: true

                Text {
                  id: summaryBody
                  width: parent.width
                  textFormat: Text.PlainText
                  wrapMode: root.summaryExpanded ? Text.Wrap : Text.NoWrap
                  elide: root.summaryExpanded ? Text.ElideNone : Text.ElideRight
                  maximumLineCount: root.summaryExpanded ? 6 : 1
                  text: root.summaryText()
                  color: Color.foreground
                  font.family: Style.font.family
                  font.pixelSize: Style.font.bodySmall
                  renderType: Text.NativeRendering

                  // Slow horizontal ping-pong only when overflowing, panel
                  // open, and not hovered. Rests readable at both ends so a
                  // still glance usually catches text (no blank marathons).
                  // No vertical motion, no speed above ~30px/s.
                  property bool overflow: implicitWidth > width + 2
                  property bool drift: overflow && !root.summaryExpanded && root.opened && !tickHover.hovered
                  property int driftSpan: Math.max(0, summaryBody.implicitWidth - summaryBody.width)
                  property int driftMs: Math.max(4000, driftSpan * 33)
                  SequentialAnimation on x {
                    id: tickAnim
                    running: summaryBody.drift
                    loops: Animation.Infinite
                    PauseAnimation { duration: 2500 }
                    NumberAnimation {
                      from: 0
                      to: -summaryBody.driftSpan
                      duration: summaryBody.driftMs
                      easing.type: Easing.InOutQuad
                    }
                    PauseAnimation { duration: 2500 }
                    NumberAnimation {
                      from: -summaryBody.driftSpan
                      to: 0
                      duration: summaryBody.driftMs
                      easing.type: Easing.InOutQuad
                    }
                  }
                  onDriftChanged: if (!drift) x = 0

                  HoverHandler {
                    id: tickHover
                  }

                  MouseArea {
                    anchors.fill: parent
                    acceptedButtons: Qt.LeftButton
                    cursorShape: Qt.PointingHandCursor
                    onClicked: root.summaryExpanded = !root.summaryExpanded
                  }
                }
              }

              Row {
                width: parent.width
                spacing: Style.space(8)

                Button {
                  visible: !root.summary && !root.generating
                  text: "Generate summary"
                  onClicked: root.generateSummary()
                }

                Text {
                  visible: root.generating
                  textFormat: Text.PlainText
                  text: "Generating…"
                  color: Qt.darker(Color.foreground, 1.4)
                  font.family: Style.font.family
                  font.pixelSize: Style.font.caption
                  renderType: Text.NativeRendering
                }

                Text {
                  visible: !!root.summary && root.summary.status === "error" && !root.generating
                  textFormat: Text.PlainText
                  text: "Unavailable" + (root.summary && root.summary.message ? " — " + root.summary.message : "")
                  color: Qt.darker(Color.foreground, 1.4)
                  font.family: Style.font.family
                  font.pixelSize: Style.font.caption
                  renderType: Text.NativeRendering
                }
              }
            }

            // ---- session rail ----
            Repeater {
              model: root.liveSessions
              delegate: SessionBar {
                width: column.width
                entry: modelData
                resumable: false
                frac: root.barFrac(modelData.age_secs)
                selected: root.selectedId === modelData.id
                dimmed: root.selectedId !== "" && root.selectedId !== modelData.id
                onClicked: {
                  root.selectedId = (root.selectedId === modelData.id) ? "" : modelData.id
                }
                onHovered: function(h) {
                  root.hoveredId = h ? modelData.id : ""
                }
              }
            }

            // ---- attached toast for hovered/pinned session ----
            Rectangle {
              visible: root.toastEntry() !== null
              width: parent.width
              height: toastColumn.implicitHeight + Style.space(16)
              color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.05)
              border.width: 1
              border.color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.22)

              Behavior on opacity {
                NumberAnimation { duration: 140; easing.type: Easing.OutCubic }
              }

              Column {
                id: toastColumn
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: Style.space(8)
                spacing: Style.space(4)

                Text {
                  width: parent.width
                  textFormat: Text.PlainText
                  wrapMode: Text.Wrap
                  maximumLineCount: 4
                  elide: Text.ElideRight
                  text: root.toastText()
                  color: Color.foreground
                  font.family: Style.font.family
                  font.pixelSize: Style.font.bodySmall
                  renderType: Text.NativeRendering
                }

                Row {
                  spacing: Style.space(4)

                  PanelActionButton {
                    visible: root.toastIsLive()
                    iconText: String.fromCodePoint(0xF034E)
                    tooltipText: "Focus terminal"
                    focusable: true
                    onClicked: {
                      var s = root.toastEntry()
                      if (s) root.focusSession(s)
                    }
                  }

                  PanelActionButton {
                    visible: root.toastIsLive()
                    iconText: "■"
                    tooltipText: "Stop session processes (SIGTERM)"
                    focusable: true
                    onClicked: {
                      var s = root.toastEntry()
                      if (s) root.stopSession(s)
                    }
                  }

                  PanelActionButton {
                    visible: root.toastIsLive()
                    iconText: "✕"
                    tooltipText: "Close window"
                    focusable: true
                    onClicked: {
                      var s = root.toastEntry()
                      if (s) root.closeSession(s)
                    }
                  }

                  PanelActionButton {
                    visible: !root.toastIsLive() && root.toastEntry() !== null
                    iconText: "▶"
                    tooltipText: root.resumeTooltip()
                    focusable: true
                    onClicked: {
                      var s = root.toastEntry()
                      if (s) root.resumeCheckpoint(String(s.session_id || ""))
                    }
                  }
                }
              }
            }

            // ---- resumable history rail ----
            PanelSectionHeader {
              visible: root.resumable.length > 0
              width: parent.width
              text: "RESUME"
            }

            Repeater {
              model: Math.min(root.resumable.length, 5)
              delegate: SessionBar {
                width: column.width
                entry: root.resumable[index]
                resumable: true
                frac: 0.25
                selected: root.selectedId === ("r:" + root.resumable[index].session_id)
                dimmed: root.selectedId !== "" && root.selectedId !== ("r:" + root.resumable[index].session_id)
                onClicked: {
                  var sid = "r:" + root.resumable[index].session_id
                  root.selectedId = (root.selectedId === sid) ? "" : sid
                }
                onHovered: function(h) {
                  root.hoveredId = h ? ("r:" + root.resumable[index].session_id) : ""
                }
              }
            }

            Text {
              visible: root.resumable.length > 5
              width: parent.width
              textFormat: Text.PlainText
              text: "+" + (root.resumable.length - 5) + " more checkpoints"
              color: Qt.darker(Color.foreground, 1.4)
              font.family: Style.font.family
              font.pixelSize: Style.font.caption
              renderType: Text.NativeRendering
            }
          }
        }
      }
    }
  }

  // ---- selection + toast model ----
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

  function hoveredSession() {
    if (hoveredId === "" || hoveredId === selectedId) return null
    return findEntry(hoveredId)
  }

  function toastEntry() {
    return selectedSession() || hoveredSession()
  }

  function toastIsLive() {
    var s = toastEntry()
    return !!s && s.id !== undefined
  }

  function toastText() {
    var s = toastEntry()
    if (!s) return ""
    if (s.id !== undefined) {
      // Live session: deterministic detail (never AI-inferred).
      var parts = []
      var a = s.agent || {}
      parts.push("agent  " + String(a.kind || "unknown") + " · " + String(a.confidence || "unknown"))
      var w = s.window || null
      var win = w ? String(w.class || "?") : "no window"
      var ws = (w && w.workspace) ? " · workspace " + w.workspace : ""
      parts.push("window  " + win + ws + " · " + Number(s.process_count || 0) + " procs")
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

  function resumeTooltip() {
    var s = toastEntry()
    var dir = s ? String(s.project_dir || "") : ""
    if (dir === "") return "Resume unavailable"
    return "Resume: open terminal at " + dir
  }

  function summaryText() {
    if (!summary) return "No summary yet."
    if (summary.status === "error") return "Summary unavailable."
    return String(summary.text || "No summary yet.")
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
