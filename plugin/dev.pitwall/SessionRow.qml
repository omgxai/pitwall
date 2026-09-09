import QtQuick
import qs.Commons
import qs.Ui

// One compact 28px session row: state dot, `project:branch — agent` label,
// elapsed caption, and a Focus action. Row owns the keyboard cursor via
// CursorSurface conventions used across first-party panels.
Item {
  id: root

  property var session: null
  property string elapsed: ""
  property bool stale: false

  signal focusRequested()

  readonly property var project: (session && session.project) || null
  readonly property string stateKey: {
    var st = String((session && session.state) || "unknown")
    return (st === "running" || st === "sleeping" || st === "stopped") ? st : "unknown"
  }
  readonly property string stateLabel: {
    if (stateKey === "running") return "working"
    if (stateKey === "sleeping") return "idle"
    if (stateKey === "stopped") return "waiting"
    return "unknown"
  }
  readonly property string agentText: {
    var a = (session && session.agent) || {}
    var kind = String(a.kind || "unknown")
    var conf = String(a.confidence || "unknown")
    return conf === "unknown" ? kind : kind + " · " + conf
  }

  implicitWidth: parent ? parent.width : 200
  implicitHeight: Style.spacing.popupRowHeight

  StateDot {
    id: dot
    anchors.left: parent.left
    anchors.verticalCenter: parent.verticalCenter
    stateKey: root.stateKey
    pulsing: root.stateKey === "running" && !root.stale
    fontSize: Style.font.body
  }

  Text {
    id: labelText
    textFormat: Text.PlainText
    anchors.left: dot.right
    anchors.leftMargin: Style.space(8)
    anchors.right: elapsedText.left
    anchors.rightMargin: Style.space(8)
    anchors.verticalCenter: parent.verticalCenter
    elide: Text.ElideRight
    text: {
      var proj = project ? (project.name || "session") : "session"
      var branch = (project && project.branch) ? ":" + project.branch : ""
      return proj + branch + " — " + root.agentText
    }
    color: Color.foreground
    font.family: Style.font.family
    font.pixelSize: Style.font.body
    renderType: Text.NativeRendering
  }

  Text {
    id: elapsedText
    textFormat: Text.PlainText
    anchors.right: focusButton.left
    anchors.rightMargin: Style.space(4)
    anchors.verticalCenter: parent.verticalCenter
    text: root.elapsed !== "" ? root.elapsed : root.stateLabel
    color: Qt.darker(Color.foreground, 1.4)
    font.family: Style.font.family
    font.pixelSize: Style.font.bodySmall
    renderType: Text.NativeRendering
  }

  PanelActionButton {
    id: focusButton
    anchors.right: parent.right
    anchors.verticalCenter: parent.verticalCenter
    iconText: String.fromCodePoint(0xF034E)
    tooltipText: "Focus terminal"
    focusable: true
    onClicked: root.focusRequested()
  }
}
