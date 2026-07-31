//! On-disk capture of raw LLM request/response payloads.
//!
//! `tracing` (see [`crate::client`]) is the right shape for "tell me what
//! happened"; it is the wrong shape for "keep this exact byte sequence so I
//! can promote it to a test fixture." Scraping a log stream for a JSON body
//! is lossy and fiddly, so capture is a second, independent layer: one
//! timestamped folder per invocation, one subfolder per LLM call, bodies
//! written verbatim and never truncated.
//!
//! ```text
//! <root>/
//!   2026_07_31_14_22_07/
//!     run_metadata.json
//!     call_0001/
//!       request.json
//!       response.json
//!       call_metadata.json
//!     call_0002/
//! ```
//!
//! Two rules govern everything here:
//!
//! - **Append-only.** A run folder is never reused, so pointing repeated
//!   invocations at the same root accumulates evidence instead of
//!   destroying it.
//! - **Capture never fails the call.** Every write is best-effort; an I/O
//!   error is reported through `tracing` at `WARN` and otherwise swallowed.
//!   A full disk must not turn a successful `score` run into a failure.
//!
//! The API key is never written, in any form — [`RunMetadata`] records only
//! [`RunMetadata::api_key_present`], and `Authorization` is omitted entirely
//! from captured request headers (it is never collected in the first place).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

/// Static, run-wide provenance for a capture directory.
///
/// The goal is that `run_metadata.json` alone is enough to label and
/// reproduce a run: a reader should never have to consult the shell history
/// that produced it. Fields that do not apply to a given subcommand are
/// left `None` and omitted from the JSON.
#[derive(Debug, Clone, Serialize)]
pub struct RunMetadata {
    /// The version of the `adept` tooling that produced the run.
    pub adept_version: String,
    /// [`crate::prompts::PROMPT_VERSION`] at capture time — prompt wording
    /// moves scores, so a fixture is meaningless without it.
    pub prompt_version: String,
    /// The subcommand that issued the calls, e.g. `"score"` or `"fix"`.
    pub subcommand: String,
    /// The resolved model identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The resolved base URL of the OpenAI-compatible endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// The resolved tokenizer name, where token analysis applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokenizer: Option<String>,
    /// The resolved sampling seed, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// The resolved number of generated trigger prompts (`score`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_prompts: Option<usize>,
    /// The resolved number of judge samples per prompt (`score`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_samples: Option<usize>,
    /// The resolved maximum number of fix rounds (`fix`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rounds: Option<usize>,
    /// The skill (or directory) the run targeted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    /// Whether an API key was configured. The key itself is **never**
    /// recorded — this boolean exists so that "the run was unauthenticated"
    /// is distinguishable from "the key leaked out of the artifact."
    pub api_key_present: bool,
    /// Any further caller-supplied provenance (e.g. where each resolved
    /// value came from: CLI flag / `adept.toml` / env var / default).
    /// Flattened into the top level of `run_metadata.json`.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl RunMetadata {
    /// Construct metadata for `subcommand`, filling in the crate version and
    /// [`crate::prompts::PROMPT_VERSION`] automatically.
    #[must_use]
    pub fn new(subcommand: impl Into<String>) -> Self {
        Self {
            adept_version: env!("CARGO_PKG_VERSION").to_string(),
            prompt_version: crate::prompts::PROMPT_VERSION.to_string(),
            subcommand: subcommand.into(),
            model: None,
            base_url: None,
            tokenizer: None,
            seed: None,
            num_prompts: None,
            judge_samples: None,
            max_rounds: None,
            target_path: None,
            api_key_present: false,
            extra: BTreeMap::new(),
        }
    }
}

/// What `run_metadata.json` actually contains: the caller's [`RunMetadata`]
/// plus the timing and exit code the sink owns.
#[derive(Debug, Serialize)]
struct RunRecord<'a> {
    #[serde(flatten)]
    metadata: &'a RunMetadata,
    started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
}

