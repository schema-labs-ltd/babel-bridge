//! Log lines may describe what happened, never what was said or who said it.
//!
//! The MCP library prints full JSON-RPC messages at info. Verbose mode also
//! enables its debug dumps of requests and errors. Those records include
//! message bodies, phone numbers, contact names, and chat ids, so library
//! targets stay off at every verbosity and this process logs only metadata.

use regex::Regex;
use serde::Serialize;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tracing::info;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

pub fn init_logging(verbose: bool) {
    tracing_subscriber::fmt()
        .with_env_filter(logging_filter(verbose))
        .with_target(false)
        .without_time()
        .with_ansi(false)
        .with_writer(RedactingMakeWriter)
        .init();
}

pub fn logging_filter(verbose: bool) -> EnvFilter {
    // `rmcp` info/debug/warn events include the JSON-RPC body. Other debug
    // loggers can print HTTP bodies or SQL, so verbose mode stays on our crate.
    let directives = if verbose {
        "debug,rmcp=off,hyper=warn,h2=warn,tower=warn,reqwest=info,tokio=info,sqlx=warn"
    } else {
        "info,rmcp=off"
    };
    EnvFilter::try_new(directives).unwrap_or_else(|_| EnvFilter::new("info,rmcp=off"))
}

pub fn log_mcp_call(
    method: &str,
    tool: Option<&str>,
    status: &str,
    error_code: Option<i32>,
    count: usize,
    duration: Duration,
) {
    let method = safe_label(method);
    let tool = tool.map(safe_label).unwrap_or_default();
    let duration_ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
    match error_code {
        Some(error_code) => info!(
            method,
            tool, status, error_code, count, duration_ms, "MCP call"
        ),
        None => info!(method, tool, status, count, duration_ms, "MCP call"),
    }
}

fn safe_label(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '.'))
    {
        value.to_string()
    } else {
        "invalid".to_string()
    }
}

/// Remove chat ids, phone numbers, and protocol payloads from a log line.
pub fn redact_log_text(input: &str) -> String {
    let without_protocol = strip_protocol_nodes(input);
    let without_jids = jid_pattern().replace_all(&without_protocol, "[chat]");
    phone_pattern()
        .replace_all(&without_jids, |caps: &regex::Captures<'_>| {
            let matched = caps.get(0).expect("phone match");
            if is_size_count(&without_jids, matched.start(), matched.end()) {
                matched.as_str().to_string()
            } else {
                "[redacted]".to_string()
            }
        })
        .into_owned()
}

fn strip_protocol_nodes(input: &str) -> String {
    let Some(start) = input.find('<') else {
        return input.to_string();
    };
    let Some(end) = input.rfind('>') else {
        return input.to_string();
    };
    if end <= start {
        return input.to_string();
    }
    let mut output = String::with_capacity(input.len());
    output.push_str(&input[..start]);
    output.push_str("[protocol]");
    output.push_str(&input[end + 1..]);
    output
}

fn is_size_count(text: &str, start: usize, end: usize) -> bool {
    let after = text[end..].trim_start();
    if after.starts_with("bytes") || after.starts_with("byte") {
        return true;
    }
    let before = text[..start].trim_end();
    before.ends_with("size:") || before.ends_with("size")
}

fn jid_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r"(?i)\b[0-9A-Za-z._-]{1,128}@(?:s\.whatsapp\.net|c\.us|g\.us|lid|broadcast|newsletter|hosted)\b",
        )
        .expect("jid pattern")
    })
}

fn phone_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"\b\d{8,15}\b").expect("phone pattern"))
}

/// Warnings and errors kept in memory for `/api/diagnostics`, oldest first.
const RECENT_PROBLEM_LIMIT: usize = 200;
const RECENT_PROBLEM_MAX_CHARS: usize = 2000;

/// One already-redacted warning or error line.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LoggedProblem {
    /// Unix seconds when the line was written.
    pub at: i64,
    pub line: String,
}

fn recent_problem_store() -> &'static Mutex<VecDeque<LoggedProblem>> {
    static STORE: OnceLock<Mutex<VecDeque<LoggedProblem>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(VecDeque::with_capacity(RECENT_PROBLEM_LIMIT)))
}

/// Keeps a redacted log line if it is a warning or an error.
fn remember_problem(redacted_line: &str) {
    let line = redacted_line.trim();
    if !(line.starts_with("WARN") || line.starts_with("ERROR")) {
        return;
    }
    // Diagnostics leave the server, and a URL's path or query can name a
    // person or carry a token (a failed link preview logs the whole link).
    let line = strip_url_paths(line);
    let line: String = line.chars().take(RECENT_PROBLEM_MAX_CHARS).collect();
    let mut store = recent_problem_store()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if store.len() == RECENT_PROBLEM_LIMIT {
        store.pop_front();
    }
    store.push_back(LoggedProblem {
        at: chrono::Utc::now().timestamp(),
        line,
    });
}

/// Keep each URL's scheme and host; drop credentials, path, query and fragment.
fn strip_url_paths(line: &str) -> String {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r#"(?i)\b([a-z][a-z0-9+.-]*://)(?:[^@/\s]*@)?([^/\s?#)\]"'<>]+)([^\s)\]"'<>]*)"#)
            .expect("url pattern")
    });
    pattern
        .replace_all(line, |caps: &regex::Captures<'_>| {
            let rest = if caps[3].is_empty() { "" } else { "/[path]" };
            format!("{}{}{}", &caps[1], &caps[2], rest)
        })
        .into_owned()
}

