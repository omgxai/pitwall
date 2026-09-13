import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

// One workspace session as a timeline bar with an attached detail card.
// Interaction contract (flicker-free by construction):
// - hover highlights the bar ONLY (never opens/switches the card);
// - click pins: the card renders directly beneath its own bar, geometry
//   stable, so bar -> card -> button never crosses another bar;
// - the card lives and dies with the pinned selection alone.
// Clicks are explicit; hover can never steal a pinned selection, and
// touch (no hover) gets identical behavior through click-to-pin.
Column {
  id: root

  // Session object (live) or resumable checkpoint entry.
  property var entry: null
  property bool resumable: false
  // 0..1 duration fraction (parent scales logarithmically, bounded).
  property real frac: 0.15
  property bool selected: false
  property bool dimmed: false
  property bool hovered: false
  // Attached card (parent-owned content + actions).
  property bool showCard: false
  // Unread inbox rows for this entry (parent-filtered). Empty = none.
  property var notifs: []
  readonly property bool hasUnread: notifs.length > 0
  property string detailText: ""
  property bool canFocus: false
  property bool canStop: false
  property bool canClose: false
  property bool canResume: false
  property string resumeTooltip: "Resume"
  // Assign flow (live sessions only): inline mini-form, fixed argv,
  // explicit Assign click. Display labels only; validation enforced
  // again in Rust before anything spawns.
  property bool canAssign: false
  property bool assigning: false
  property string assignRole: "UI Auditor"
  property string assignResult: ""
  property bool assignRunning: false
  readonly property string pitwallBinary: {
    var home = String(Quickshell.env("HOME") || "")
    return home !== "" ? home + "/.local/bin/pitwall" : "pitwall"
  }

  signal clicked()
  signal hovered(bool isHovered)
  signal notifClicked(int notifId)

  // Assignment execution: fixed argv, validated session id, capped
  // prompt. No shell, no interpolation. Result is display text only.
  function submitAssign(promptText) {
    var sid = String((entry && entry.id) || "")
    if (!/^sess_[0-9a-f]{16}$/.test(sid)) {
      assignResult = "Refusing: invalid session id."
      return
    }
    var prompt = String(promptText || "").trim()
    if (prompt === "") {
      assignResult = "Describe the task first."
      return
    }
    assignRunning = true
    assignResult = ""
    assignProc.sessionId = sid
    assignProc.command = [
      root.pitwallBinary, "assign",
      "--session-id", sid,
      "--role", String(assignRole),
      "--prompt", prompt
    ]
    assignProc.running = true
  }

  Process {
    id: assignProc
    property string sessionId: ""
    stdout: StdioCollector { id: assignOut }
    stderr: StdioCollector {}
    onExited: function(code) {
      root.assignRunning = false
      if (code === 0) {
        var t = String(assignOut.text || "").trim().replace(/\s+/g, " ")
        root.assignResult = t === "" ? "Done (no output)." : ("Result: " + t.slice(0, 280))
      } else {
        root.assignResult = "Assignment failed — see shell log."
        console.warn("pitwall", "assign exited", code, "for", assignProc.sessionId)
      }
    }
  }
  signal focusRequested()
  signal stopRequested()
  signal closeRequested()
  signal resumeRequested()

  readonly property string stateKey: {
    var st = String((entry && entry.state) || "unknown")
    return (st === "running" || st === "sleeping" || st === "stopped") ? st : "unknown"
  }
  readonly property string agentKind: String((entry && entry.agent_kind) || (entry && entry.agent && entry.agent.kind) || "unknown")
  readonly property string agentConf: String((entry && entry.agent_confidence) || (entry && entry.agent && entry.agent.confidence) || "unknown")
  readonly property string projectName: {
    if (!entry) return "session"
    if (entry.project_name) return String(entry.project_name)
    if (entry.project && entry.project.name) return String(entry.project.name)
    return "session"
  }
  readonly property string branchText: {
    var b = (entry && entry.branch) || (entry && entry.project && entry.project.branch) || ""
    return b ? ":" + b : ""
  }
  // Confidence glyph (verified codepoints only): filled = strong,
  // hollow = medium, ? = uncertain. Always paired with text/tooltip.
  readonly property string confGlyph: {
    if (agentConf === "high") return "●"
    if (agentConf === "medium") return "○"
    return "?"
  }
  readonly property color confColor: {
    // Resumable rows are past, never live: always muted, even when the
    // checkpoint captured a running state. Muted, not hidden.
    if (root.resumable) return Color.muted
    if (stateKey === "running") return Color.accent
    if (stateKey === "stopped") return Color.urgent
    return Color.muted
  }
  readonly property string labelText: {
    var kind = agentKind
    if (kind === "unknown") {
      var role = String((entry && entry.role) || "unknown")
      if (role === "app" && entry && entry.window && entry.window.class) {
        kind = String(entry.window.class)
      } else {
        kind = "shell"
      }
    }
    var label = confGlyph + " " + kind + " · " + projectName + branchText
    // Tiny process count (tooltip/detail carry the words, not the rail).
    var procs = (entry && entry.process_count !== undefined) ? Number(entry.process_count) : -1
    if (procs >= 0) label += "  · " + procs
    // Expand affordance: the card (not a new list) opens beneath this bar.
    label += selected ? "  ▴" : "  ▾"
    return label
  }
  // History segments: R running, S present-but-quiet, U unknown.
  // Only samples the record contains; never filler.
  readonly property string history: String((entry && entry.history) || "")

  width: parent ? parent.width : 200
  spacing: Style.space(6)

  opacity: (dimmed && !hovered && !selected) ? 0.55 : 1.0
  Behavior on opacity {
    NumberAnimation { duration: 160; easing.type: Easing.OutCubic }
  }

  Item {
    id: barBlock
    width: parent.width
    height: Style.space(14) + Style.space(22)

    Rectangle {
      id: unreadDot
      visible: root.hasUnread
      width: Style.space(6)
      height: Style.space(6)
      radius: width / 2
      anchors.left: parent.left
      anchors.top: parent.top
      anchors.topMargin: Style.space(4)
      color: Color.accent
    }

    Text {
      id: barLabel
      textFormat: Text.PlainText
      anchors.left: unreadDot.visible ? unreadDot.right : parent.left
      anchors.leftMargin: unreadDot.visible ? Style.space(4) : 0
      anchors.right: parent.right
      anchors.top: parent.top
      elide: Text.ElideRight
      text: root.labelText
      color: root.selected ? Color.foreground : Qt.darker(Color.foreground, 1.15)
      font.family: Style.font.family
      font.pixelSize: Style.font.bodySmall
      renderType: Text.NativeRendering
    }

    // Track: full-width dim base + duration-proportional fill.
    Rectangle {
      id: track
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: barLabel.bottom
      anchors.topMargin: Style.space(4)
      height: Style.space(12)
      color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.08)
    }

    Item {
      id: fillClip
      anchors.left: track.left
      anchors.top: track.top
      width: Math.max(Style.space(16), Math.round(track.width * root.frac))
      height: track.height
      clip: true

      Behavior on width {
        NumberAnimation { duration: 180; easing.type: Easing.OutCubic }
      }

      Row {
        anchors.fill: parent
        spacing: 0
        Repeater {
          model: root.history.length > 0 ? root.history.length : 1
          Rectangle {
            width: Math.max(2, Math.round(fillClip.width / (root.history.length > 0 ? root.history.length : 1)))
            height: fillClip.height
            color: {
              if (root.history.length === 0) return root.confColor
              var ch = root.history[index]
              if (ch === "R") return Color.accent
              if (ch === "S") return Qt.darker(Color.foreground, 1.6)
              return Qt.darker(Color.muted, 1.3)
            }
            opacity: {
              if (root.history.length === 0) return root.stateKey === "running" ? 0.95 : 0.55
              var ch2 = root.history[index]
              return ch2 === "R" ? 0.95 : 0.55
            }
          }
        }
      }

      // Live pulse: opacity breath on the running fill only. Stops dead
      // otherwise (binding includes running state).
      SequentialAnimation on opacity {
        running: root.stateKey === "running" && root.visible && !root.resumable
        loops: Animation.Infinite
        NumberAnimation { from: 1.0; to: 0.7; duration: 700; easing.type: Easing.OutCubic }
        NumberAnimation { from: 0.7; to: 1.0; duration: 700; easing.type: Easing.OutCubic }
      }
    }

    // Selected outline (border only, no fill change).
    Rectangle {
      anchors.fill: track
      color: "transparent"
      border.width: root.selected ? 1 : 0
      border.color: Color.accent
    }

    MouseArea {
      anchors.fill: parent
      hoverEnabled: true
      acceptedButtons: Qt.LeftButton
      cursorShape: Qt.PointingHandCursor
      onEntered: {
        root.hovered = true
        root.hovered(true)
      }
      onExited: {
        root.hovered = false
        root.hovered(false)
      }
      onClicked: root.clicked()
    }
  }

  // Attached detail card: part of this delegate, directly beneath its own
  // bar. Stable geometry while pinned; pointer travel bar -> card ->
  // button never crosses another bar. No hover handlers here on purpose:
  // nothing inside the card can change or dismiss the selection.
  Rectangle {
    visible: root.showCard
    width: parent.width
    height: cardColumn.implicitHeight + Style.space(16)
    color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.05)
    border.width: 1
    border.color: Qt.rgba(Color.foreground.r, Color.foreground.g, Color.foreground.b, 0.22)

    Behavior on opacity {
      NumberAnimation { duration: 140; easing.type: Easing.OutCubic }
    }

    Column {
      id: cardColumn
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.margins: Style.space(8)
      spacing: Style.space(4)

      Text {
        width: parent.width
        textFormat: Text.PlainText
        wrapMode: Text.Wrap
        maximumLineCount: 5
        elide: Text.ElideRight
        text: root.detailText
        color: Color.foreground
        font.family: Style.font.family
        font.pixelSize: Style.font.bodySmall
        renderType: Text.NativeRendering
      }

      // Inbox rows for this entry only. Click marks exactly that row
      // read (parent runs the fixed CLI + refreshes). Listing never
      // marks; expanding never marks.
      Repeater {
        model: Math.min(root.notifs.length, 3)
        delegate: Item {
          width: cardColumn.width
          height: notifText.implicitHeight + Style.space(2)

          Text {
            id: notifText
            textFormat: Text.PlainText
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            elide: Text.ElideRight
            maximumLineCount: 2
            wrapMode: Text.Wrap
            text: {
              var nb = root.notifs[index] || {}
              var mark = String(nb.severity) === "attention" ? "! "
                : String(nb.severity) === "completion" ? "\u2713 " : "\u2022 "
              return mark + String(nb.detail || "")
            }
            color: {
              var sev = String((root.notifs[index] || {}).severity || "")
              if (sev === "attention") return Color.urgent
              return Qt.darker(Color.foreground, 1.15)
            }
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
            renderType: Text.NativeRendering
          }

          MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.LeftButton
            cursorShape: Qt.PointingHandCursor
            onClicked: root.notifClicked(Number((root.notifs[index] || {}).id || 0))
          }
        }
      }

      Text {
        visible: root.notifs.length > 3
        width: parent.width
        textFormat: Text.PlainText
        text: "+" + (root.notifs.length - 3) + " more"
        color: Qt.darker(Color.foreground, 1.4)
        font.family: Style.font.family
        font.pixelSize: Style.font.caption
        renderType: Text.NativeRendering
      }

      // Assign mini-form: prompt + role + explicit Assign. Inline in
      // the pinned card (no floating popup, stable geometry).
      Column {
        visible: root.assigning
        width: parent.width
        spacing: Style.space(6)

        TextField {
          id: assignPrompt
          width: parent.width
          placeholderText: "Task for this session…"
          font.pixelSize: Style.font.bodySmall
        }

        Dropdown {
          id: assignRole
          width: parent.width
          label: "Role"
          value: root.assignRole
          options: ["UI Auditor", "Systems", "Reviewer", "Researcher", "QA"]
          onChanged: function(v) { root.assignRole = v }
        }

        Row {
          spacing: Style.space(8)

          Button {
            text: root.assignRunning ? "Working…" : "Assign"
            enabled: !root.assignRunning && assignPrompt.text.trim() !== ""
            onClicked: root.submitAssign(assignPrompt.text)
          }

          Button {
            text: "Cancel"
            enabled: !root.assignRunning
            onClicked: {
              root.assigning = false
              root.assignResult = ""
            }
          }
        }

        Text {
          visible: root.assignResult !== ""
          width: parent.width
          textFormat: Text.PlainText
          wrapMode: Text.Wrap
          maximumLineCount: 4
          elide: Text.ElideRight
          text: root.assignResult
          color: Qt.darker(Color.foreground, 1.4)
          font.family: Style.font.family
          font.pixelSize: Style.font.caption
          renderType: Text.NativeRendering
        }
      }

      Row {
        spacing: Style.space(4)

        PanelActionButton {
          visible: root.canFocus
          iconText: String.fromCodePoint(0xF034E)
          tooltipText: "Focus terminal"
          focusable: true
          onClicked: root.focusRequested()
        }

        PanelActionButton {
          visible: root.canStop
          iconText: "■"
          tooltipText: "Stop session processes (SIGTERM)"
          focusable: true
          onClicked: root.stopRequested()
        }

        PanelActionButton {
          visible: root.canClose
          iconText: "✕"
          tooltipText: "Close window"
          focusable: true
          onClicked: root.closeRequested()
        }

        PanelActionButton {
          visible: root.canResume
          iconText: "▶"
          tooltipText: root.resumeTooltip
          focusable: true
          onClicked: root.resumeRequested()
        }

        PanelActionButton {
          visible: root.canAssign
          iconText: "+"
          tooltipText: "Assign a task to this session"
          focusable: true
          onClicked: {
            root.assigning = !root.assigning
            root.assignResult = ""
          }
        }
      }
    }
  }
}
