import QtQuick
import QtTest

// Qt Quick Test coverage for the AI_Brief_Ticker laws of M8 Part A, plus the
// brief-provenance and chat-label laws
// (spec: .kiro/specs/pitwall-chat-and-brief-ticker, design §4.11, §5.3).
//
// IMPORTANT — MIRRORED FORMULAS, KEEP IN SYNC WITH Widget.qml, StateReader.qml
// AND SessionBar.qml
//
// `Widget.qml` is a Quickshell bar widget: its root is `Panel` and it imports
// `Quickshell`, `Quickshell.Io`, `Quickshell.Wayland`, `qs.Commons` and `qs.Ui`
// (Style/Color tokens). `StateReader.qml` imports `Quickshell` and
// `Quickshell.Io` for `FileView` and `Quickshell.env`; `SessionBar.qml` imports
// `Quickshell`, `Quickshell.Io`, `qs.Commons` and `qs.Ui`. None of those modules
// resolve under a bare `qmltestrunner`, so none of these components can be
// instantiated here and their properties cannot be driven directly.
//
// Therefore the functions in the MIRRORS block below are verbatim copies of the
// single-expression logic in `Widget.qml`, and the pass model reproduces exactly
// what the `NumberAnimation on x` in `Widget.qml` does (linear interpolation of
// `x` from `tickerTrack.x` to `-marqueeText.implicitWidth`, restarted from the
// right edge by `tickerTrack.startPassFromRightEdge()`). Any edit to
// `root.summaryShort()`, `root.tickerDurationMs()`, `root.tickerSpeedPxPerSec`,
// `tickerTrack.startPassFromRightEdge()`, `tickerTrack.onXChanged`,
// `tickerTrack.Component.onCompleted` or the `tickerPass` animation MUST be
// mirrored here or these tests stop describing the shipped widget.
//
// Source of each mirror (as of task 3.3):
//   Widget.qml root:            readonly property real tickerSpeedPxPerSec: 180
//   Widget.qml root:            function tickerDurationMs(fromX, contentW)
//   Widget.qml root:            function summaryShort()
//   Widget.qml tickerViewport:  visible: root.summaryShort() !== ""
//   Widget.qml tickerTrack:     startPassFromRightEdge(), onXChanged,
//                               Component.onCompleted
//   Widget.qml tickerPass:      from / to / duration / Easing.Linear
//
// Added by task 4.3 (Properties 5 and 25):
//   Widget.qml root:            function briefStatusText()
//   Widget.qml root:            function briefStatusAttention()
//   Widget.qml root:            function briefNeedsGenerate()
//   Widget.qml root:            function attentionText()
//   Widget.qml briefCard:       the fixed strings "AI BRIEF" and
//                               "Generate summary", the "! " attention prefix,
//                               and the visible: conditions of the caption row,
//                               ticker line, status row and attention line
//   StateReader.qml:            function chatOf(s)
//   StateReader.qml:            function stateOf(s)
//   SessionBar.qml:             stateKey, stateWord, stateGlyph, and the chat
//                               branch of labelText (task 13.2's label builder)
//
// Added by task 13.4 (Properties 6 and 24):
//   Widget.qml root:            function chatPinnedIsResumable(pinned)
//   Widget.qml root:            function openChat() — the whole argv
//                               construction, the `sess_[0-9a-f]{16}` gate, the
//                               refusal branch (announce + early return, no
//                               runFixed call) and the single runFixed(argv, …)
//                               spawn point
//   Widget.qml root:            readonly property string pitwallBinary (shape
//                               only: an absolute path, stubbed here because
//                               Quickshell.env is unavailable)
//   StateReader.qml:            readonly property var sessions
//   StateReader.qml:            readonly property var resumable
//   StateReader.qml:            readonly property var notifications
//   StateReader.qml:            readonly property int unreadCount
//   StateReader.qml:            readonly property var summary
//   StateReader.qml:            readonly property string pitwallVersion
//   StateReader.qml:            readonly property var chatSessions
//   StateReader.qml:            function sessionLabel(s)
//   StateReader.qml:            function parse(content) — the `ok` acceptance
//                               test only (state_version 1|2|3|4 plus an array
//                               of sessions); JSON.parse itself is the
//                               runtime's, so it is used directly here
//
// QML has no property-testing library, so each test drives a deterministic
// seeded PRNG over generated geometry and text for well over the required 100
// iterations. Seeds are fixed so a failure is reproducible.
//
// EXECUTION STATUS: NOT RUN as of tasks 3.3, 4.3 and 13.4. Gate 1.4 recorded that neither
// `qmltestrunner` nor `qmllint` is installed on the development machine, so
// this file has never been executed and must not be reported as passing. It is
// committed so that a Qt-equipped machine can run it with
// `qmltestrunner -input plugin/dev.pitwall/tests/tst_ticker.qml`. The mirrored
// formulas and every assertion below were checked by transcribing them into a
// throwaway plain-JavaScript harness, including a negative control confirming
// that Property 3 fails against the old `max(700, …)` duration floor; that
// check exercises the logic, not the QML runtime. Property 5 carries its
// negative control inside the test itself (an "inventive" status function that
// the provenance checker must reject). Properties 6 and 24 each carry their own
// in-test negative control for the same reason: Property 6 asserts that all
// three of its branches were actually reached by the generators, and Property 24
// asserts that a well-formed chat value still yields a chat entry, so neither
// can pass by rejecting or accepting everything. Property 24 additionally pins
// the two JavaScript traps the reader's explicit guards exist for
// (`typeof [] === "object"` and `typeof null === "object"`) by generating an
// array that carries well-formed `number` and `harness` fields, so dropping
// `Array.isArray` from `chatOf` fails the test rather than passing unnoticed.