/// One LLM call's worth of captured evidence, handed to
/// [`CaptureSink::record_call`] at the moment of receipt.
///
/// Bodies are owned `String`s written verbatim; the sequence number is
/// assigned by the sink, not the caller, so concurrent calls cannot collide.
#[derive(Debug, Clone)]
pub struct CapturedCall {
    /// Zero-based retry index within [`crate::LlmClient::chat`].
    pub attempt: u32,
    /// The full endpoint URL the request was sent to.
    pub endpoint: String,
    /// The HTTP status, or `None` if the request never got a response
    /// (connection error, timeout).
    pub status: Option<u16>,
    /// Request headers — `Authorization` is never present.
    pub request_headers: BTreeMap<String, String>,
    /// Response headers, if a response was received.
    pub response_headers: BTreeMap<String, String>,
    /// The request body exactly as serialized and sent.
    pub request_body: String,
    /// The response body exactly as received, before any parsing.
    pub response_body: String,
    /// RFC 3339 wall-clock time the request was issued.
    pub started_at: String,
    /// RFC 3339 wall-clock time the body finished being read.
    pub finished_at: String,
    /// Elapsed milliseconds between the two.
    pub duration_ms: u64,
    /// What became of the call: `"ok"`, `"retried"`, or an [`crate::LlmError`]
    /// variant name.
    pub outcome: String,
    /// Which logical step issued it (e.g. `"generate_prompts"`, `"judge"`,
    /// `"fix_round_2"`), where the caller knows.
    pub step: Option<String>,
}

/// The serialized shape of `call_metadata.json`.
#[derive(Debug, Serialize)]
struct CallRecord<'a> {
    sequence: u64,
    attempt: u32,
    endpoint: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    step: Option<&'a str>,
    request_headers: &'a BTreeMap<String, String>,
    response_headers: &'a BTreeMap<String, String>,
    started_at: &'a str,
    finished_at: &'a str,
    duration_ms: u64,
    outcome: &'a str,
}

/// An append-only capture directory for a single invocation.
///
/// Construction eagerly creates the timestamped run folder and writes a
/// first `run_metadata.json`, so a crashed run still leaves a labelled,
/// readable directory behind. [`CaptureSink::finalize`] rewrites it with
/// `finished_at` and the process exit code.
///
/// `Send + Sync` (the sequence counter is atomic), so an
/// [`crate::OpenAiCompatClient`] holding one behind an `Arc` stays
/// `Send + Sync` too.
#[derive(Debug)]
pub struct CaptureSink {
    run_dir: PathBuf,
    metadata: RunMetadata,
    started_at: String,
    next_sequence: AtomicU64,
}

impl CaptureSink {
    /// Create a new run folder under `root`, named for the current local
    /// time (`YYYY_MM_DD_HH_MM_SS`), and write its initial
    /// `run_metadata.json`.
    ///
    /// Two runs started within the same second get `_2`, `_3`, … suffixes
    /// rather than sharing a folder: capture is append-only, and silently
    /// interleaving two runs' calls would be worse than an ugly name.
    ///
    /// # Errors
    /// Returns any I/O error from creating the run directory. Only
    /// *construction* is fallible — once a sink exists, recording is
    /// best-effort and never fails a call.
    pub fn new(root: impl AsRef<Path>, metadata: RunMetadata) -> std::io::Result<Self> {
        let root = root.as_ref();
        std::fs::create_dir_all(root)?;

        let stamp = jiff::Zoned::now().strftime("%Y_%m_%d_%H_%M_%S").to_string();
        let mut run_dir = root.join(&stamp);
        let mut suffix = 1u32;
        while run_dir.exists() {
            suffix += 1;
            run_dir = root.join(format!("{stamp}_{suffix}"));
        }
        std::fs::create_dir(&run_dir)?;

        let sink = Self {
            run_dir,
            metadata,
            started_at: jiff::Timestamp::now().to_string(),
            next_sequence: AtomicU64::new(1),
        };
        sink.write_run_metadata(None, None);
        Ok(sink)
    }

    /// The timestamped run folder this sink writes into.
    #[must_use]
    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    /// Write one call's artifacts into the next `call_NNNN/` folder.
    ///
    /// Best-effort: I/O failures are logged at `WARN` and swallowed, because
    /// losing capture evidence is strictly better than turning a working LLM
    /// call into a hard error.
    pub fn record_call(&self, call: &CapturedCall) {
        let sequence = self.next_sequence.fetch_add(1, Ordering::SeqCst);
        let dir = self.run_dir.join(format!("call_{sequence:04}"));
        if let Err(err) = std::fs::create_dir_all(&dir) {
            tracing::warn!(path = %dir.display(), error = %err, "failed to create capture call directory");
            return;
        }

        // Bodies go to disk verbatim, before anything tries to parse them:
        // a malformed body is exactly the payload the reader needs.
        write_best_effort(&dir.join("request.json"), &call.request_body);
        write_best_effort(&dir.join("response.json"), &call.response_body);

        let record = CallRecord {
            sequence,
            attempt: call.attempt,
            endpoint: &call.endpoint,
            status: call.status,
            step: call.step.as_deref(),
            request_headers: &call.request_headers,
            response_headers: &call.response_headers,
            started_at: &call.started_at,
            finished_at: &call.finished_at,
            duration_ms: call.duration_ms,
            outcome: &call.outcome,
        };
        match serde_json::to_string_pretty(&record) {
            Ok(json) => write_best_effort(&dir.join("call_metadata.json"), &json),
            Err(err) => {
                tracing::warn!(error = %err, "failed to serialize call metadata");
            }
        }
    }

