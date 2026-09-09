import QtQuick
import qs.Commons

// Honest micro-strip: one segment per session (max 7), segment height
// encodes recency (taller = more recent activity, floor 15%), color
// encodes session state. No fake time series — every pixel comes from
// last_activity epochs already in state.json. Static rectangles only.
Item {
  id: root

  property var sessions: []
  property double nowMs: Date.now()
  property int maxSegments: 7
  property int segmentWidth: 10
  property int maxHeight: 26

  readonly property int shown: Math.min(sessions.length, maxSegments)

  function fracFor(s) {
    var la = (s.last_activity && isFinite(Number(s.last_activity.epoch))) ? Number(s.last_activity.epoch) : -1
    if (la <= 0) return 0.15
    var ageSec = Math.max(0, nowMs / 1000 - la)
    return Math.max(0.15, Math.min(1, 1 - ageSec / 86400))
  }

  function colorFor(s) {
    var st = String(s.state || "unknown")
    if (st === "running") return Color.accent
    if (st === "stopped") return Color.urgent
    return Color.muted
  }

  implicitWidth: shown * segmentWidth + Math.max(0, shown - 1) * Style.space(3)
  implicitHeight: maxHeight

  Row {
    anchors.bottom: parent.bottom
    spacing: Style.space(3)
    Repeater {
      model: root.shown
      Rectangle {
        property var sess: root.sessions[index] || {}
        width: root.segmentWidth
        height: Math.max(3, Math.round(root.maxHeight * root.fracFor(sess)))
        anchors.bottom: parent.bottom
        color: root.colorFor(sess)
        opacity: 0.9
      }
    }
  }
}
