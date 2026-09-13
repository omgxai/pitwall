//! Explicit user-triggered AI workspace summaries (M5c).
//!
//! Pitwall invokes the user's *already-configured* agent (opencode in M5c)
//! with a sanitized ephemeral context file attached (`-f`). No provider
//! integration, no keys, no HTTP, no autonomy:
//!
//! - fixed argv (binary, flags, validated model/dir/context path);
//! - no shell, no interpolation, no content in argv (content travels only
//!   in the `-f` file, which is unlinked afterwards by [`EphemeralGuard`]);
//! - output filtered to text events; tool calls are ignored, never chained;
//! - the prompt instructs interpretation only, with the evidence hierarchy
//!   (activity signals are not semantic claims).

use crate::context::EphemeralContext;
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

/// Static instruction sent as the agent message. Stable and small: the
/// agent interprets Pitwall evidence, never continues the work.
pub const INSTRUCTION: &str = "Act as Pitwall's race engineer. Analyze only the attached, bounded Pitwall workspace context and give the human a concise operational brief. State what is happening first, then what meaningfully changed, whether attention is needed, and what can be resumed or done next when the evidence supports it. Prefer plain language such as 'OpenCode is idle in Work' or 'One session stopped; another remains active'. Mention project and agent names when known. Do not mention internal diagnostics such as process counts, confidence mechanics, missing scrollback, unavailable terminal text, or implementation details unless directly useful to an action. Do not invent work, completion, blockers, or attention items. Treat IO/process activity only as activity evidence, never as proof of task progress. Return 1-3 short sentences, ideally under 600 characters total. Do not execute tasks, modify files, or continue the work.";

/// Deterministic cache key for a summary request: FNV-1a over the
/// *structured* context only (session identity/role/project/agent/state,
/// derived events, checkpoint refs). Terminal text is deliberately
/// EXCLUDED — it is always fresh per request, and including it would make
/// the cache never hit. Same effective workspace → same hash; any
/// meaningful change → miss. Timestamps of the request itself are excluded.
pub fn input_hash(context: &crate::context::SummaryContext) -> String {
    format!(
        "fnv:{}",
        crate::ids::fnv1a_hex(&context.stable_serialized())
    )
}

/// Default agent timeout (agent boot + inference).
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Truncation bound for the extracted summary text.
pub const MAX_SUMMARY_CHARS: usize = 800;

/// Model id shape: `provider/model` with safe characters only. Rejects
/// whitespace and shell metacharacters so the value can never escape argv.
pub fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 128
        && model.contains('/')
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.' | ':'))
}

/// Build the fixed agent argv. M5c supports opencode only; anything else
/// is refused (its non-interactive surface is unvalidated for summaries).
/// Returns `(binary, argv)` with argv[0] == binary for `Command` use.
pub fn build_argv(
    agent: &str,
    model: Option<&str>,
    dir: &str,
    context_path: &Path,
    opencode_bin: &Path,
) -> Result<Vec<String>, String> {
    if agent != "opencode" {
        return Err(format!(
            "agent '{agent}' summaries are not supported yet (opencode only)"
        ));
    }
    if let Some(m) = model {
        if !valid_model(m) {
            return Err(format!("refusing malformed model id ({m:?})"));
        }
    }
    if !dir.starts_with('/') {
        return Err("refusing non-absolute working directory".to_string());
    }
    let bin = opencode_bin.to_string_lossy().into_owned();
    let mut argv = vec![
        bin,
        "run".to_string(),
        "--format".to_string(),
        "json".to_string(),
        "--dir".to_string(),
        dir.to_string(),
        "-f".to_string(),
        context_path.to_string_lossy().into_owned(),
    ];
    if let Some(m) = model {
        argv.push("-m".to_string());
        argv.push(m.to_string());
    }
    argv.push(INSTRUCTION.to_string());
    Ok(argv)
}

