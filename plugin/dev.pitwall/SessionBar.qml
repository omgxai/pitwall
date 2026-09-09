import QtQuick
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
  property string detailText: ""
  property bool canFocus: false
  property bool canStop: false
  property bool canClose: false
  property bool canResume: false
  property string resumeTooltip: "Resume"

  signal clicked()
  signal hovered(bool isHovered)
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
    return confGlyph + " " + kind + " · " + projectName + branchText
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
      }
    }
  }
}
