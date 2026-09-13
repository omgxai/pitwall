import QtQuick
import QtTest

// Qt Quick Test coverage for the AI_Brief_Ticker laws of M8 Part A
// (spec: .kiro/specs/pitwall-chat-and-brief-ticker, design §4.11).
//
// IMPORTANT — MIRRORED FORMULAS, KEEP IN SYNC WITH Widget.qml
//
// `Widget.qml` is a Quickshell bar widget: its root is `Panel` and it imports
// `Quickshell`, `Quickshell.Io`, `Quickshell.Wayland`, `qs.Commons` and `qs.Ui`
// (Style/Color tokens). None of those modules resolve under a bare
// `qmltestrunner`, so the component cannot be instantiated here and its
// properties cannot be driven directly.
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
// QML has no property-testing library, so each test drives a deterministic
// seeded PRNG over generated geometry and text for well over the required 100
// iterations. Seeds are fixed so a failure is reproducible.
//
// EXECUTION STATUS: NOT RUN as of task 3.3. Gate 1.4 recorded that neither
// `qmltestrunner` nor `qmllint` is installed on the development machine, so
// this file has never been executed and must not be reported as passing. It is
// committed so that a Qt-equipped machine can run it with
// `qmltestrunner -input plugin/dev.pitwall/tests/tst_ticker.qml`. The mirrored
// formulas and every assertion below were checked by transcribing them into a
// throwaway plain-JavaScript harness, including a negative control confirming
// that Property 3 fails against the old `max(700, …)` duration floor; that
// check exercises the logic, not the QML runtime.

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
}