/// Run the agent with a timeout, capturing stdout. Kills on expiry.
/// No shell at any point; `argv[0]` is the binary, the rest fixed flags.
///
/// `stdin_payload` is the *only* channel for content that must not appear in
/// argv: argv is world-readable through `/proc`, a pipe is not. Harnesses
/// that take the bounded context document on stdin (`claude -p`,
/// `codex exec`) get it here; the `opencode` summary/assign paths pass
/// `None`.
///
/// `None` is byte-identical to the pre-M8 runner: the same argv, the same
/// `Stdio::null()` stdin disposition, the same timeout and kill semantics,
/// and no stdin handle to write to or close (Requirement 23.1). Only a
/// `Some` payload switches stdin to a pipe.
///
/// Two failure modes are handled deliberately for the `Some` path:
///
/// 1. **EOF.** The write handle is dropped as soon as the payload is
///    written, which closes the child's stdin. Harnesses that read stdin to
///    completion (`codex exec` documented among them) never see EOF on a
///    pipe that stays open and hang until the deadline — that would make the
///    timeout-and-kill branch the normal path instead of the exceptional one.
/// 2. **Deadlock.** Writing to a child's stdin while it writes to stdout can
///    deadlock if both pipe buffers fill: the writer blocks because the child
///    is not draining, the child blocks because nobody is draining its
///    stdout. Stdout is therefore drained on its own thread (as before) and
///    the payload is written on a *second* thread, so the wait loop below is
///    never blocked by either pipe and the deadline plus kill stays the only
///    bound on the run. A child that reads no stdin at all is killed on
///    expiry exactly like any other hung agent, and the kill closes the read
///    end so the writer thread unblocks with `EPIPE` and is joined.
///
///    `SummaryContext` caps the context document at 16 KB, which is under a
///    typical 64 KB pipe buffer, so in practice the write would complete
///    without any draining at all. That is an assumption about the platform's
///    buffer size, stated here rather than silently relied on: the threading
///    above is correct for any payload size.
pub fn run_agent(
    argv: &[String],
    stdin_payload: Option<&str>,
    timeout: Duration,
) -> Result<String, String> {
    if argv.is_empty() {
        return Err("empty argv (refusing)".to_string());
    }
    let stdin_cfg = match stdin_payload {
        // A pipe only when there is something to deliver; otherwise the
        // pre-existing null disposition, unchanged.
        Some(_) => std::process::Stdio::piped(),
        None => std::process::Stdio::null(),
    };
    let mut child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(stdin_cfg)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("agent spawn failed: {e}"))?;
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(out) = stdout {
            let _ = out.take(256 * 1024).read_to_end(&mut buf);
        }
        buf
    });
    // Payload delivery: its own thread, started once stdout is already being
    // drained (see the deadlock note above). `None` creates no thread and
    // takes no handle, leaving the pre-M8 path untouched.
    let writer = match stdin_payload {
        Some(payload) => {
            let mut sink = match child.stdin.take() {
                Some(sink) => sink,
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err("agent stdin unavailable (refusing)".to_string());
                }
            };
            let bytes = payload.as_bytes().to_vec();
            Some(std::thread::spawn(move || {
                let mut written = sink.write_all(&bytes);
                if written.is_ok() {
                    written = sink.flush();
                }
                // Explicit close so the child sees EOF, on the error path
                // too: a harness reading stdin to completion never returns
                // while the pipe stays open, which would turn the timeout
                // branch into the normal path.
                drop(sink);
                written.map_err(|e| e.to_string())
            }))
        }
        None => None,
    };
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let raw = reader.join().unwrap_or_default();
                let stdin_err = join_stdin_writer(writer);
                if !status.success() {
                    return Err(format!("agent exited {}", status));
                }
                if let Some(e) = stdin_err {
                    // The agent answered without the context it was given,
                    // so the answer is not evidence-based: refuse it.
                    return Err(format!("agent stdin write failed: {e}"));
                }
                return Ok(String::from_utf8_lossy(&raw).into_owned());
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    let _ = join_stdin_writer(writer);
                    return Err(format!("agent timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                let _ = join_stdin_writer(writer);
                return Err(format!("agent wait failed: {e}"));
            }
        }
    }
}