/// The most recent warnings and errors, oldest first, after redaction.
pub fn recent_problems() -> Vec<LoggedProblem> {
    recent_problem_store()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .cloned()
        .collect()
}

struct RedactingMakeWriter;

struct RedactingWriter {
    buf: Vec<u8>,
}

impl<'a> MakeWriter<'a> for RedactingMakeWriter {
    type Writer = RedactingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter { buf: Vec::new() }
    }
}

impl Write for RedactingWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.finish()
    }
}

impl Drop for RedactingWriter {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

impl RedactingWriter {
    fn finish(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&self.buf);
        let redacted = redact_log_text(&text);
        self.buf.clear();
        for line in redacted.lines() {
            remember_problem(line);
        }
        let mut stderr = io::stderr().lock();
        stderr.write_all(redacted.as_bytes())?;
        if !redacted.ends_with('\n') {
            stderr.write_all(b"\n")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing::info;
    use tracing_subscriber::fmt;

    #[test]
    fn warnings_and_errors_are_kept_redacted_for_diagnostics() {
        let marker = uuid::Uuid::new_v4().to_string();
        for line in [
            format!(" INFO kept-out {marker}\n"),
            format!(" WARN send failed {marker} for 447700900123@s.whatsapp.net\n"),
            format!("ERROR bridge exited {marker} phone 447700900123\n"),
            format!("ERROR Failed to fetch link preview {marker} for https://docs.example.com/d/Ana-Lopez-lease?token=s3cret#p2: error sending request for url (https://user:pw@docs.example.com/d/Ana-Lopez-lease?token=s3cret)\n"),
        ] {
            let mut writer = RedactingMakeWriter.make_writer();
            writer.write_all(line.as_bytes()).unwrap();
            writer.flush().unwrap();
        }
        let kept: Vec<String> = recent_problems()
            .into_iter()
            .map(|problem| problem.line)
            .filter(|line| line.contains(&marker))
            .collect();
        assert_eq!(
            kept,
            [
                format!("WARN send failed {marker} for [chat]"),
                format!("ERROR bridge exited {marker} phone [redacted]"),
                format!("ERROR Failed to fetch link preview {marker} for https://docs.example.com/[path] error sending request for url (https://docs.example.com/[path])"),
            ]
        );
    }

    const BODY: &str = "zz-unique-body-449d";
    const PHONE: &str = "15550001111";

    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("log buffer").extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn capture(verbose: bool, write_logs: impl FnOnce()) -> String {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let subscriber = fmt()
            .with_env_filter(logging_filter(verbose))
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_writer({
                let buf = buf.clone();
                move || Capture(buf.clone())
            })
            .finish();
        tracing::subscriber::with_default(subscriber, write_logs);
        let bytes = buf.lock().expect("log buffer").clone();
        String::from_utf8(bytes).expect("utf-8 log")
    }

    #[test]
    fn redacts_numbers_chat_ids_and_protocol_payloads_but_keeps_sizes() {
        let input = format!(
            "Decrypting message from {PHONE}@s.whatsapp.net <message><conversation>{BODY}</conversation></message> size: 10485760 bytes"
        );
        let got = redact_log_text(&input);
        assert!(!got.contains(PHONE), "{got}");
        assert!(!got.contains(BODY), "{got}");
        assert!(!got.contains("@s.whatsapp.net"), "{got}");
        assert!(got.contains("Decrypting message"), "{got}");
        assert!(got.contains("10485760 bytes"), "{got}");
    }

    #[test]
    fn library_payload_logs_are_dropped_and_mcp_metadata_remains() {
        for verbose in [false, true] {
            let logs = capture(verbose, || {
                info!(
                    target: "rmcp::transport::streamable_http_server::tower",
                    message = format!("{BODY} {PHONE}@s.whatsapp.net"),
                    "response"
                );
                tracing::debug!(
                    target: "rmcp::service",
                    request = format!("{BODY} {PHONE}"),
                    "received request"
                );
                tracing::warn!(
                    target: "rmcp::service",
                    error = format!("failed {BODY} {PHONE}"),
                    "response error"
                );
                log_mcp_call(
                    "tools/call",
                    Some("read_messages"),
                    "ok",
                    None,
                    2,
                    Duration::from_millis(5),
                );
                log_mcp_call(
                    "tools/call",
                    Some("read_messages"),
                    "error",
                    Some(-32602),
                    0,
                    Duration::from_millis(7),
                );
            });
            assert!(!logs.contains(BODY), "verbose={verbose} {logs}");
            assert!(!logs.contains(PHONE), "verbose={verbose} {logs}");
            assert!(logs.contains("method=\"tools/call\""), "{logs}");
            assert!(logs.contains("tool=\"read_messages\""), "{logs}");
            assert!(logs.contains("status=\"ok\""), "{logs}");
            assert!(logs.contains("status=\"error\""), "{logs}");
            assert!(logs.contains("count=2"), "{logs}");
            assert!(logs.contains("count=0"), "{logs}");
            assert!(logs.contains("duration_ms=5"), "{logs}");
            assert!(logs.contains("duration_ms=7"), "{logs}");
            assert!(logs.contains("error_code=-32602"), "{logs}");
        }
    }
}