TestCase {
  id: tc
  name: "PitwallBriefTicker"

  // Iterations per property (spec minimum: 100).
  readonly property int iterations: 128

  // ------------------------------------------------------------------
  // MIRRORS of Widget.qml (see file header)
  // ------------------------------------------------------------------

  // Widget.qml: readonly property real tickerSpeedPxPerSec: 180  (Scroll_Rate)
  readonly property real scrollRate: 180

  // Widget.qml: function tickerDurationMs(fromX, contentW)
  function mirrorTickerDurationMs(fromX, contentW) {
    return Math.max(1, (fromX + contentW) / tc.scrollRate * 1000)
  }

  // Widget.qml: function summaryShort()
  function mirrorSummaryShort(summary) {
    if (!summary) return ""
    if (summary.status === "error") return ""
    return String(summary.text || "").replace(/\s+/g, " ").trim()
  }

  // Widget.qml tickerPass duration binding:
  //   duration: root.tickerDurationMs(tickerTrack.x, marqueeText.implicitWidth)
  // The only text-derived input is the measured natural width, supplied here by
  // a stub measurer standing in for Text.implicitWidth.
  function mirrorPassDurationForBrief(text, fromX, measure) {
    return tc.mirrorTickerDurationMs(fromX, measure(text))
  }

  // Widget.qml tickerViewport: visible: root.summaryShort() !== ""
  function mirrorViewportVisible(summary) {
    return tc.mirrorSummaryShort(summary) !== ""
  }

  // Widget.qml tickerTrack.onXChanged: root.tickerOffset = x
  function mirrorTrackXChanged(state, newX) {
    state.x = newX
    state.tickerOffset = newX
  }

  // Widget.qml tickerTrack.Component.onCompleted:
  //   x = isNaN(root.tickerOffset) ? tickerViewport.width : root.tickerOffset
  function mirrorTrackCompleted(state, viewportW) {
    state.x = isNaN(state.tickerOffset) ? viewportW : state.tickerOffset
    state.tickerOffset = state.x
    return state.x
  }

  // Widget.qml tickerTrack.startPassFromRightEdge()
  function mirrorStartPassFromRightEdge(state, viewportW) {
    state.tickerOffset = viewportW
    state.x = viewportW
    return state.x
  }

  // Widget.qml tickerPass: from: tickerTrack.x, to: -marqueeText.implicitWidth,
  // duration: root.tickerDurationMs(...), easing.type: Easing.Linear.
  // Position of x after tMs of a pass that began at fromX.
  function mirrorPassXAt(fromX, contentW, durationMs, tMs) {
    var to = -contentW
    var progress = durationMs <= 0 ? 1 : Math.min(1, Math.max(0, tMs / durationMs))
    return fromX + (to - fromX) * progress
  }

  // Widget.qml: function briefStatusText()
  // `root.generating` is a property, passed in here as an argument.
  function mirrorBriefStatusText(summary, generating) {
    if (generating) return "Generating…"
    if (!summary) return "No brief yet"
    var st = String(summary.status || "")
    if (st === "ready") return ""
    if (st === "stale") return "Outdated"
    var msg = String(summary.message || "")
    return msg !== "" ? "Unavailable — " + msg : "Unavailable"
  }

  // Widget.qml: function briefStatusAttention()
  function mirrorBriefStatusAttention(summary, generating) {
    if (generating || !summary) return false
    var st = String(summary.status || "")
    return st === "stale" || st === "error" || st === "unavailable"
  }

  // Widget.qml: function briefNeedsGenerate()
  function mirrorBriefNeedsGenerate(summary, generating) {
    if (generating) return false
    if (!summary) return true
    return String(summary.status || "") !== "ready"
  }

  // Widget.qml: function attentionText()
  function mirrorAttentionText(summary) {
    if (!summary || summary.status !== "ready") return ""
    var m = String(summary.text || "").match(/needs attention:?([^\n]*)/i)
    if (!m) return ""
    var line = m[1].trim().replace(/\s+/g, " ")
    if (line === "") return ""
    return line.length > 140 ? line.slice(0, 137) + "…" : line
  }

  // ------------------------------------------------------------------
  // MIRRORS of Widget.qml — Open_Chat_Control (Property 6)
  // ------------------------------------------------------------------

  // Widget.qml: readonly property string pitwallBinary
  // The real value is `$HOME + "/.local/bin/pitwall"` (or the bare name when
  // HOME is empty); Quickshell.env does not exist here, so the installed shape
  // is stubbed. It is deliberately free of shell metacharacters, so any
  // metacharacter the assertions find in argv[0] came from the code under test.
  readonly property string pitwallBinaryStub: "/home/dev/.local/bin/pitwall"

  // Widget.qml: function chatPinnedIsResumable(pinned)
  function mirrorChatPinnedIsResumable(pinned) {
    return pinned !== "" && pinned.charAt(0) === "r"
  }

  // Widget.qml: function openChat()
  //
  // Returns the launch decision rather than performing it, so the "produces no
  // launch at all" branch is assertable: `launched` is true exactly where
  // `openChat` reaches `runFixed(argv, …)`, and `argv` is null wherever it
  // returns early. The result-line text is carried too because the refusal
  // branch's announce(..., true) is part of the same law (8.6).
  function mirrorOpenChat(selectedId) {
    var out = { launched: false, argv: null, refused: false,
                announcement: "", attention: false, viaResumable: false }
    var argv = [tc.pitwallBinaryStub, "chat"]
    var pinned = String(selectedId || "")
    var viaResumable = tc.mirrorChatPinnedIsResumable(pinned)
    out.viaResumable = viaResumable
    if (pinned !== "" && !viaResumable) {
      if (!/^sess_[0-9a-f]{16}$/.test(pinned)) {
        // Widget.qml refuses here and returns before runFixed is reached.
        out.refused = true
        out.announcement = "Could not open chat: invalid session."
        out.attention = true
        return out
      }
      argv.push("--session")
      argv.push(pinned)
    }
    // Widget.qml: runFixed(argv, function(code) { … }) — the single spawn point.
    out.launched = true
    out.argv = argv
    out.announcement = viaResumable
      ? "Pitwall Chat opening with workspace context \u2014 the pinned entry is not live."
      : "Pitwall Chat opening."
    out.attention = false
    return out
  }

  // ------------------------------------------------------------------
  // MIRRORS of StateReader.qml (see file header)
  // ------------------------------------------------------------------

  // StateReader.qml: function chatOf(s)
  function mirrorChatOf(s) {
    if (!s || typeof s !== "object" || Array.isArray(s)) return null
    var c = s.chat
    if (!c || typeof c !== "object" || Array.isArray(c)) return null
    if (typeof c.number !== "string" || !/^[0-9]{3}$/.test(c.number)) return null
    if (typeof c.harness !== "string" || c.harness.trim().length === 0) return null
    var model = (typeof c.model === "string") ? c.model : ""
    var ctx = (typeof c.context_session_id === "string" && c.context_session_id.length > 0)
      ? c.context_session_id : null
    var started = Number(c.started_at)
    return {
      number: c.number,
      harness: c.harness,
      model: model,
      model_label: (model !== "") ? model : "agent default",
      context_label: (typeof c.context_label === "string") ? c.context_label : "",
      context_session_id: ctx,
      started_at: (isFinite(started) && started > 0) ? Math.round(started) : 0
    }
  }

  // StateReader.qml: function stateOf(s)
  function mirrorStateOf(s) {
    var st = String((s && s.state) || "unknown")
    return (st === "running" || st === "sleeping" || st === "stopped") ? st : "unknown"
  }

  // StateReader.qml: readonly property var chatSessions
  // Takes the sessions array (the binding reads `sessions`, which is itself
  // mirrored below) and yields the sessions carrying a valid chat object,
  // ordered by chat number ascending with snapshot index as the tie-break.
  function mirrorChatSessions(sessions) {
    var rows = []
    for (var i = 0; i < sessions.length; i++) {
      var c = tc.mirrorChatOf(sessions[i])
      if (c) rows.push({ order: Number(c.number), index: i, session: sessions[i] })
    }
    rows.sort(function (a, b) {
      return (a.order - b.order) || (a.index - b.index)
    })
    var out = []
    for (var j = 0; j < rows.length; j++) out.push(rows[j].session)
    return out
  }

  // StateReader.qml: readonly property var sessions
  function mirrorRecordSessions(record) {
    return (record && Array.isArray(record.sessions)) ? record.sessions : []
  }

  // StateReader.qml: readonly property var resumable
  function mirrorRecordResumable(record) {
    return (record && Array.isArray(record.resumable)) ? record.resumable : []
  }

  // StateReader.qml: readonly property var notifications
  function mirrorRecordNotifications(record) {
    return (record && Array.isArray(record.notifications)) ? record.notifications : []
  }

  // StateReader.qml: readonly property int unreadCount
  function mirrorRecordUnreadCount(record) {
    var n = record ? Number(record.unread_count) : 0
    return isFinite(n) && n > 0 ? Math.round(n) : 0
  }

  // StateReader.qml: readonly property var summary
  function mirrorRecordSummary(record) {
    var s = record ? record.summary : null
    if (!s || typeof s !== "object" || Array.isArray(s)) return null
    var status = String(s.status || "")
    if (status !== "ready" && status !== "error" && status !== "unavailable"
        && status !== "stale") return null
    return {
      text: String(s.text || ""),
      model: s.model ? String(s.model) : "",
      created_at: Number(s.created_at) || 0,
      input_hash: String(s.input_hash || ""),
      status: status,
      message: String(s.message || "")
    }
  }

  // StateReader.qml: readonly property string pitwallVersion
  function mirrorRecordPitwallVersion(record) {
    var v = record ? record.pitwall_version : null
    if (typeof v !== "string") return ""
    return v.trim()
  }

  // StateReader.qml: function sessionLabel(s)
  function mirrorSessionLabel(s) {
    s = s || {}
    var p = s.project || null
    var proj = p ? (p.name || "session") : "session"
    var branch = (p && p.branch) ? ":" + p.branch : ""
    return proj + branch
  }

  // StateReader.qml: function parse(content) — the acceptance test only.
  // `record` stays null (and the whole panel empties) for anything this
  // rejects, so Property 24 asserts it accepts every document it builds: a
  // malformed `chat` value must not cost the document its parse.
  function mirrorParseAccepts(parsed) {
    var version = Number(parsed && parsed.state_version)
    return !!(parsed && typeof parsed === "object"
      && (version === 1 || version === 2 || version === 3 || version === 4)
      && Array.isArray(parsed.sessions))
  }

  // ------------------------------------------------------------------
  // Deterministic generators (no PBT library exists for QML)
  // ------------------------------------------------------------------

  property int rngState: 1

  // Lehmer / MINSTD: state stays inside 2^31 and every intermediate product
  // stays inside 2^53, so JS number precision is exact.
  function seed(s) {
    tc.rngState = (s % 2147483646) + 1
  }

  function nextUnit() {
    tc.rngState = (tc.rngState * 48271) % 2147483647
    return (tc.rngState - 1) / 2147483646
  }

  function nextInt(maxExclusive) {
    return Math.floor(tc.nextUnit() * maxExclusive)
  }

  function nextReal(lo, hi) {
    return lo + tc.nextUnit() * (hi - lo)
  }

  // ------------------------------------------------------------------
  // AI_Brief_Surface presentation model + provenance checker (Property 5)
  // ------------------------------------------------------------------

  // The COMPLETE fixed panel string allowlist for the AI_Brief_Surface. Every
  // entry is a literal in Widget.qml's brief card; nothing else may appear.
  // Keep this list exhaustive and small — if a new fixed string is added to
  // the card, adding it here is a deliberate act, and until then Property 5
  // fails, which is the point.
  readonly property var fixedPanelStrings: [
    "AI BRIEF",         // Widget.qml briefCard caption row
    "Generating…",      // briefStatusText(), run in progress (6.6)
    "No brief yet",     // briefStatusText(), record absent (6.4)
    "Outdated",         // briefStatusText(), status stale (6.2)
    "Unavailable",      // briefStatusText(), error/unavailable, no record message (6.3)
    "Generate summary"  // generateBriefButton label (6.2, 6.3, 6.4)
  ]

  // The two fixed affixes the card puts in front of record-carried text.
  readonly property string unavailablePrefix: "Unavailable — " // briefStatusText()
  readonly property string attentionPrefix: "! "               // attention line

  // Glyph-only indicators are not sentences and carry no meaning alone (the
  // status word beside them does), so they are excluded from the sentence set
  // deliberately rather than by omission.
  readonly property var nonSentenceGlyphs: ["●", "○",
    String.fromCodePoint(0xF0140), String.fromCodePoint(0xF0142)]

  // Every sentence the AI_Brief_Surface presents, in card order, gated by the
  // same `visible:` conditions Widget.qml uses.
  function briefPresentedSentences(summary, generating) {
    var out = []
    // (a) caption row.
    out.push({ role: "caption", text: "AI BRIEF" })
    // (b) ticker viewport / expanded summary: both bind root.summaryShort().
    var body = tc.mirrorSummaryShort(summary)
    if (body !== "") {
      out.push({ role: "ticker", text: body })
      out.push({ role: "expanded", text: body })
    }
    // (d) status line, and the generate control beside it.
    var status = tc.mirrorBriefStatusText(summary, generating)
    if (status !== "") out.push({ role: "status", text: status })
    if (tc.mirrorBriefNeedsGenerate(summary, generating))
      out.push({ role: "generate", text: "Generate summary" })
    // attention line.
    var attention = tc.mirrorAttentionText(summary)
    if (attention !== "")
      out.push({ role: "attention", text: tc.attentionPrefix + attention })
    return out
  }

  function collapse(s) {
    return String(s).replace(/\s+/g, " ").trim()
  }

  // Classifies where a presented sentence came from. Returns "" when the
  // sentence cannot be traced to the record or to the allowlist — which is
  // exactly the signal that the panel has started inventing prose.
  function briefProvenanceOf(text, summary) {
    if (tc.fixedPanelStrings.indexOf(text) !== -1) return "fixed-panel-string"

    var body = tc.mirrorSummaryShort(summary)
    if (body !== "" && text === body) return "record-text"

    // "Unavailable — " + the record's own failure message, verbatim.
    var msg = summary ? String(summary.message || "") : ""
    if (msg !== "" && text === tc.unavailablePrefix + msg)
      return "fixed-prefix + record-message"

    // "! " + a run taken out of the record's own text (possibly truncated with
    // a single trailing ellipsis).
    if (text.indexOf(tc.attentionPrefix) === 0) {
      var line = text.slice(tc.attentionPrefix.length)
      if (line.charAt(line.length - 1) === "…") line = line.slice(0, -1)
      var haystack = summary ? tc.collapse(String(summary.text || "")) : ""
      if (line !== "" && haystack.indexOf(line) !== -1)
        return "fixed-prefix + record-text-run"
    }

    return ""
  }

  // Word-level backstop for the same law: a sentence may only use words the
  // allowlist itself uses or words the record carries. This is what fails if
  // an interpretive clause is ever spliced onto an otherwise legal sentence.
  function wordsOf(s) {
    var found = String(s).match(/[0-9A-Za-z]+/g)
    return found ? found : []
  }

  function briefAllowedWordList(summary) {
    var allowed = []
    var i
    for (i = 0; i < tc.fixedPanelStrings.length; i++)
      allowed = allowed.concat(tc.wordsOf(tc.fixedPanelStrings[i]))
    allowed = allowed.concat(tc.wordsOf(tc.unavailablePrefix))
    if (summary) {
      allowed = allowed.concat(tc.wordsOf(String(summary.text || "")))
      allowed = allowed.concat(tc.wordsOf(String(summary.message || "")))
    }
    return allowed
  }

  // Returns the first word of `text` that is neither an allowlist word nor a
  // word the record carries, or "" when every word is traceable. The one
  // concession: the attention line's 140-character truncation can cut its last
  // word in half, so a trailing fragment counts as traceable when it is the
  // start of a word the record carries.
  function briefUntraceableWord(text, summary) {
    var allowed = tc.briefAllowedWordList(summary)
    var map = {}
    for (var a = 0; a < allowed.length; a++) map[allowed[a]] = true

    var words = tc.wordsOf(text)
    var truncated = String(text).charAt(String(text).length - 1) === "…"
    for (var i = 0; i < words.length; i++) {
      if (map[words[i]] === true) continue
      if (truncated && i === words.length - 1) {
        var prefixOfRecordWord = false
        for (var k = 0; k < allowed.length; k++) {
          if (allowed[k].indexOf(words[i]) === 0) { prefixOfRecordWord = true; break }
        }
        if (prefixOfRecordWord) continue
      }
      return words[i]
    }
    return ""
  }

  // ------------------------------------------------------------------
  // MIRRORS of SessionBar.qml — chat rail entry label (Property 25)
  // ------------------------------------------------------------------
  //
  // RECONCILED WITH TASK 13.2. When this test was started the rail entry label
  // builder did not exist and the composition below was mirrored from
  // Requirement 18.2 plus the middot grammar design §5.3 shows for the chat
  // window title. Task 13.2 ("Render chat entries in SessionBar.qml") landed
  // while this test was being written, so the mirror was repointed at the
  // shipped `SessionBar.qml` `labelText` chat branch, which agrees with that
  // reading: `Pitwall Chat <number>`, then Harness, Model-or-agent-default and
  // Context_Label as middot-separated segments with empty facts dropped, closed
  // by the observed state word.
  //
  // `chatFacts` is parent-supplied (`stateReader.chatOf(s)`), so the mirror
  // feeds `mirrorChatOf` into it exactly as the panel does. `selected` only
  // picks the expand affordance glyph.
  //
  // Any edit to `SessionBar.qml`'s `stateKey`, `stateWord`, `stateGlyph` or the
  // chat branch of `labelText` MUST be mirrored here.
  readonly property string chatLiteral: "Pitwall Chat"
  readonly property string chatSeparator: " \u00b7 "

  // SessionBar.qml: readonly property string stateKey
  // (the same fold StateReader.stateOf performs)
  function mirrorStateKey(entry) {
    var st = String((entry && entry.state) || "unknown")
    return (st === "running" || st === "sleeping" || st === "stopped") ? st : "unknown"
  }

  // SessionBar.qml: readonly property string stateWord — the panel's existing
  // state vocabulary (the same words Widget.qml's session detail has always
  // used: active / idle / waiting / unknown).
  function mirrorStateWord(stateKey) {
    if (stateKey === "running") return "active"
    if (stateKey === "sleeping") return "idle"
    if (stateKey === "stopped") return "waiting"
    return "unknown"
  }

  // SessionBar.qml: readonly property string stateGlyph
  function mirrorStateGlyph(stateKey) {
    return stateKey === "running" ? "●" : "○"
  }

  // SessionBar.qml: readonly property string labelText, chat branch only.
  function mirrorChatEntryLabel(entry, selected) {
    var chatFacts = tc.mirrorChatOf(entry)
    if (!chatFacts) return ""
    var stateKey = tc.mirrorStateKey(entry)
    var seg = [tc.mirrorStateGlyph(stateKey) + " Pitwall Chat "
               + String(chatFacts.number || "")]
    var harness = String(chatFacts.harness || "")
    if (harness !== "") seg.push(harness)
    var modelLabel = String(chatFacts.model_label || "")
    if (modelLabel !== "") seg.push(modelLabel)
    var ctx = String(chatFacts.context_label || "")
    if (ctx !== "") seg.push(ctx)
    seg.push(tc.mirrorStateWord(stateKey))
    return seg.join(" · ") + (selected ? "  ▴" : "  ▾")
  }

  function mirrorChatEntryStateWord(entry) {
    return tc.mirrorStateWord(tc.mirrorStateKey(entry))
  }

  // The existing panel state vocabulary (18.3).
  readonly property var stateVocabulary: ["active", "idle", "waiting", "unknown"]

  // Presentation glyphs on a rail entry: state shape and expand affordance.
  // Not facts, so the fact-containment checks skip them explicitly.
  readonly property var railEntryGlyphs: ["●", "○", "▴", "▾"]

  // Words use only letters and digits, so the "no marker / no separator /
  // no filler" assertions in Property 1 cannot be satisfied by generated text.
  readonly property string wordAlphabet: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
  readonly property var whitespaceChars: [" ", "\t", "\n", "\r", "\u000b", "\f"]

  function randomWord() {
    var n = 1 + tc.nextInt(12)
    var w = ""
    for (var i = 0; i < n; i++)
      w += tc.wordAlphabet.charAt(tc.nextInt(tc.wordAlphabet.length))
    return w
  }

  function randomWhitespaceRun() {
    var n = 1 + tc.nextInt(4)
    var s = ""
    for (var i = 0; i < n; i++)
      s += tc.whitespaceChars[tc.nextInt(tc.whitespaceChars.length)]
    return s
  }

  // A brief with irregular whitespace runs, plus leading/trailing whitespace.
  function randomBriefText() {
    var words = tc.nextInt(40)
    var s = tc.nextUnit() < 0.3 ? tc.randomWhitespaceRun() : ""
    for (var i = 0; i < words; i++) {
      s += tc.randomWord()
      if (i < words - 1) s += tc.randomWhitespaceRun()
    }
    if (tc.nextUnit() < 0.3) s += tc.randomWhitespaceRun()
    return s
  }

  function stripWhitespace(s) {
    return s.replace(/\s+/g, "")
  }

  // Summary_Record shapes as StateReader.summary yields them: the four legal
  // status words, a text, an optional failure message and the metadata keys.
  readonly property var summaryStatuses: ["ready", "stale", "error", "unavailable"]

  function randomSentence(words) {
    var s = ""
    for (var i = 0; i < words; i++) s += (i > 0 ? " " : "") + tc.randomWord()
    return s
  }

  function randomSummaryRecord() {
    var text = tc.randomBriefText()
    // Sometimes the brief carries its own "Needs attention" section, which is
    // the only thing that makes the attention line appear at all.
    var roll = tc.nextUnit()
    if (roll < 0.2) text = "Needs attention: " + tc.randomSentence(4) + "\n" + text
    else if (roll < 0.3) text = "needs attention " + tc.randomSentence(40) + "\n" + text
    return {
      text: text,
      status: tc.summaryStatuses[tc.nextInt(tc.summaryStatuses.length)],
      message: tc.nextUnit() < 0.5 ? tc.randomSentence(1 + tc.nextInt(4)) : "",
      model: tc.nextUnit() < 0.5 ? tc.randomWord() : "",
      created_at: 1700000000 + tc.nextInt(100000),
      input_hash: tc.randomWord()
    }
  }

  // Chat sessions as they appear in state.json v4 (design §5.3).
  readonly property var harnessNames: ["opencode", "claude", "codex"]

  function randomChatNumber() {
    var n = ""
    for (var i = 0; i < 3; i++) n += String(tc.nextInt(10))
    return n
  }

  function randomSessionId() {
    var hex = "0123456789abcdef"
    var s = "sess_"
    for (var i = 0; i < 16; i++) s += hex.charAt(tc.nextInt(16))
    return s
  }

  function randomChatSession() {
    // State words: the three legal ones plus values the vocabulary must fold
    // into "unknown".
    var stateRoll = tc.nextInt(6)
    var state
    if (stateRoll === 0) state = "running"
    else if (stateRoll === 1) state = "sleeping"
    else if (stateRoll === 2) state = "stopped"
    else if (stateRoll === 3) state = "zombie"
    else if (stateRoll === 4) state = ""
    else state = undefined

    return {
      id: tc.randomSessionId(),
      state: state,
      role: "chat",
      chat: {
        number: tc.randomChatNumber(),
        harness: tc.nextUnit() < 0.6
          ? tc.harnessNames[tc.nextInt(tc.harnessNames.length)]
          : tc.randomWord(),
        // "" is legitimate and means agent default.
        model: tc.nextUnit() < 0.3 ? "" : tc.randomWord(),
        // "" is what StateReader yields for a document with no context label.
        context_label: tc.nextUnit() < 0.2 ? "" : tc.randomSentence(1 + tc.nextInt(2)),
        context_session_id: tc.nextUnit() < 0.5 ? tc.randomSessionId() : null,
        started_at: 1700000000 + tc.nextInt(100000)
      }
    }
  }

  // ------------------------------------------------------------------
  // Generators for Property 6 — pinned-selection values
  // ------------------------------------------------------------------

  readonly property string hexAlphabet: "0123456789abcdef"
  // Letters that cannot appear in a lowercase-hex session id, used to build
  // near-misses that can never accidentally satisfy `sess_[0-9a-f]{16}`.
  readonly property string nonHexAlphabet: "ghijklmnopqstuvwxyz"

  // Characters a shell would act on. argv is a vector, never a command line, so
  // none of these may ever be introduced by the panel; a generated selection
  // carrying one must be refused rather than passed through.
  readonly property var shellMetacharacters: [
    ";", "&", "|", "`", "$", "(", ")", "<", ">", "\n", "\r", "\t", " ",
    "\"", "'", "\\", "*", "?", "{", "}", "[", "]", "!", "#", "~", "="
  ]

  function containsShellMetacharacter(s) {
    var str = String(s)
    for (var i = 0; i < tc.shellMetacharacters.length; i++) {
      if (str.indexOf(tc.shellMetacharacters[i]) !== -1)
        return tc.shellMetacharacters[i]
    }
    return ""
  }

  function randomHexRun(n) {
    var s = ""
    for (var i = 0; i < n; i++) s += tc.hexAlphabet.charAt(tc.nextInt(16))
    return s
  }

  function randomNonHexWord() {
    var n = 1 + tc.nextInt(8)
    var w = ""
    for (var i = 0; i < n; i++)
      w += tc.nonHexAlphabet.charAt(tc.nextInt(tc.nonHexAlphabet.length))
    return w
  }

  // A pinned resumable: the panel keys these off a leading "r" alone
  // (`chatPinnedIsResumable`), so both the `r:` form and a bare one are drawn.
  function randomResumableSelection() {
    return (tc.nextUnit() < 0.7 ? "r:" : "r") + tc.randomHexRun(16)
  }

  // Selections that look almost like a session id but are not one. Every branch
  // is built so it cannot collide with the valid shape: uppercase forms are
  // forced to carry at least one A–F letter, and interior junk is drawn from the
  // non-hex alphabet.
  function randomNearMissSelection() {
    var hex = tc.randomHexRun(16)
    switch (tc.nextInt(11)) {
      case 0: return "sess_" + tc.randomHexRun(15)                  // 15 digits
      case 1: return "sess_" + tc.randomHexRun(17)                  // 17 digits
      case 2: {                                                     // uppercase hex
        var pos = tc.nextInt(16)
        var upper = hex.slice(0, pos) + "ABCDEF".charAt(tc.nextInt(6))
                    + hex.slice(pos + 1)
        return "sess_" + upper
      }
      case 3: return "xsess_" + hex                                 // wrong prefix
      case 4: return "session_" + hex                               // wrong prefix
      case 5: return "sess-" + hex                                  // wrong separator
      case 6: return "sess_" + hex
        + tc.shellMetacharacters[tc.nextInt(tc.shellMetacharacters.length)]
        + tc.randomNonHexWord()                                     // trailing junk
      case 7: return "-" + hex                                      // leading dash
      case 8: return "--session"                                    // a bare flag
      case 9: return "sess_" + hex.slice(0, 8) + tc.randomNonHexWord() // interior junk
      default: return "sess_" + hex + "/../../etc/passwd"            // path traversal
    }
  }

  // ------------------------------------------------------------------
  // Generators for Property 24 — documents with a malformed `chat` value
  // ------------------------------------------------------------------

  // Every value below fails at least one `chatOf` gate. Both `typeof []` and
  // `typeof null` are "object" in JavaScript, which is why the array and null
  // cases are here and why the mirror rejects them explicitly.
  readonly property int malformedChatKindCount: 24

  function malformedChatKindName(kind) {
    var names = ["null", "undefined", "empty array", "array wrapping a valid object",
      "empty string", "random string", "zero", "number", "true", "false",
      "empty object", "number as integer", "number with 2 digits",
      "number with 4 digits", "number with whitespace", "number empty",
      "harness empty", "harness whitespace", "harness non-string",
      "harness missing", "number missing", "number non-digits", "harness null",
      "array carrying valid chat keys"]
    return names[kind] || ("kind " + kind)
  }

  function malformedChatValue(kind) {
    switch (kind) {
      case 0: return null
      case 1: return undefined
      case 2: return []
      case 3: return [{ number: "001", harness: "opencode" }]
      case 4: return ""
      case 5: return tc.randomWord()
      case 6: return 0
      case 7: return 1700000000 + tc.nextInt(1000)
      case 8: return true
      case 9: return false
      case 10: return {}
      case 11: return { number: 1 + tc.nextInt(998), harness: "opencode" }
      case 12: return { number: String(10 + tc.nextInt(90)), harness: "opencode" }
      case 13: return { number: tc.randomChatNumber() + String(tc.nextInt(10)),
                        harness: "opencode" }
      case 14: return { number: " " + tc.randomChatNumber() + " ", harness: "opencode" }
      case 15: return { number: "", harness: "opencode" }
      case 16: return { number: tc.randomChatNumber(), harness: "" }
      case 17: return { number: tc.randomChatNumber(), harness: "   \t " }
      case 18: return { number: tc.randomChatNumber(), harness: 42 }
      case 19: return { number: tc.randomChatNumber() }
      case 20: return { harness: "opencode" }
      case 21: return { number: tc.randomNonHexWord().slice(0, 3), harness: "opencode" }
      case 22: return { number: tc.randomChatNumber(), harness: null }
      default: {
        // An array that satisfies every *other* gate: `typeof arr === "object"`
        // is true and both string fields are well formed, so only the explicit
        // `Array.isArray` guard can reject it. This is the case that makes that
        // guard load-bearing rather than decorative.
        var arr = []
        arr.number = tc.randomChatNumber()
        arr.harness = tc.harnessNames[tc.nextInt(tc.harnessNames.length)]
        arr.model = tc.randomWord()
        arr.context_label = tc.randomWord()
        arr.started_at = 1700000000 + tc.nextInt(100000)
        return arr
      }
    }
  }

  // An ordinary (non-chat) session as the collector emits it.
  function randomOrdinarySession() {
    var states = ["running", "sleeping", "stopped"]
    return {
      id: tc.randomSessionId(),
      state: states[tc.nextInt(states.length)],
      role: tc.nextUnit() < 0.5 ? "agent" : "editor",
      project: { name: tc.randomWord(), branch: tc.nextUnit() < 0.5 ? tc.randomWord() : "" },
      last_activity: { epoch: 1700000000 + tc.nextInt(100000) },
      root_pid: 1000 + tc.nextInt(30000)
    }
  }

  function randomResumableRow() {
    return {
      id: "r:" + tc.randomHexRun(16),
      project: tc.randomWord(),
      created_at: 1700000000 + tc.nextInt(100000)
    }
  }

  function randomNotificationRow() {
    return {
      id: 1 + tc.nextInt(500),
      kind: tc.randomWord(),
      message: tc.randomSentence(1 + tc.nextInt(4)),
      created_at: 1700000000 + tc.nextInt(100000)
    }
  }

  // A whole v4 document. `victimChat` is substituted verbatim for the last
  // session's `chat` key; `chatCount` well-formed chat sessions ride alongside
  // so the test can see that a malformed value damaged none of them.
  function buildStateDocument(victimChat, includeVictimChatKey, chatCount) {
    var sessions = []
    var ordinary = 1 + tc.nextInt(2)
    for (var o = 0; o < ordinary; o++) sessions.push(tc.randomOrdinarySession())
    for (var c = 0; c < chatCount; c++) sessions.push(tc.randomChatSession())

    var victim = tc.randomOrdinarySession()
    if (includeVictimChatKey) victim.chat = victimChat
    sessions.push(victim)

    var resumable = []
    var rows = tc.nextInt(3)
    for (var r = 0; r < rows; r++) resumable.push(tc.randomResumableRow())

    var notifications = []
    var notes = tc.nextInt(3)
    for (var n = 0; n < notes; n++) notifications.push(tc.randomNotificationRow())

    var summary = tc.randomSummaryRecord()

    return {
      doc: {
        state_version: 4,
        collected_at: 1700000000 + tc.nextInt(100000),
        pitwall_version: "0." + tc.nextInt(20) + "." + tc.nextInt(20),
        sessions: sessions,
        resumable: resumable,
        notifications: notifications,
        unread_count: notifications.length,
        summary: summary
      },
      victim: victim,
      victimIndex: sessions.length - 1,
      chatCount: chatCount
    }
  }

  function closeEnough(a, b, eps) {
    return Math.abs(a - b) <= eps
  }

  // ------------------------------------------------------------------
  // Properties
  // ------------------------------------------------------------------

  // **Validates: Requirements 1.2, 2.6, 5.4**
  // Feature: pitwall-chat-and-brief-ticker, Property 1: Ticker text is the brief and nothing else — For any summary text, the string rendered by the AI_Brief_Ticker equals that text with whitespace runs collapsed — with no truncation marker, no separator, and no filler character appended — and is empty (hiding the viewport) exactly when the normalised text is empty.
  function test_property1_ticker_text_is_the_brief_and_nothing_else() {
    tc.seed(20250101)
    for (var i = 0; i < tc.iterations; i++) {
      // Forced corner cases first, generated text afterwards.
      var summary
      if (i === 0) summary = null
      else if (i === 1) summary = { text: "", status: "ready" }
      else if (i === 2) summary = { text: "   \t\n  ", status: "ready" }
      else if (i === 3) summary = { text: tc.randomBriefText(), status: "error" }
      else {
        var statuses = ["ready", "stale", "unavailable"]
        summary = { text: tc.randomBriefText(),
                    status: statuses[tc.nextInt(statuses.length)] }
      }

      var raw = summary ? String(summary.text || "") : ""
      var normalised = raw.replace(/\s+/g, " ").trim()
      var expected = (!summary || summary.status === "error") ? "" : normalised
      var rendered = tc.mirrorSummaryShort(summary)

      verify(rendered === expected,
             "iteration " + i + ": rendered text is not the normalised brief; got "
             + JSON.stringify(rendered) + " want " + JSON.stringify(expected))

      if (expected !== "") {
        // Nothing truncated and nothing appended: every non-whitespace
        // character of the brief survives, in order, and no others exist.
        verify(tc.stripWhitespace(rendered) === tc.stripWhitespace(raw),
               "iteration " + i + ": brief characters were dropped or added")
        // No truncation marker, no separator, no filler.
        verify(rendered.indexOf("\u2026") === -1, "iteration " + i + ": ellipsis marker present")
        verify(rendered.indexOf("...") === -1, "iteration " + i + ": ellipsis marker present")
        verify(rendered.indexOf("\u2192") === -1, "iteration " + i + ": arrow affordance marker present")
        verify(rendered.indexOf("\u00b7") === -1, "iteration " + i + ": middot filler present")
        // Whitespace runs collapsed to a single space, no edge whitespace.
        verify(rendered.indexOf("  ") === -1, "iteration " + i + ": whitespace run not collapsed")
        verify(rendered === rendered.trim(), "iteration " + i + ": edge whitespace kept")
      }

      // Empty exactly when the normalised text is empty, and the viewport
      // (and with it the pass) is hidden exactly in that case.
      verify((rendered === "") === (expected === ""),
             "iteration " + i + ": emptiness disagrees with the normalised text")
      verify(tc.mirrorViewportVisible(summary) === (rendered !== ""),
             "iteration " + i + ": viewport visibility does not follow the rendered text")
    }
  }

  // **Validates: Requirements 1.3, 2.1, 2.2, 2.3, 2.4, 5.1, 5.3**
  // Feature: pitwall-chat-and-brief-ticker, Property 2: Every pass covers the whole travel distance, right to left — For any content width and viewport width, a pass starts at x = viewportWidth, ends at x = -contentWidth, moves monotonically non-increasing throughout, and the pass that follows starts again at x = viewportWidth.
  function test_property2_every_pass_covers_the_whole_travel_distance_right_to_left() {
    tc.seed(20250202)
    var samples = 32
    for (var i = 0; i < tc.iterations; i++) {
      var viewportW = tc.nextReal(40, 1400)
      // Content narrower than, equal to and far wider than the viewport.
      var contentW = i % 4 === 0 ? viewportW : tc.nextReal(1, 24000)

      var state = { x: 0, tickerOffset: NaN }
      var start = tc.mirrorStartPassFromRightEdge(state, viewportW)
      verify(start === viewportW,
             "iteration " + i + ": pass did not start at the right edge")
      verify(state.tickerOffset === viewportW,
             "iteration " + i + ": start offset not mirrored to root.tickerOffset")

      var duration = tc.mirrorTickerDurationMs(start, contentW)
      var previous = start
      for (var k = 1; k <= samples; k++) {
        var xk = tc.mirrorPassXAt(start, contentW, duration, duration * k / samples)
        verify(xk <= previous + 1e-9,
               "iteration " + i + ": x moved right at sample " + k
               + " (" + previous + " -> " + xk + ")")
        previous = xk
      }

      var end = tc.mirrorPassXAt(start, contentW, duration, duration)
      verify(tc.closeEnough(end, -contentW, 1e-6),
             "iteration " + i + ": pass ended at " + end + ", want " + (-contentW))

      // The whole travel distance was covered, right to left.
      verify(tc.closeEnough(start - end, viewportW + contentW, 1e-6),
             "iteration " + i + ": travel distance was not viewportW + contentW")

      // onFinished -> startPassFromRightEdge(): the next pass enters from the right.
      tc.mirrorTrackXChanged(state, end)
      var nextStart = tc.mirrorStartPassFromRightEdge(state, viewportW)
      verify(nextStart === viewportW,
             "iteration " + i + ": following pass did not restart at the right edge")
      verify(state.tickerOffset === viewportW,
             "iteration " + i + ": following pass offset not reset to the right edge")
    }
  }

  // **Validates: Requirements 3.1, 3.3, 3.6, 5.2**
  // Feature: pitwall-chat-and-brief-ticker, Property 3: Pass duration is travel distance over rate — For any start offset and content width, pass duration in milliseconds equals (startOffset + contentWidth) / 180 * 1000 with no lower clamp, so the implied speed is exactly the Scroll_Rate for every brief length, and duration is a function of pixel geometry only (never of character count).
  function test_property3_pass_duration_is_travel_distance_over_rate() {
    tc.seed(20250303)
    for (var i = 0; i < tc.iterations; i++) {
      var contentW
      var startOffset
      if (i % 3 === 0) {
        // Deliberately short travel: the removed max(700, …) floor would show up
        // here as a duration far above the geometric one.
        contentW = tc.nextReal(1, 60)
        startOffset = tc.nextReal(0, 40)
      } else {
        contentW = tc.nextReal(1, 24000)
        startOffset = tc.nextReal(-contentW + 1, 1400)
      }

      var travel = startOffset + contentW
      var wanted = travel / 180 * 1000
      var duration = tc.mirrorTickerDurationMs(startOffset, contentW)

      verify(tc.closeEnough(duration, wanted, 1e-6),
             "iteration " + i + ": duration " + duration + " != travel/rate " + wanted)

      // No lower clamp: short travel really does produce a short pass.
      if (wanted < 700)
        verify(duration < 700,
               "iteration " + i + ": duration " + duration
               + " is floored above the geometric " + wanted)

      // The implied speed is exactly the Scroll_Rate, whatever the brief length.
      var impliedRate = travel / (duration / 1000)
      verify(tc.closeEnough(impliedRate, 180, 1e-6),
             "iteration " + i + ": implied speed " + impliedRate + " px/s != 180 px/s")

      // Geometry only, never character count: two briefs with wildly different
      // character counts but the same measured width cost the same pass. The
      // duration goes through the widget's binding mirror, whose only
      // text-derived input is the measured natural width.
      var shortBrief = tc.randomWord()
      var longBrief = ""
      for (var w = 0; w < 120; w++) longBrief += tc.randomWord() + " "
      verify(longBrief.length > shortBrief.length * 10,
             "iteration " + i + ": generated briefs are not different enough in length")
      // A proportional font can render very different character counts to the
      // same pixel width; this measurer stands in for Text.implicitWidth.
      var equalWidthMeasure = function (t) { return contentW }
      var durShort = tc.mirrorPassDurationForBrief(shortBrief, startOffset, equalWidthMeasure)
      var durLong = tc.mirrorPassDurationForBrief(longBrief, startOffset, equalWidthMeasure)
      verify(durShort === durLong,
             "iteration " + i + ": duration varied with character count ("
             + shortBrief.length + " vs " + longBrief.length + " chars): "
             + durShort + " vs " + durLong)
      verify(durShort === duration,
             "iteration " + i + ": the bound duration is not the geometric duration")

      // And it varies strictly with geometry: twice the travel, twice the time.
      var doubled = tc.mirrorTickerDurationMs(startOffset + travel, contentW)
      verify(tc.closeEnough(doubled, duration * 2, 1e-6),
             "iteration " + i + ": duration is not linear in travel distance")
    }
  }

  // **Validates: Requirements 4.2, 4.5**
  // Feature: pitwall-chat-and-brief-ticker, Property 4: Pause offset round-trips exactly — For any horizontal offset in [-contentWidth, viewportWidth], capturing that offset while paused and restoring it on resume yields the same offset, and the resumed pass ends at -contentWidth.
  function test_property4_pause_offset_round_trips_exactly() {
    tc.seed(20250404)
    for (var i = 0; i < tc.iterations; i++) {
      var viewportW = tc.nextReal(40, 1400)
      var contentW = tc.nextReal(1, 24000)

      // First iteration covers the no-pass-yet case: NaN offset means the pass
      // enters from the right edge instead of a restored position.
      if (i === 0) {
        var fresh = { x: 0, tickerOffset: NaN }
        var freshX = tc.mirrorTrackCompleted(fresh, viewportW)
        verify(freshX === viewportW,
               "iteration 0: NaN offset did not start the pass at the right edge")
      }

      // Offsets across the whole legal span, including both endpoints.
      var held
      if (i === 1) held = viewportW
      else if (i === 2) held = -contentW
      else held = tc.nextReal(-contentW, viewportW)

      // Paused pass: x holds still and onXChanged has already mirrored it to
      // root.tickerOffset, which survives panel content teardown.
      var state = { x: 0, tickerOffset: NaN }
      tc.mirrorTrackXChanged(state, held)
      var captured = state.tickerOffset
      verify(captured === held,
             "iteration " + i + ": paused offset was not captured exactly")

      // Panel content torn down and rebuilt: Component.onCompleted restores x.
      var rebuilt = { x: 0, tickerOffset: captured }
      var restored = tc.mirrorTrackCompleted(rebuilt, viewportW)
      verify(restored === held,
             "iteration " + i + ": offset did not round-trip exactly; got "
             + restored + " want " + held)
      verify(Math.abs(restored - held) === 0,
             "iteration " + i + ": offset drifted on resume")

      // The resumed pass runs from the restored offset and still ends at
      // -contentWidth, at the Scroll_Rate for the remaining travel.
      var duration = tc.mirrorTickerDurationMs(restored, contentW)
      var end = tc.mirrorPassXAt(restored, contentW, duration, duration)
      verify(tc.closeEnough(end, -contentW, 1e-6),
             "iteration " + i + ": resumed pass ended at " + end
             + ", want " + (-contentW))

      var remaining = restored + contentW
      if (remaining >= 1) {
        var impliedRate = remaining / (duration / 1000)
        verify(tc.closeEnough(impliedRate, 180, 1e-6),
               "iteration " + i + ": resumed pass speed " + impliedRate
               + " px/s != 180 px/s")
      }
    }
  }

  // **Validates: Requirements 6.7**
  // Feature: pitwall-chat-and-brief-ticker, Property 5: Presented brief content comes only from the Summary_Record — For any Summary_Record, every sentence the AI_Brief_Surface presents is either the normalised record text or a member of the fixed panel string allowlist (`AI BRIEF`, the status labels, the generate control label).
  function test_property5_presented_brief_content_comes_only_from_the_summary_record() {
    tc.seed(20250505)
    for (var i = 0; i < tc.iterations; i++) {
      // Forced corner cases first, generated records afterwards.
      var summary
      var generating = false
      if (i === 0) { summary = null }
      else if (i === 1) { summary = null; generating = true }
      else if (i === 2) {
        // A run in progress over a cached brief: the cached text stays on
        // screen (6.6) and is still the record's own text.
        summary = { text: tc.randomBriefText(), status: "ready", message: "" }
        generating = true
      }
      else if (i === 3) summary = { text: tc.randomBriefText(), status: "error",
                                    message: tc.randomSentence(3) }
      else if (i === 4) summary = { text: tc.randomBriefText(), status: "unavailable",
                                    message: tc.randomSentence(2) }
      else if (i === 5) summary = { text: tc.randomBriefText(), status: "unavailable",
                                    message: "" }
      else if (i === 6) summary = { text: tc.randomBriefText(), status: "stale",
                                    message: "" }
      else if (i === 7) summary = { text: "Needs attention: " + tc.randomSentence(60),
                                    status: "ready", message: "" }
      else if (i === 8) summary = { text: "   \t\n ", status: "ready", message: "" }
      else summary = tc.randomSummaryRecord()

      var sentences = tc.briefPresentedSentences(summary, generating)
      var body = tc.mirrorSummaryShort(summary)

      verify(sentences.length > 0,
             "iteration " + i + ": the brief surface presented nothing at all")

      for (var s = 0; s < sentences.length; s++) {
        var role = sentences[s].role
        var text = sentences[s].text

        // Every sentence traces back to the record or to the allowlist.
        var provenance = tc.briefProvenanceOf(text, summary)
        verify(provenance !== "",
               "iteration " + i + ": the " + role + " sentence has no provenance in "
               + "the Summary_Record and is not an allowlisted panel string: "
               + JSON.stringify(text))

        // And uses no word of its own invention.
        var stray = tc.briefUntraceableWord(text, summary)
        verify(stray === "",
               "iteration " + i + ": the " + role + " sentence invented the word "
               + JSON.stringify(stray) + " in " + JSON.stringify(text))

        // Glyph-only indicators are not sentences and never leak into one.
        for (var g = 0; g < tc.nonSentenceGlyphs.length; g++) {
          verify(text.indexOf(tc.nonSentenceGlyphs[g]) === -1,
                 "iteration " + i + ": the " + role
                 + " sentence carries a bare indicator glyph: " + JSON.stringify(text))
        }

        // With no record at all, nothing record-shaped can be presented.
        if (!summary)
          verify(provenance === "fixed-panel-string",
                 "iteration " + i + ": content was presented with no Summary_Record: "
                 + JSON.stringify(text))
      }

      // The body sentences, when present, are exactly the normalised record
      // text — the ticker and the expanded view read the same record.
      for (var b = 0; b < sentences.length; b++) {
        if (sentences[b].role === "ticker" || sentences[b].role === "expanded")
          verify(sentences[b].text === body,
                 "iteration " + i + ": the " + sentences[b].role
                 + " sentence is not the normalised record text")
      }
      verify((body !== "") === (sentences.length >= 3 && sentences[1].role === "ticker"),
             "iteration " + i + ": body sentences appear without record text")
    }

    // Negative control: the checker must reject interpretation. If these ever
    // pass, Property 5 has stopped being able to fail.
    var control = { text: "alpha beta", status: "stale", message: "" }
    verify(tc.briefProvenanceOf("Outdated \u2014 the workspace looks quiet, rerun this", control) === "",
           "control: interpretive prose was accepted as a legal sentence")
    verify(tc.briefProvenanceOf("alpha beta \u2014 looks calm to me", control) === "",
           "control: record text with an appended opinion was accepted")
    verify(tc.briefProvenanceOf("Everything is fine", control) === "",
           "control: an invented sentence was accepted")
    verify(tc.briefProvenanceOf("Outdated", control) === "fixed-panel-string",
           "control: a legal fixed panel string was rejected")
    verify(tc.briefProvenanceOf("alpha beta", control) === "record-text",
           "control: the normalised record text was rejected")
    verify(tc.briefUntraceableWord("Outdated \u2014 the workspace looks quiet", control) !== "",
           "control: the word backstop missed an interpretive clause")
    verify(tc.briefUntraceableWord("alpha beta", control) === "",
           "control: the word backstop rejected the record's own words")
    verify(tc.briefUntraceableWord("Unavailable \u2014 alpha", control) === "",
           "control: the word backstop rejected an allowlisted prefix")
  }

  // **Validates: Requirements 18.2, 18.3**
  // Feature: pitwall-chat-and-brief-ticker, Property 25: Chat entry labels carry every displayed fact — For any chat object in `state.json`, the rail entry label contains the literal `Pitwall Chat`, the Chat_Number, the Harness, the Model (or the agent-default label) and the Context_Label, and the entry's state word comes from the existing session-state vocabulary.
  function test_property25_chat_entry_labels_carry_every_displayed_fact() {
    tc.seed(20252505)
    for (var i = 0; i < tc.iterations; i++) {
      var session
      var selected = (i % 2) === 0
      if (i === 0) {
        // Not a chat object at all: no chat entry exists, so there is no label
        // to check. (Malformed-shape degradation is Property 24's subject.)
        session = { id: tc.randomSessionId(), state: "running", chat: null }
        verify(tc.mirrorChatEntryLabel(session, selected) === "",
               "iteration 0: a non-chat session produced a chat entry label")
        continue
      }
      else if (i === 1) {
        session = tc.randomChatSession()
        session.chat.model = ""            // agent default
        session.state = "running"
      }
      else if (i === 2) {
        session = tc.randomChatSession()
        session.chat.context_label = ""    // no context label in the artifact
        session.state = "sleeping"
      }
      else if (i === 3) {
        session = tc.randomChatSession()
        session.chat.number = "001"
        session.chat.harness = "opencode"
        session.chat.model = "muse-spark"
        session.chat.context_label = "Work"
        session.state = "stopped"
      }
      else if (i === 4) {
        session = tc.randomChatSession()
        session.state = "zombie"           // outside the vocabulary
      }
      else session = tc.randomChatSession()

      var chat = tc.mirrorChatOf(session)
      verify(chat !== null,
             "iteration " + i + ": the generated chat object was rejected by chatOf")

      var label = tc.mirrorChatEntryLabel(session, selected)
      verify(label !== "",
             "iteration " + i + ": a valid chat object produced no entry label")

      // The literal, and every displayed fact, present in the one label.
      verify(label.indexOf(tc.chatLiteral) !== -1,
             "iteration " + i + ": label is missing the literal " + tc.chatLiteral
             + ": " + JSON.stringify(label))
      verify(label.indexOf(chat.number) !== -1,
             "iteration " + i + ": label is missing the Chat_Number " + chat.number
             + ": " + JSON.stringify(label))
      verify(label.indexOf(tc.chatLiteral + " " + chat.number) !== -1,
             "iteration " + i + ": the Chat_Number is not bound to the literal: "
             + JSON.stringify(label))
      verify(label.indexOf(chat.harness) !== -1,
             "iteration " + i + ": label is missing the Harness "
             + JSON.stringify(chat.harness) + ": " + JSON.stringify(label))
      verify(label.indexOf(chat.model_label) !== -1,
             "iteration " + i + ": label is missing the Model "
             + JSON.stringify(chat.model_label) + ": " + JSON.stringify(label))
      if (chat.model === "")
        verify(label.indexOf("agent default") !== -1,
               "iteration " + i + ": an empty Model did not render the agent-default "
               + "label: " + JSON.stringify(label))
      else
        verify(label.indexOf(chat.model) !== -1,
               "iteration " + i + ": label is missing the Model " + chat.model)
      if (chat.context_label !== "")
        verify(label.indexOf(chat.context_label) !== -1,
               "iteration " + i + ": label is missing the Context_Label "
               + JSON.stringify(chat.context_label) + ": " + JSON.stringify(label))
      // No dangling separator, whatever the empty facts are.
      verify(label.indexOf(tc.chatSeparator + tc.chatSeparator) === -1,
             "iteration " + i + ": a dropped fact left a doubled separator: "
             + JSON.stringify(label))

      // The state word comes from the existing panel vocabulary, and its key is
      // the same fold StateReader.stateOf performs on the session's own state.
      var word = tc.mirrorChatEntryStateWord(session)
      var key = tc.mirrorStateKey(session)
      verify(tc.stateVocabulary.indexOf(word) !== -1,
             "iteration " + i + ": state word " + JSON.stringify(word)
             + " is outside the session-state vocabulary")
      verify(label.indexOf(word) !== -1,
             "iteration " + i + ": label is missing the state word "
             + JSON.stringify(word) + ": " + JSON.stringify(label))
      verify(key === tc.mirrorStateOf(session),
             "iteration " + i + ": the entry's state key disagrees with "
             + "StateReader.stateOf")
      if (session.state === "running" || session.state === "sleeping"
          || session.state === "stopped")
        verify(key === session.state,
               "iteration " + i + ": an observed state was not carried verbatim")
      else
        verify(key === "unknown",
               "iteration " + i + ": an unrecognised state did not fold to unknown")
      verify(word === tc.mirrorStateWord(key),
             "iteration " + i + ": the state word is not the vocabulary word for "
             + "the observed state")

      // Displayed facts only: the label carries no identifier and no raw
      // timestamp from the chat object (22.7).
      if (chat.context_session_id !== null)
        verify(label.indexOf(chat.context_session_id) === -1,
               "iteration " + i + ": the label leaked the context session id")
      verify(label.indexOf(session.id) === -1,
             "iteration " + i + ": the label leaked the session id")
      var epoch = String(chat.started_at)
      if (chat.harness.indexOf(epoch) === -1 && chat.model_label.indexOf(epoch) === -1
          && chat.context_label.indexOf(epoch) === -1)
        verify(label.indexOf(epoch) === -1,
               "iteration " + i + ": the label carried the raw start epoch")
    }
  }

  // Which of the three legal outcomes the property allows for a given pinned
  // selection. This classifies; the assertions below check the *shape* of what
  // came out, which is the substance of the law. Kept separate so the branch
  // coverage check at the end of the test can prove all three were reached.
  function expectedOpenChatBranch(selectedId) {
    var pinned = String(selectedId || "")
    if (pinned === "") return "workspace"                   // 8.3
    if (pinned.charAt(0) === "r") return "workspace"        // resumable is not live (8.3)
    if (/^sess_[0-9a-f]{16}$/.test(pinned)) return "session" // 8.2
    return "no-launch"                                      // 8.6
  }

  // **Validates: Requirements 8.2, 8.3, 8.4, 8.6**
  // Feature: pitwall-chat-and-brief-ticker, Property 6: Open Chat argv is fixed and shape-gated — For any pinned-selection value, the Open_Chat_Control either produces the argv `[pitwall, "chat"]`, or `[pitwall, "chat", "--session", id]` when the selection is a live session id matching `sess_[0-9a-f]{16}`, or produces no launch at all — never a longer vector, never a shell string, never a caller-supplied flag.
  function test_property6_open_chat_argv_is_fixed_and_shape_gated() {
    tc.seed(20250606)
    var seenWorkspaceEmpty = 0
    var seenWorkspaceResumable = 0
    var seenSession = 0
    var seenNoLaunch = 0
    var seenNoLaunchWithMetacharacter = 0

    for (var i = 0; i < tc.iterations; i++) {
      // Forced corner cases first, generated selections afterwards.
      var sel
      if (i === 0) sel = ""                                   // nothing pinned
      else if (i === 1) sel = null                            // empty-ish
      else if (i === 2) sel = undefined                       // empty-ish
      else if (i === 3) sel = false                           // empty-ish
      else if (i === 4) sel = 0                               // empty-ish
      else if (i === 5) sel = "   "                           // whitespace only
      else if (i === 6) sel = "sess_0123456789abcdef"          // the valid shape
      else if (i === 7) sel = "r:" + tc.randomHexRun(16)       // pinned resumable
      else if (i === 8) sel = "r" + tc.randomHexRun(16)        // bare r-prefixed
      else if (i === 9) sel = "sess_0123456789ABCDEF"          // uppercase hex
      else if (i === 10) sel = "--session"                     // a bare flag
      else if (i === 11) sel = "-sess_0123456789abcdef"         // leading dash
      else if (i === 12) sel = "sess_0123456789abcdef; rm -rf /" // shell injection
      else if (i === 13) sel = "sess_0123456789abcdef\n--session sess_0123456789abcdef"
      else if (i === 14) sel = "$(id)"                          // command substitution
      else if (i === 15) sel = "`id`"                           // command substitution
      else if (i === 16) sel = "sess_0123456789abcde"           // 15 hex digits
      else if (i === 17) sel = "sess_0123456789abcdef0"         // 17 hex digits
      else {
        var roll = tc.nextUnit()
        if (roll < 0.35) sel = tc.randomSessionId()
        else if (roll < 0.55) sel = tc.randomResumableSelection()
        else if (roll < 0.60) sel = ""
        else sel = tc.randomNearMissSelection()
      }

      var pinned = String(sel || "")
      var branch = tc.expectedOpenChatBranch(sel)
      var res = tc.mirrorOpenChat(sel)

      if (branch === "no-launch") {
        // Produces no launch at all: no argv exists, and the refusal is
        // announced as needing attention (8.6).
        seenNoLaunch++
        if (tc.containsShellMetacharacter(pinned) !== "")
          seenNoLaunchWithMetacharacter++
        verify(res.launched === false,
               "iteration " + i + ": a launch happened for the invalid selection "
               + JSON.stringify(pinned))
        verify(res.argv === null,
               "iteration " + i + ": a refused activation still produced an argv: "
               + JSON.stringify(res.argv))
        verify(res.refused === true,
               "iteration " + i + ": the invalid selection was not refused: "
               + JSON.stringify(pinned))
        verify(res.attention === true,
               "iteration " + i + ": the refusal was not marked as needing attention")
        verify(res.announcement === "Could not open chat: invalid session.",
               "iteration " + i + ": the refusal presented no refusal message; got "
               + JSON.stringify(res.announcement))
        continue
      }

      // Every other selection launches, and the vector is one of exactly two
      // shapes.
      verify(res.launched === true,
             "iteration " + i + ": no launch for the legal selection "
             + JSON.stringify(pinned))
      var argv = res.argv

      // Never a shell string: argv is a vector the process API takes verbatim.
      verify(argv !== null && argv !== undefined,
             "iteration " + i + ": a launch happened with no argv")
      verify(Array.isArray(argv),
             "iteration " + i + ": argv is not a vector: " + JSON.stringify(argv))
      verify(typeof argv !== "string",
             "iteration " + i + ": argv is a shell string: " + JSON.stringify(argv))

      // Never a longer vector.
      verify(argv.length === 2 || argv.length === 4,
             "iteration " + i + ": argv length " + argv.length
             + " is neither 2 nor 4: " + JSON.stringify(argv))

      // Element 0 is the Pitwall binary, absolute, and nothing else.
      verify(argv[0] === tc.pitwallBinaryStub,
             "iteration " + i + ": argv[0] is not the pitwall binary: "
             + JSON.stringify(argv[0]))
      verify(argv[0].charAt(0) === "/",
             "iteration " + i + ": argv[0] is not an absolute path: "
             + JSON.stringify(argv[0]))
      // Never a shell as the program: the vector is executed directly.
      verify(argv[0].slice(-8) === "/pitwall",
             "iteration " + i + ": argv[0] is not the pitwall executable: "
             + JSON.stringify(argv[0]))

      // Element 1 is exactly the subcommand.
      verify(argv[1] === "chat",
             "iteration " + i + ": argv[1] is not exactly \"chat\": "
             + JSON.stringify(argv[1]))

      if (branch === "workspace") {
        if (pinned === "") seenWorkspaceEmpty++
        else seenWorkspaceResumable++
        // Whole-workspace context: the two-element vector, and the pinned value
        // (a resumable id or nothing at all) never reaches the command line.
        verify(argv.length === 2,
               "iteration " + i + ": workspace context produced a "
               + argv.length + "-element vector: " + JSON.stringify(argv))
        if (pinned !== "")
          verify(argv.indexOf(pinned) === -1,
                 "iteration " + i + ": a non-live pinned selection reached argv: "
                 + JSON.stringify(argv))
        verify(argv.indexOf("--session") === -1,
               "iteration " + i + ": workspace context carried a --session flag: "
               + JSON.stringify(argv))
      } else {
        seenSession++
        // Pinned live session: exactly four elements, the flag fixed, the id
        // verbatim, and the id genuinely of the gated shape.
        verify(argv.length === 4,
               "iteration " + i + ": session context produced a "
               + argv.length + "-element vector: " + JSON.stringify(argv))
        verify(argv[2] === "--session",
               "iteration " + i + ": argv[2] is not exactly \"--session\": "
               + JSON.stringify(argv[2]))
        verify(argv[3] === pinned,
               "iteration " + i + ": argv[3] is not the pinned id verbatim; got "
               + JSON.stringify(argv[3]) + " want " + JSON.stringify(pinned))
        verify(/^sess_[0-9a-f]{16}$/.test(argv[3]),
               "iteration " + i + ": argv[3] is not a gated session id: "
               + JSON.stringify(argv[3]))
      }

      // Never a caller-supplied flag: every element is one of the three fixed
      // literals or the gated id in position 3, and only "--session" may look
      // like an option.
      var flagCount = 0
      for (var e = 0; e < argv.length; e++) {
        var el = argv[e]
        verify(typeof el === "string",
               "iteration " + i + ": argv[" + e + "] is not a string: "
               + JSON.stringify(el))
        var fixed = (e === 0 && el === tc.pitwallBinaryStub)
          || (e === 1 && el === "chat")
          || (e === 2 && el === "--session")
          || (e === 3 && el === pinned)
        verify(fixed,
               "iteration " + i + ": argv[" + e + "] is not a fixed element: "
               + JSON.stringify(el) + " in " + JSON.stringify(argv))
        if (el.charAt(0) === "-") {
          flagCount++
          verify(el === "--session",
                 "iteration " + i + ": argv carries the caller-supplied flag "
                 + JSON.stringify(el) + ": " + JSON.stringify(argv))
        }
        verify(el !== "-c",
               "iteration " + i + ": argv carries a shell command flag: "
               + JSON.stringify(argv))
        // No element carries a character a shell would act on — neither one the
        // panel introduced nor one it let through from the selection.
        var meta = tc.containsShellMetacharacter(el)
        verify(meta === "",
               "iteration " + i + ": argv[" + e + "] carries the shell "
               + "metacharacter " + JSON.stringify(meta) + ": " + JSON.stringify(argv))
      }
      verify(flagCount === (argv.length === 4 ? 1 : 0),
             "iteration " + i + ": argv carries " + flagCount
             + " option-shaped elements: " + JSON.stringify(argv))

      // A launch is never announced as needing attention at construction time;
      // failures are the exit-code path, not the argv path.
      verify(res.attention === false,
             "iteration " + i + ": a constructed launch was marked as a failure")
    }

    // Negative control: all three branches of the property were actually
    // exercised, so the test cannot pass by never launching (or never refusing).
    verify(seenWorkspaceEmpty > 0, "no unpinned selection was generated")
    verify(seenWorkspaceResumable > 0, "no pinned resumable selection was generated")
    verify(seenSession > 0, "no valid pinned session id was generated")
    verify(seenNoLaunch > 0, "no refusable selection was generated")
    verify(seenNoLaunchWithMetacharacter > 0,
           "no refused selection carried a shell metacharacter")
  }

  // **Validates: Requirements 22.5, 22.6**
  // Feature: pitwall-chat-and-brief-ticker, Property 24: Malformed chat fields degrade without collateral damage — For any value substituted for a session's `chat` key that is not a well-formed chat object, the reader yields no chat entry for that session while still parsing sessions, resumable entries, summary, notifications and unread count; and a document with no `chat` keys yields no chat region at all.
  function test_property24_malformed_chat_fields_degrade_without_collateral_damage() {
    tc.seed(20252404)
    var kindsSeen = {}
    var kindsSeenCount = 0
    var controlsSeen = 0

    for (var i = 0; i < tc.iterations; i++) {
      // Every malformed kind forced first, two controls next, then a mix.
      var isControl
      var kind = -1
      if (i < tc.malformedChatKindCount) { isControl = false; kind = i }
      else if (i === tc.malformedChatKindCount || i === tc.malformedChatKindCount + 1)
        isControl = true
      else {
        isControl = tc.nextUnit() < 0.2
        if (!isControl) kind = tc.nextInt(tc.malformedChatKindCount)
      }

      var chatValue = isControl ? tc.randomChatSession().chat : tc.malformedChatValue(kind)
      if (isControl) controlsSeen++
      else if (kindsSeen[kind] !== true) { kindsSeen[kind] = true; kindsSeenCount++ }

      var label = isControl ? "control" : ("malformed \"" + tc.malformedChatKindName(kind) + "\"")
      var bystanders = tc.nextInt(3)
      var built = tc.buildStateDocument(chatValue, true, bystanders)
      var doc = built.doc
      var victim = built.victim

      // The document still parses: a malformed per-session value never costs the
      // whole record (22.6).
      verify(tc.mirrorParseAccepts(doc) === true,
             "iteration " + i + " (" + label + "): the document stopped parsing")

      // No chat entry for that session — or, for the control, one.
      var chat = tc.mirrorChatOf(victim)
      if (isControl)
        verify(chat !== null,
               "iteration " + i + ": a well-formed chat value yielded no chat entry")
      else
        verify(chat === null,
               "iteration " + i + " (" + label + "): chatOf accepted a malformed value: "
               + JSON.stringify(chatValue))

      // Sessions still parse, and the affected session is still an ordinary
      // session with its own fields intact (22.6).
      var sessions = tc.mirrorRecordSessions(doc)
      verify(sessions.length === doc.sessions.length,
             "iteration " + i + " (" + label + "): sessions were dropped; got "
             + sessions.length + " want " + doc.sessions.length)
      verify(sessions[built.victimIndex] === victim,
             "iteration " + i + " (" + label + "): the affected session left the "
             + "sessions list")
      verify(tc.stateVocabulary.indexOf(tc.mirrorStateWord(tc.mirrorStateOf(victim))) !== -1,
             "iteration " + i + " (" + label + "): the affected session lost its state")
      verify(tc.mirrorStateOf(victim) === victim.state,
             "iteration " + i + " (" + label + "): the affected session's state changed")
      var wantLabel = victim.project.name
        + (victim.project.branch !== "" ? ":" + victim.project.branch : "")
      verify(tc.mirrorSessionLabel(victim) === wantLabel,
             "iteration " + i + " (" + label + "): the affected session's label changed; got "
             + JSON.stringify(tc.mirrorSessionLabel(victim)) + " want "
             + JSON.stringify(wantLabel))
      verify(victim.id === sessions[built.victimIndex].id,
             "iteration " + i + " (" + label + "): the affected session's id changed")

      // Resumable entries still parse, entry for entry.
      var resumable = tc.mirrorRecordResumable(doc)
      verify(resumable.length === doc.resumable.length,
             "iteration " + i + " (" + label + "): resumable entries were dropped")
      for (var r = 0; r < resumable.length; r++)
        verify(resumable[r].id === doc.resumable[r].id,
               "iteration " + i + " (" + label + "): resumable entry " + r + " changed")

      // Summary still parses, field for field.
      var summary = tc.mirrorRecordSummary(doc)
      verify(summary !== null,
             "iteration " + i + " (" + label + "): the summary stopped parsing")
      verify(summary.status === doc.summary.status,
             "iteration " + i + " (" + label + "): the summary status changed")
      verify(summary.text === String(doc.summary.text),
             "iteration " + i + " (" + label + "): the summary text changed")
      verify(summary.message === String(doc.summary.message || ""),
             "iteration " + i + " (" + label + "): the summary message changed")
      verify(summary.created_at === doc.summary.created_at,
             "iteration " + i + " (" + label + "): the summary timestamp changed")
      verify(summary.input_hash === doc.summary.input_hash,
             "iteration " + i + " (" + label + "): the summary input hash changed")

      // Notifications and the unread count still parse.
      var notifications = tc.mirrorRecordNotifications(doc)
      verify(notifications.length === doc.notifications.length,
             "iteration " + i + " (" + label + "): notifications were dropped")
      for (var n = 0; n < notifications.length; n++)
        verify(notifications[n].id === doc.notifications[n].id,
               "iteration " + i + " (" + label + "): notification " + n + " changed")
      verify(tc.mirrorRecordUnreadCount(doc) === doc.notifications.length,
             "iteration " + i + " (" + label + "): the unread count changed; got "
             + tc.mirrorRecordUnreadCount(doc) + " want " + doc.notifications.length)

      // The footer version still parses.
      verify(tc.mirrorRecordPitwallVersion(doc) === doc.pitwall_version,
             "iteration " + i + " (" + label + "): the pitwall version changed")

      // The chat region carries every well-formed chat session and nothing else:
      // the malformed value cost only its own entry.
      var chatRows = tc.mirrorChatSessions(sessions)
      var wantRows = bystanders + (isControl ? 1 : 0)
      verify(chatRows.length === wantRows,
             "iteration " + i + " (" + label + "): the chat region holds "
             + chatRows.length + " entries, want " + wantRows)
      verify((chatRows.indexOf(victim) !== -1) === isControl,
             "iteration " + i + " (" + label + "): the affected session's presence in "
             + "the chat region is wrong")
      var previous = -1
      for (var c = 0; c < chatRows.length; c++) {
        var facts = tc.mirrorChatOf(chatRows[c])
        verify(facts !== null,
               "iteration " + i + " (" + label + "): the chat region holds a session "
               + "with no chat facts")
        verify(Number(facts.number) >= previous,
               "iteration " + i + " (" + label + "): the chat region is not ordered "
               + "by chat number")
        previous = Number(facts.number)
      }

      // The same document through the JSON path the reader actually uses. An
      // `undefined` value drops its key here, which is the absent-key case, and
      // it must degrade the same way.
      var rt = JSON.parse(JSON.stringify(doc))
      verify(tc.mirrorParseAccepts(rt) === true,
             "iteration " + i + " (" + label + "): the serialised document stopped parsing")
      var rtVictim = tc.mirrorRecordSessions(rt)[built.victimIndex]
      verify((tc.mirrorChatOf(rtVictim) !== null) === isControl,
             "iteration " + i + " (" + label + "): the serialised document disagrees "
             + "about the affected session's chat entry")
      verify(tc.mirrorChatSessions(tc.mirrorRecordSessions(rt)).length === wantRows,
             "iteration " + i + " (" + label + "): the serialised chat region size changed")
      verify(tc.mirrorRecordUnreadCount(rt) === doc.notifications.length,
             "iteration " + i + " (" + label + "): the serialised unread count changed")
      verify(tc.mirrorRecordSummary(rt) !== null
             && tc.mirrorRecordSummary(rt).status === doc.summary.status,
             "iteration " + i + " (" + label + "): the serialised summary changed")
    }

    // Every malformed kind and both controls were exercised, so the property
    // cannot pass by accepting or rejecting everything.
    verify(kindsSeenCount === tc.malformedChatKindCount,
           "only " + kindsSeenCount + " of " + tc.malformedChatKindCount
           + " malformed chat shapes were generated")
    verify(controlsSeen > 0, "no well-formed control chat object was generated")

    // The two JavaScript traps the reader's explicit guards exist for. Both are
    // asserted here so the guards cannot be dropped without this test noticing.
    var trapArray = tc.malformedChatValue(23)
    verify(Array.isArray(trapArray),
           "trap: the array-shaped chat value is not an array")
    verify(typeof trapArray === "object",
           "trap: `typeof array` is no longer \"object\", so the Array.isArray "
           + "guard is testing the wrong thing")
    verify(typeof trapArray.number === "string" && /^[0-9]{3}$/.test(trapArray.number),
           "trap: the array-shaped chat value fails the number gate, so it never "
           + "reaches the Array.isArray guard")
    verify(typeof trapArray.harness === "string" && trapArray.harness.trim().length > 0,
           "trap: the array-shaped chat value fails the harness gate, so it never "
           + "reaches the Array.isArray guard")
    verify(tc.mirrorChatOf({ chat: trapArray }) === null,
           "trap: an array carrying well-formed chat fields was accepted as a "
           + "chat object")
    verify(typeof null === "object",
           "trap: `typeof null` is no longer \"object\", so the explicit null "
           + "guard is testing the wrong thing")
    verify(tc.mirrorChatOf({ chat: null }) === null,
           "trap: a null chat value was accepted as a chat object")
    // And the same two traps at the session level, where `chatOf` guards its
    // own argument with the identical pair of checks.
    verify(tc.mirrorChatOf(null) === null,
           "trap: a null session was accepted")
    verify(tc.mirrorChatOf([]) === null,
           "trap: an array session was accepted")

    // A document with no `chat` keys at all yields no chat region (22.5), while
    // everything else still reads.
    for (var d = 0; d < 8; d++) {
      var plain = tc.buildStateDocument(null, false, 0)
      var plainSessions = tc.mirrorRecordSessions(plain.doc)
      for (var p = 0; p < plainSessions.length; p++)
        verify(plainSessions[p].chat === undefined,
               "document " + d + ": the no-chat-key document carried a chat key")
      verify(tc.mirrorParseAccepts(plain.doc) === true,
             "document " + d + ": the no-chat-key document did not parse")
      verify(tc.mirrorChatSessions(plainSessions).length === 0,
             "document " + d + ": a document with no chat keys still produced a "
             + "chat region")
      verify(plainSessions.length === plain.doc.sessions.length,
             "document " + d + ": sessions were dropped from the no-chat-key document")
      verify(tc.mirrorRecordResumable(plain.doc).length === plain.doc.resumable.length,
             "document " + d + ": resumable entries were dropped")
      verify(tc.mirrorRecordSummary(plain.doc) !== null,
             "document " + d + ": the summary stopped parsing")
      verify(tc.mirrorRecordNotifications(plain.doc).length === plain.doc.notifications.length,
             "document " + d + ": notifications were dropped")
      verify(tc.mirrorRecordUnreadCount(plain.doc) === plain.doc.notifications.length,
             "document " + d + ": the unread count changed")
      verify(tc.mirrorRecordPitwallVersion(plain.doc) === plain.doc.pitwall_version,
             "document " + d + ": the pitwall version changed")

      var plainRt = JSON.parse(JSON.stringify(plain.doc))
      verify(tc.mirrorChatSessions(tc.mirrorRecordSessions(plainRt)).length === 0,
             "document " + d + ": the serialised no-chat-key document produced a "
             + "chat region")
    }

    // A document with no sessions at all also has no chat region.
    var empty = { state_version: 4, sessions: [], collected_at: 1700000000 }
    verify(tc.mirrorParseAccepts(empty) === true,
           "control: an empty-session document did not parse")
    verify(tc.mirrorChatSessions(tc.mirrorRecordSessions(empty)).length === 0,
           "control: an empty-session document produced a chat region")
  }
}
