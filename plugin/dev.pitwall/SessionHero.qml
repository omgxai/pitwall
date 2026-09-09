import QtQuick
import qs.Commons
import qs.Ui

// Hero for the primary session: project identity, git + elapsed meta,
// state label, and the Focus action. Trailing control is keyboard-focusable.
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
    if (root.stale) return "stale"
    if (stateKey === "running") return "working"
    if (stateKey === "sleeping") return "idle"
    if (stateKey === "stopped") return "waiting"
    return "unknown"
  }

  implicitWidth: parent ? parent.width : 200
  implicitHeight: hero.implicitHeight

  PanelHero {
    id: hero
    width: parent.width
    title: project ? (project.name || "session") : "session"
    meta: heroMeta()
    detail: root.stateLabel
    trailingControl: focusControl

    iconComponent: Component {
      StateDot {
        stateKey: root.stateKey
        pulsing: root.stateKey === "running" && !root.stale
        fontSize: Style.font.display
      }
    }
  }

  Component {
    id: focusControl
    PanelActionButton {
      // md-crosshairs U+F034E — coverage verified in JetBrainsMono Nerd Font.
      iconText: String.fromCodePoint(0xF034E)
      tooltipText: "Resume: focus terminal"
      focusable: true
      onClicked: root.focusRequested()
    }
  }

  function heroMeta() {
    var parts = []
    if (project && project.branch) parts.push(project.branch)
    if (project) {
      if (project.git_clean === true) parts.push("clean")
      else if (project.git_clean === false) parts.push("dirty")
    }
    if (root.elapsed !== "") parts.push(root.elapsed)
    if (root.stale) parts.push("stale data")
    return parts.join(" • ")
  }
}
