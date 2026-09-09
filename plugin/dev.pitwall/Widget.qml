import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs.Commons
import qs.Ui

// Pitwall bar widget + popup. Extends the shared Panel base (IPC open/close
// lifecycle) exactly like first-party popup widgets. Renders the M2
// state.json artifact only — no SQLite, no /proc, no polling.
Panel {
  id: root
  moduleName: "dev.pitwall"
  ipcTarget: "dev.pitwall"

  visible: stateReader.sessionCount > 0
  implicitWidth: buttonHolder.implicitWidth
  implicitHeight: buttonHolder.implicitHeight

  property double nowMs: Date.now()

  readonly property var primary: stateReader.primarySession
  readonly property var others: stateReader.otherSessions
  readonly property bool hasResumable: stateReader.resumable.length > 0
  readonly property bool stale: stateReader.isStale(nowMs)

  // Attention = a stopped (waiting) session or stale data. Drives the
  // bar's urgent tint; state itself is always icon + text, never color alone.
  readonly property bool needsAttention: {
    if (stale) return true
    var ss = stateReader.sessions
    for (var i = 0; i < ss.length; i++) {
      if (String((ss[i] || {}).state || "") === "stopped") return true
    }
    return false
  }

  readonly property string primaryState: {
    var st = String((primary && primary.state) || "unknown")
    return (st === "running" || st === "sleeping" || st === "stopped") ? st : "unknown"
  }

  function indicatorText() {
    if (!primary) return ""
    var dot = (primaryState === "running" || primaryState === "stopped") ? "●" : "○"
    if (root.bar && root.bar.vertical) return dot
    var label = stateReader.sessionLabel(primary)
    if (stateReader.sessionCount > 1) label += " +" + (stateReader.sessionCount - 1)
    return dot + " " + label
  }

  function tooltipText() {
    if (!primary) return "Pitwall"
    var s = String(primary.summary || stateReader.sessionLabel(primary))
    if (stale) s += " (stale data)"
    return s
  }

  // Focus action, fully native: match the live compositor toplevel and call
  // Toplevel.activate() — the same mechanism first-party widgets use
  // (ActiveWindow, Tray). No subprocess, no shell, no command construction.
  //
  // Matching: candidates share the recorded app-id (class/initialClass);
  // a unique candidate wins, else an exact title match wins, else refuse
  // (window retitled/closed or ambiguous — focusing the wrong terminal
  // would be worse than doing nothing). The address format check guards
  // stale or malformed records before we even look.
  function focusSession(s) {
    var w = (s && s.window) || null
    if (!w) {
      console.warn("pitwall", "Focus requested with no window")
      return
    }
    var addr = String(w.address || "")
    if (!/^0x[0-9a-fA-F]+$/.test(addr)) {
      console.warn("pitwall", "Refusing to focus invalid window address", addr)
      return
    }
    var wanted = []
    if (w.class) wanted.push(String(w.class))
    if (w.initial_class) wanted.push(String(w.initial_class))
    // ToplevelManager.toplevels is a model object; live windows live in
    // `.values` (verified against real-world quickshell usage).
    var model = ToplevelManager.toplevels
    var list = (model && model.values) || []
    var candidates = []
    for (var i = 0; i < list.length; i++) {
      var t = list[i]
      if (!t) continue
      if (wanted.indexOf(String(t.appId || "")) >= 0) candidates.push(t)
    }
    var target = null
    if (candidates.length === 1) {
      target = candidates[0]
    } else if (candidates.length > 1) {
      var title = String(w.title || "")
      for (var j = 0; j < candidates.length; j++) {
        if (title !== "" && String(candidates[j].title || "") === title) {
          target = candidates[j]
          break
        }
      }
    }
    if (!target) {
      console.warn("pitwall", "Focus target gone or ambiguous", addr)
      return
    }
    target.activate()
    root.close()
  }

  function elapsedFor(s) {
    if (!s || !s.last_activity) return ""
    return stateReader.ageText(s.last_activity.epoch, nowMs)
  }

  // Level-2 Resume for vanished sessions: fixed-form `pitwall resume`
  // through Quickshell.Process (exit-visible, no shell). The session id is
  // re-validated here even though it came from our own state file — the
  // file is a trust boundary (user-writable). Failures warn; panel stable.
  function resumeCheckpoint(sessionId) {
    var sid = String(sessionId || "")
    if (!/^sess_[0-9a-f]{16}$/.test(sid)) {
      console.warn("pitwall", "Refusing resume with invalid session id", sid)
      return
    }
    resumeProc.sessionId = sid
    resumeProc.running = true
  }

  function elapsedForCheckpoint(c) {
    if (!c) return ""
    return stateReader.ageText(c.created_at, nowMs)
  }

  Process {
    id: resumeProc
    property string sessionId: ""
    command: ["pitwall", "resume", "--session-id", sessionId]
    stdout: StdioCollector {}
    stderr: StdioCollector {}
    onExited: function(exitCode) {
      if (exitCode !== 0) {
        console.warn("pitwall", "resume exited", exitCode, "for", resumeProc.sessionId)
      }
    }
  }

  onOpenedChanged: if (opened) {
    nowMs = Date.now()
    stateReader.refresh()
    Qt.callLater(function() { keyCatcher.forceActiveFocus() })
  }

  StateReader {
    id: stateReader
    statePath: String(root.setting("statePath", ""))
    staleAfterSec: Number(root.setting("staleAfterSec", 900)) || 900
  }

  // Elapsed text stays truthful while open; the timer lives only while open.
  Timer {
    interval: 30000
    running: root.opened
    repeat: true
    onTriggered: root.nowMs = Date.now()
  }

  Item {
    id: buttonHolder
    anchors.fill: parent
    implicitWidth: button.implicitWidth
    implicitHeight: button.implicitHeight

    WidgetButton {
      id: button
      anchors.fill: parent
      bar: root.bar
      text: root.indicatorText()
      tooltipText: root.tooltipText()
      active: root.needsAttention
      fontSize: Style.font.body
      onPressed: function(buttonCode) {
        if (buttonCode === Qt.RightButton) {
          stateReader.refresh()
          root.nowMs = Date.now()
        } else {
          root.toggle()
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
      onActivateRequested: root.focusSession(root.primary)

      Column {
        id: column
        width: parent.width
        spacing: Style.space(10)

        // Pitwall identity mark: original pixel checkered flag (assets/)
        // plus caption wordmark. Fixed 14px, no smoothing (pixel geometry
        // must stay crisp). If the asset ever fails to load, the wordmark
        // alone carries the identity — no second mark, no emoji.
        Row {
          width: parent.width
          spacing: Style.space(6)

          Image {
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
            text: "PITWALL"
            color: Qt.darker(Color.foreground, 1.4)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
            font.bold: true
            font.letterSpacing: 1
            renderType: Text.NativeRendering
          }
        }

        SessionHero {
          width: parent.width
          session: root.primary
          elapsed: root.elapsedFor(root.primary)
          stale: root.stale
          onFocusRequested: root.focusSession(root.primary)
        }

        PanelSeparator {
          width: parent.width
        }

        PanelSectionHeader {
          width: parent.width
          text: "SESSION"
        }

        Text {
          width: parent.width
          textFormat: Text.PlainText
          text: root.sessionDetailText(root.primary)
          color: Color.foreground
          font.family: Style.font.family
          font.pixelSize: Style.font.bodySmall
          wrapMode: Text.Wrap
          renderType: Text.NativeRendering
        }

        PanelSeparator {
          width: parent.width
        }

        PanelSectionHeader {
          width: parent.width
          text: "ACTIVITY"
        }

        ActivityStrip {
          sessions: stateReader.sessions
          nowMs: root.nowMs
        }

        Text {
          width: parent.width
          textFormat: Text.PlainText
          text: root.activityCaption()
          color: Qt.darker(Color.foreground, 1.4)
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          renderType: Text.NativeRendering
        }

        Repeater {
          model: Math.min(root.others.length, 4)
          delegate: SessionRow {
            width: column.width
            session: root.others[index]
            elapsed: root.elapsedFor(root.others[index])
            stale: root.stale
            onFocusRequested: root.focusSession(root.others[index])
          }
        }

        Text {
          visible: root.others.length > 4
          width: parent.width
          textFormat: Text.PlainText
          text: "+" + (root.others.length - 4) + " more"
          color: Qt.darker(Color.foreground, 1.4)
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          renderType: Text.NativeRendering
        }

        PanelSeparator {
          visible: root.hasResumable
          width: parent.width
        }

        PanelSectionHeader {
          visible: root.hasResumable
          width: parent.width
          text: "RESUME"
        }

        Repeater {
          model: Math.min(stateReader.resumable.length, 5)
          delegate: ResumableRow {
            width: column.width
            checkpoint: stateReader.resumable[index]
            elapsed: root.elapsedForCheckpoint(stateReader.resumable[index])
            onResumeRequested: function(sid) { root.resumeCheckpoint(sid) }
          }
        }

        Text {
          visible: stateReader.resumable.length > 5
          width: parent.width
          textFormat: Text.PlainText
          text: "+" + (stateReader.resumable.length - 5) + " more checkpoints"
          color: Qt.darker(Color.foreground, 1.4)
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          renderType: Text.NativeRendering
        }
      }
    }
  }

  function sessionDetailText(s) {
    if (!s) return "No session."
    var parts = []
    var a = s.agent || {}
    parts.push("agent  " + String(a.kind || "unknown") + " · " + String(a.confidence || "unknown"))
    var w = s.window || null
    var win = w ? String(w.class || "?") : "no window"
    var ws = (w && w.workspace) ? " · workspace " + w.workspace : ""
    parts.push("window  " + win + ws + " · " + Number(s.process_count || 0) + " procs")
    return parts.join("\n")
  }

  function activityCaption() {
    if (!primary || !primary.last_activity) return "latest start: unknown"
    return "latest start " + elapsedFor(primary) + " · " + String(primary.last_activity.kind || "")
  }
}
