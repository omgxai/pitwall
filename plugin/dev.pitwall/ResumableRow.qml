import QtQuick
import qs.Commons
import qs.Ui

// One resumable (vanished-session) checkpoint row: muted hollow dot (past,
// never pulsing), `project:branch — agent` label, checkpoint age, and a
// Resume action that opens a terminal at the recorded directory via
// `pitwall resume`. No action is offered without the explicit click;
// refusal (deleted dir, unknown id) surfaces in the shell log and the
// panel stays stable.
Item {
  id: root

  property var checkpoint: null
  property string elapsed: ""

  signal resumeRequested(string sessionId)

  readonly property string projectText: {
    var c = checkpoint || {}
    var name = String(c.project_name || c.project_dir || "project")
    var branch = c.branch ? ":" + c.branch : ""
    return name + branch
  }
  readonly property string agentText: String((checkpoint && checkpoint.agent_kind) || "unknown")

  implicitWidth: parent ? parent.width : 200
  implicitHeight: Math.max(Style.spacing.popupRowHeight, noteText.visible ? noteText.y + noteText.implicitHeight : Style.spacing.popupRowHeight)

  StateDot {
    id: dot
    anchors.left: parent.left
    anchors.top: parent.top
    anchors.topMargin: Style.space(6)
    stateKey: "unknown"
    pulsing: false
    fontSize: Style.font.body
  }

  Text {
    id: labelText
    textFormat: Text.PlainText
    anchors.left: dot.right
    anchors.leftMargin: Style.space(8)
    anchors.right: elapsedText.left
    anchors.rightMargin: Style.space(8)
    anchors.top: parent.top
    anchors.topMargin: Style.space(6)
    elide: Text.ElideRight
    text: root.projectText + " — " + root.agentText
    color: Color.foreground
    font.family: Style.font.family
    font.pixelSize: Style.font.body
    renderType: Text.NativeRendering
  }

  Text {
    id: elapsedText
    textFormat: Text.PlainText
    anchors.right: resumeButton.left
    anchors.rightMargin: Style.space(4)
    anchors.top: parent.top
    anchors.topMargin: Style.space(7)
    text: root.elapsed !== "" ? root.elapsed : "checkpoint"
    color: Qt.darker(Color.foreground, 1.4)
    font.family: Style.font.family
    font.pixelSize: Style.font.bodySmall
    renderType: Text.NativeRendering
  }

  PanelActionButton {
    id: resumeButton
    anchors.right: parent.right
    anchors.top: parent.top
    anchors.topMargin: Style.space(3)
    iconText: String.fromCodePoint(0xF034E)
    tooltipText: root.resumeTooltip()
    focusable: true
    onClicked: {
      var sid = String((root.checkpoint && root.checkpoint.session_id) || "")
      root.resumeRequested(sid)
    }
  }

  Text {
    id: noteText
    visible: String((root.checkpoint && root.checkpoint.note) || "") !== ""
    textFormat: Text.PlainText
    anchors.left: labelText.left
    anchors.right: parent.right
    anchors.top: labelText.bottom
    text: String((root.checkpoint && root.checkpoint.note) || "")
    color: Qt.darker(Color.foreground, 1.4)
    font.family: Style.font.family
    font.pixelSize: Style.font.caption
    elide: Text.ElideRight
    maximumLineCount: 1
    renderType: Text.NativeRendering
  }

  function resumeTooltip() {
    var c = checkpoint || {}
    var dir = String(c.project_dir || "")
    if (dir === "") return "Resume unavailable"
    return "Resume: open terminal at " + dir
  }
}