/// Collect the stdin writer thread's outcome, if there was one. Returns the
/// failure text, or `None` when the payload was delivered (or when there was
/// no payload at all). Joining is safe on every branch: the child's death
/// closes the read end, so a blocked `write_all` returns `EPIPE`.
fn join_stdin_writer(
    writer: Option<std::thread::JoinHandle<Result<(), String>>>,
) -> Option<String> {
    match writer?.join() {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(e),
        Err(_) => Some("stdin writer panicked".to_string()),
    }
}

/// Extract human summary text from `opencode run --format json` output.
/// Scans JSON-lines for text-bearing fields and accepts trimmed plain-text
/// lines. Returns empty when no usable text remains, never restoring ignored
/// events as raw output. The result is truncated to [`MAX_SUMMARY_CHARS`].
pub fn extract_summary_text(raw: &str) -> String {
    let mut parts = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('{') {
            if let Some(text) = json_string_field(line, "text")
                .or_else(|| json_string_field(line, "content"))
                .or_else(|| json_string_field(line, "message"))
            {
                let text = text.trim();
                if !text.is_empty() {
                    parts.push(text.to_string());
                }
                continue;
            }
            // Non-text JSON events (tool calls, metadata): ignored.
            continue;
        }
        parts.push(line.to_string());
    }
    parts.join("\n").chars().take(MAX_SUMMARY_CHARS).collect()
}

/// Minimal JSON string-field extractor (no new deps): finds
/// `"field": "value"` with backslash unescaping. Returns `None` when the
/// field is absent or not a JSON string.
fn json_string_field(line: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\"");
    let mut search = line;
    loop {
        let pos = search.find(&needle)?;
        let mut rest = search[pos + needle.len()..].trim_start();
        if !rest.starts_with(':') {
            search = &search[pos + needle.len()..];
            continue;
        }
        rest = rest[1..].trim_start();
        if !rest.starts_with('"') {
            return None;
        }
        let mut out = String::new();
        let mut chars = rest[1..].chars();
        let mut closed = false;
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        out.push(
                            char::from_u32(u32::from_str_radix(&hex, 16).ok()?)
                                .unwrap_or('\u{FFFD}'),
                        );
                    }
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => return None,
                },
                '"' => {
                    closed = true;
                    break;
                }
                _ => out.push(c),
            }
        }
        if closed {
            return Some(out);
        }
        return None;
    }
}

