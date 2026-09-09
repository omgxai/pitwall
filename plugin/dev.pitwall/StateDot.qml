import QtQuick
import qs.Commons

// State dot: filled for active attention states, hollow for rest.
// Shape differs by state (never color alone); labels in the rows carry
// the state name in text. Pulse runs ONLY while `pulsing` is true and
// stops completely otherwise (opacity snaps back to 1).
Text {
  id: root

  property string stateKey: "unknown" // running | sleeping | stopped | unknown
  property bool pulsing: false
  property real fontSize: Style.font.body

  readonly property bool filled: stateKey === "running" || stateKey === "stopped"
  readonly property color dotColor: {
    if (stateKey === "running") return Color.accent
    if (stateKey === "stopped") return Color.urgent
    return Color.muted
  }

  textFormat: Text.PlainText
  text: filled ? "●" : "○"
  color: dotColor
  font.family: Style.font.family
  font.pixelSize: Math.max(1, Math.round(fontSize))
  renderType: Text.NativeRendering

  SequentialAnimation on opacity {
    running: root.pulsing && root.visible
    loops: Animation.Infinite
    NumberAnimation { from: 1.0; to: 0.45; duration: 600; easing.type: Easing.OutCubic }
    NumberAnimation { from: 0.45; to: 1.0; duration: 600; easing.type: Easing.OutCubic }
  }
  onPulsingChanged: if (!pulsing) opacity = 1.0
}