    /// Rewrite `run_metadata.json` with `finished_at` and the process exit
    /// code. Best-effort, like [`CaptureSink::record_call`].
    pub fn finalize(&self, exit_code: i32) {
        self.write_run_metadata(Some(jiff::Timestamp::now().to_string()), Some(exit_code));
    }

    fn write_run_metadata(&self, finished_at: Option<String>, exit_code: Option<i32>) {
        let record = RunRecord {
            metadata: &self.metadata,
            started_at: self.started_at.clone(),
            finished_at,
            exit_code,
        };
        match serde_json::to_string_pretty(&record) {
            Ok(json) => write_best_effort(&self.run_dir.join("run_metadata.json"), &json),
            Err(err) => {
                tracing::warn!(error = %err, "failed to serialize run metadata");
            }
        }
    }
}

fn write_best_effort(path: &Path, contents: &str) {
    if let Err(err) = std::fs::write(path, contents) {
        tracing::warn!(path = %path.display(), error = %err, "failed to write capture artifact");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_call() -> CapturedCall {
        CapturedCall {
            attempt: 0,
            endpoint: "http://localhost:1/v1/chat/completions".to_string(),
            status: Some(200),
            request_headers: BTreeMap::from([(
                "content-type".to_string(),
                "application/json".to_string(),
            )]),
            response_headers: BTreeMap::new(),
            request_body: r#"{"model":"m"}"#.to_string(),
            response_body: r#"{"choices":[]}"#.to_string(),
            started_at: "2026-07-31T14:22:07Z".to_string(),
            finished_at: "2026-07-31T14:22:08Z".to_string(),
            duration_ms: 1000,
            outcome: "ok".to_string(),
            step: Some("judge".to_string()),
        }
    }

    fn metadata_with_key_present() -> RunMetadata {
        let mut meta = RunMetadata::new("score");
        meta.model = Some("gpt-test".to_string());
        meta.base_url = Some("http://localhost:1/v1".to_string());
        meta.api_key_present = true;
        meta
    }

    #[test]
    fn writes_expected_layout_and_never_the_key() {
        let root = tempfile::tempdir().unwrap();
        let sink = CaptureSink::new(root.path(), metadata_with_key_present()).unwrap();
        sink.record_call(&sample_call());
        sink.record_call(&sample_call());
        sink.finalize(1);

        let run_dir = sink.run_dir().to_path_buf();
        let run_meta = std::fs::read_to_string(run_dir.join("run_metadata.json")).unwrap();
        assert!(run_meta.contains("\"api_key_present\": true"));
        assert!(run_meta.contains("\"exit_code\": 1"));
        assert!(run_meta.contains("\"finished_at\""));
        assert!(run_meta.contains("prompt_version"));
        // The key is never handed to the sink at all; assert the artifact
        // carries no field that could hold one.
        assert!(!run_meta.contains("api_key\":"));
        assert!(!run_meta.contains("sk-"));

        for (n, expected_seq) in [("call_0001", 1), ("call_0002", 2)] {
            let call_dir = run_dir.join(n);
            assert_eq!(
                std::fs::read_to_string(call_dir.join("request.json")).unwrap(),
                r#"{"model":"m"}"#
            );
            assert_eq!(
                std::fs::read_to_string(call_dir.join("response.json")).unwrap(),
                r#"{"choices":[]}"#
            );
            let call_meta = std::fs::read_to_string(call_dir.join("call_metadata.json")).unwrap();
            assert!(call_meta.contains(&format!("\"sequence\": {expected_seq}")));
            assert!(call_meta.contains("\"status\": 200"));
            assert!(!call_meta.to_lowercase().contains("authorization"));
        }
    }

    #[test]
    fn second_run_appends_a_new_folder() {
        let root = tempfile::tempdir().unwrap();
        let first = CaptureSink::new(root.path(), RunMetadata::new("score")).unwrap();
        let second = CaptureSink::new(root.path(), RunMetadata::new("score")).unwrap();
        assert_ne!(first.run_dir(), second.run_dir());

        let mut entries: Vec<_> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        entries.sort();
        assert_eq!(entries.len(), 2, "each invocation gets its own folder");
        assert!(first.run_dir().join("run_metadata.json").exists());
        assert!(second.run_dir().join("run_metadata.json").exists());
    }
}