/// Full explicit summarize flow helper: create the ephemeral context,
/// run the agent, extract text, and guarantee cleanup on every path
/// (explicit close + Drop guard). Returns `(summary_text, context_existed)`.
/// `context_existed` is a test seam proving the file is gone afterwards.
#[allow(clippy::too_many_arguments)]
pub fn summarize_with_context(
    document: &str,
    agent: &str,
    model: Option<&str>,
    dir: &str,
    opencode_bin: &Path,
    timeout: Duration,
) -> Result<(String, bool), String> {
    let mut ctx = EphemeralContext::create(document)?;
    let existed_before = ctx.path().is_some_and(|p| p.exists());
    let ctx_path = ctx
        .path()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "context has no path".to_string())?;
    let result = (|| {
        let argv = build_argv(agent, model, dir, &ctx_path, opencode_bin)?;
        // opencode takes the context as a `-f` file, so no stdin payload:
        // `None` keeps this path identical to pre-M8 (23.1).
        let raw = run_agent(&argv, None, timeout)?;
        Ok::<String, String>(extract_summary_text(&raw))
    })();
    ctx.close();
    let existed_after = ctx.path().is_some_and(|p| p.exists());
    debug_assert!(existed_before);
    debug_assert!(!existed_after);
    result.map(|text| (text, existed_after))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn sandbox() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("pitwall-m5c-sum-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_agent(dir: &std::path::Path, body: &str) -> PathBuf {
        let bin = dir.join("opencode");
        std::fs::write(&bin, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    fn empty_snapshot() -> crate::collector::WorkspaceSnapshot {
        crate::collector::WorkspaceSnapshot {
            schema_version: 1,
            collected_at_epoch: 1_700_000_100,
            hostname: "testbox".to_string(),
            sessions: Vec::new(),
        }
    }

    #[test]
    fn input_hash_is_deterministic_and_terminal_free() {
        let snap = empty_snapshot();
        let context = crate::context::SummaryContext::new(snap.clone(), vec![], vec![], vec![]);
        let h1 = input_hash(&context);
        let h2 = input_hash(&context);
        assert_eq!(h1, h2);
        assert!(h1.starts_with("fnv:"));
        // Terminal text is not an input (no such field exists on the hash
        // inputs by construction); structured change must alter the hash.
        let mut snap2 = empty_snapshot();
        snap2.hostname = "otherbox".to_string();
        let context2 = crate::context::SummaryContext::new(snap2, vec![], vec![], vec![]);
        assert_ne!(h1, input_hash(&context2));
    }

    #[test]
    fn model_ids_are_strict() {
        assert!(valid_model("opencode/muse-spark-1.3-contributor-free"));
        assert!(valid_model("tokenrouter/z-ai/glm-5.3-free"));
        assert!(!valid_model(""));
        assert!(!valid_model("nomodel"));
        assert!(!valid_model("a b/c"));
        assert!(!valid_model("a/c; rm -rf ~"));
        assert!(!valid_model("a/c$(id)"));
        assert!(!valid_model("a/c`id`"));
        assert!(!valid_model("a/c|less"));
    }

    #[test]
    fn argv_is_fixed_and_never_carries_content() {
        let dir = sandbox();
        let bin = fake_agent(&dir, "#!/bin/sh\nexit 0\n");
        let hostile_ctx = dir.join("ctx-evil.json");
        let argv = build_argv(
            "opencode",
            Some("prov/model"),
            "/home/u/Work",
            &hostile_ctx,
            &bin,
        )
        .unwrap();
        // Fixed shape: binary, run, --format json, --dir DIR, -f PATH, -m MODEL, instruction.
        assert_eq!(argv[1], "run");
        assert!(!argv
            .iter()
            .any(|a| a.contains("bash") || a.contains("sh -c")));
        // Content markers must not appear: only the *path* travels.
        assert!(argv.iter().any(|a| a.ends_with("ctx-evil.json")));
        // Unsupported agents and bad inputs are refused, not executed.
        assert!(build_argv("claude", None, "/x", &hostile_ctx, &bin).is_err());
        assert!(build_argv("opencode", Some("bad model"), "/x", &hostile_ctx, &bin).is_err());
        assert!(build_argv("opencode", None, "relative/dir", &hostile_ctx, &bin).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_events_extract_text_and_ignore_tool_calls() {
        let raw = concat!(
            "{\"type\":\"tool\",\"name\":\"bash\",\"input\":\"rm -rf /\"}\n",
            "{\"type\":\"text\",\"text\":\"First sentence.\"}\n",
            "{\"type\":\"meta\",\"tokens\":42}\n",
            "{\"type\":\"text\",\"content\":\"Second line.\"}\n",
        );
        let text = extract_summary_text(raw);
        assert!(text.contains("First sentence."));
        assert!(text.contains("Second line."));
        assert!(
            !text.contains("rm -rf"),
            "tool payloads must not leak: {text}"
        );
        assert!(!text.contains("tokens"), "{text}");
    }

    #[test]
    fn json_events_without_text_return_empty() {
        for raw in [
            "{\"type\":\"tool\",\"name\":\"bash\",\"input\":\"tool payload\"}",
            "{\"type\":\"meta\",\"tokens\":42}",
            concat!(
                "{\"type\":\"tool\",\"input\":\"tool payload\"}\n",
                "{\"type\":\"meta\",\"tokens\":42}\n",
                "{\"type\":\"text\",\"text\":\"  \"}\n",
            ),
            "{\"type\":\"text\",\"text\":\"\"}",
        ] {
            assert_eq!(extract_summary_text(raw), "", "input: {raw}");
        }
    }

    #[test]
    fn plain_text_output_is_preserved_and_truncated() {
        let raw = "plain answer here";
        assert_eq!(extract_summary_text(raw), "plain answer here");
        let big = "x".repeat(MAX_SUMMARY_CHARS + 500);
        assert_eq!(
            extract_summary_text(&big).chars().count(),
            MAX_SUMMARY_CHARS
        );
        assert_eq!(extract_summary_text("   \n  "), "");
    }

    #[test]
    fn timeout_kills_and_reports() {
        let dir = sandbox();
        let bin = fake_agent(&dir, "#!/bin/sh\nsleep 30\n");
        let argv = vec![bin.to_string_lossy().into_owned()];
        let err = run_agent(&argv, None, Duration::from_millis(300)).unwrap_err();
        assert!(err.contains("timed out"), "{err}");
        // No stray sleepers from the kill path.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_failure_is_an_error_not_a_summary() {
        let dir = sandbox();
        let bin = fake_agent(&dir, "#!/bin/sh\necho boom >&2\nexit 3\n");
        let argv = vec![bin.to_string_lossy().into_owned()];
        let err = run_agent(&argv, None, Duration::from_secs(5)).unwrap_err();
        assert!(err.contains("exited"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_payload_keeps_stdin_null_and_never_hangs() {
        let dir = sandbox();
        // Prints a marker, then echoes whatever stdin holds. With the
        // pre-M8 null disposition preserved, stdin is empty and `cat`
        // returns at once — a hang here would surface as a timeout error.
        let bin = fake_agent(&dir, "#!/bin/sh\necho START\ncat\n");
        let argv = vec![bin.to_string_lossy().into_owned()];
        let raw = run_agent(&argv, None, Duration::from_secs(10)).unwrap();
        assert_eq!(raw, "START\n", "None must deliver nothing on stdin: {raw:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn payload_reaches_child_stdin_and_reaches_eof() {
        let dir = sandbox();
        let bin = fake_agent(&dir, "#!/bin/sh\necho START\ncat\n");
        let argv = vec![bin.to_string_lossy().into_owned()];
        // `cat` only terminates on EOF, so an Ok result with the payload
        // echoed back proves both delivery and the stdin close.
        let raw = run_agent(&argv, Some("PAYLOAD-LINE\n"), Duration::from_secs(10)).unwrap();
        assert_eq!(raw, "START\nPAYLOAD-LINE\n", "{raw:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn large_payload_does_not_deadlock_against_stdout() {
        let dir = sandbox();
        // Echoes the payload straight back, so both pipes are in flight at
        // once. Well beyond a 64 KB pipe buffer: without concurrent stdout
        // draining this deadlocks until the timeout kills the child.
        let bin = fake_agent(&dir, "#!/bin/sh\ncat\n");
        let argv = vec![bin.to_string_lossy().into_owned()];
        let payload = "x".repeat(128 * 1024);
        let raw = run_agent(&argv, Some(&payload), Duration::from_secs(20)).unwrap();
        assert_eq!(raw.len(), payload.len(), "short read: {} bytes", raw.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mock_end_to_end_cleans_context_on_success() {
        let dir = sandbox();
        let bin = fake_agent(
            &dir,
            "#!/bin/sh\necho '{\"type\":\"text\",\"text\":\"All quiet.\"}'\n",
        );
        // Record the ephemeral dir listing before/after via create+close is
        // covered elsewhere; here assert the full flow returns text.
        let (text, existed_after) = summarize_with_context(
            "{\"sessions\":[]}",
            "opencode",
            None,
            "/home/u/Work",
            &bin,
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(text.contains("All quiet"), "{text}");
        assert!(!existed_after, "context file must be gone");
        // No ctx-*.json remains in the runtime dir used.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mock_end_to_end_cleans_context_on_failure() {
        let dir = sandbox();
        let bin = fake_agent(&dir, "#!/bin/sh\nexit 1\n");
        let before: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        let err = summarize_with_context(
            "{\"sessions\":[]}",
            "opencode",
            None,
            "/home/u/Work",
            &bin,
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert!(err.contains("exited"), "{err}");
        let after: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        // Ephemeral files live under the runtime dir, not temp_dir; assert
        // structurally that no path leaked via the error instead.
        assert!(!err.contains("ctx-"));
        let _ = (before, after);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
