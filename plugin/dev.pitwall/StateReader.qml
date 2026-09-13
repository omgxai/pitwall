import QtQuick
import Quickshell
import Quickshell.Io

// Reads the M2 state.json artifact event-driven (FileView watchChanges).
// The panel never learns how the numbers were made: malformed or missing
// input yields record=null, never a crash. No polling, no SQLite, no /proc.
Item {
  id: root
  visible: false

  property string statePath: ""
  property int staleAfterSec: 900
  property var record: null
  property bool fileMissing: false

  // Effective path: explicit override, else XDG, else ~/.local/share.
  // Evaluated once; FileView binds to the result.
  readonly property string effectivePath: {
    if (statePath !== "") return statePath
    var xdg = Quickshell.env("XDG_DATA_HOME")
    var base = (xdg && xdg.length > 0) ? xdg : Quickshell.env("HOME") + "/.local/share"
    return base + "/pitwall/state.json"
  }

  readonly property var sessions: (record && Array.isArray(record.sessions)) ? record.sessions : []
  readonly property int sessionCount: sessions.length
  // v2 additive key; absent on v1 (panel tolerates both).
  readonly property var resumable: (record && Array.isArray(record.resumable)) ? record.resumable : []
  // v4 additive keys (M5g inbox): unread notification rows + badge count.
  // Malformed shapes degrade to empty/zero, never a crash.
  readonly property var notifications: (record && Array.isArray(record.notifications)) ? record.notifications : []
  readonly property int unreadCount: {
    var n = record ? Number(record.unread_count) : 0
    return isFinite(n) && n > 0 ? Math.round(n) : 0
  }
  // v3 additive key (M5d contract; M5f renders it). Null/absent means
  // no summary yet; malformed shapes degrade to null, never a crash.
  readonly property var summary: {
    var s = record ? record.summary : null
    if (!s || typeof s !== "object" || Array.isArray(s)) return null
    var status = String(s.status || "")
    if (status !== "ready" && status !== "error" && status !== "unavailable" && status !== "stale") return null
    return {
      text: String(s.text || ""),
      model: s.model ? String(s.model) : "",
      created_at: Number(s.created_at) || 0,
      input_hash: String(s.input_hash || ""),
      status: status,
      message: String(s.message || "")
    }
  }
  readonly property double collectedAt: record ? Number(record.collected_at || 0) : 0

  // Primary session = most recently active. Stable choice: ties keep array
  // order (snapshot is id-sorted), so the hero doesn't flicker.
  readonly property var primarySession: {
    var best = null
    var bestEpoch = -1
    for (var i = 0; i < sessions.length; i++) {
      var s = sessions[i] || {}
      var la = (s.last_activity && isFinite(Number(s.last_activity.epoch))) ? Number(s.last_activity.epoch) : -1
      if (la > bestEpoch) {
        bestEpoch = la
        best = s
      }
    }
    return best || (sessions.length > 0 ? sessions[0] : null)
  }

  // Everything except the primary, in snapshot order.
  readonly property var otherSessions: {
    var out = []
    var primary = primarySession
    for (var i = 0; i < sessions.length; i++) {
      if (sessions[i] !== primary) out.push(sessions[i])
    }
    return out
  }

  function isStale(nowMs) {
    if (!record || collectedAt <= 0) return false
    return (nowMs / 1000 - collectedAt) > staleAfterSec
  }

  function ageText(epoch, nowMs) {
    var delta = Math.max(0, Math.round(nowMs / 1000 - Number(epoch || 0)))
    if (!(isFinite(delta) && Number(epoch) > 0)) return "unknown age"
    if (delta < 10) return "just now"
    if (delta < 60) return delta + "s ago"
    var m = Math.floor(delta / 60)
    if (m < 60) return m + "m ago"
    var h = Math.floor(m / 60)
    if (h < 24) return h + "h " + (m % 60) + "m ago"
    return Math.floor(h / 24) + "d ago"
  }

  function sessionLabel(s) {
    s = s || {}
    var p = s.project || null
    var proj = p ? (p.name || "session") : "session"
    var branch = (p && p.branch) ? ":" + p.branch : ""
    return proj + branch
  }

  function stateOf(s) {
    var st = String((s && s.state) || "unknown")
    return (st === "running" || st === "sleeping" || st === "stopped") ? st : "unknown"
  }

  FileView {
    id: stateFile
    path: root.effectivePath
    watchChanges: true
    printErrors: false
    onFileChanged: reload()
    onLoaded: root.parse(text())
    onLoadFailed: {
      root.record = null
      root.fileMissing = true
    }
  }

  function refresh() {
    stateFile.reload()
  }

  function parse(content) {
    try {
      var parsed = JSON.parse(String(content || ""))
      var version = Number(parsed && parsed.state_version)
      var ok = parsed && typeof parsed === "object"
        && (version === 1 || version === 2 || version === 3)
        && Array.isArray(parsed.sessions)
      if (!ok) throw new Error("unsupported state shape")
      root.record = parsed
      root.fileMissing = false
    } catch (e) {
      console.warn("pitwall", "Ignoring malformed state file", root.effectivePath, e)
      root.record = null
    }
  }
}
