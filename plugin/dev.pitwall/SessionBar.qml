import QtQuick
import qs.Commons
import qs.Ui

// One workspace session as a timeline bar: label + duration-proportional
// track with observed-history segments. Pure presentation over state.json
// fields (age_secs, history, state, role): no inference, no polling.
// Click selects (parent pins); hover previews. All motion is state-bound.
Item {
  id: root

  // Session object (live) or resumable checkpoint entry.
  property var entry: null
  property bool resumable: false
  // 0..1 duration fraction (parent scales logarithmically, bounded).
  property real frac: 0.15
  property bool selected: false
  property bool dimmed: false

  signal clicked()
  signal hovered(bool isHovered)

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
    return confGlyph + " " + kind + " · " + projectName + branchText
  }
  // History segments: R running, S present-but-quiet, U unknown.
  // Only samples the record contains; never filler.
  readonly property string history: String((entry && entry.history) || "")

  implicitWidth: parent ? parent.width : 200
  implicitHeight: Style.space(14) + Style.space(22)

  opacity: dimmed ? 0.55 : 1.0
  Behavior on opacity {
    NumberAnimation { duration: 160; easing.type: Easing.OutCubic }
  }

  Text {
    id: barLabel
    textFormat: Text.PlainText
    anchors.left: parent.left
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
            var ch = root.history[index]
            return ch === "R" ? 0.95 : 0.55
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
    onEntered: root.hovered(true)
    onExited: root.hovered(false)
    onClicked: root.clicked()
  }
}
