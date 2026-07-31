//! The eval-dataset schema: the published contract for `evals/evals.jsonl`.
//!
//! A forthcoming `adept create` command writes a synthetic eval dataset
//! alongside every skill it generates, so the skill's *behaviour* — not just
//! its lint-checked *form* — has something to be measured against. This
//! module defines that dataset's shape, parses and serializes it, and
//! validates it. **adept never executes a dataset.** Grading (running
//! `command` assertions, checking file contents, etc.) is the job of a
//! separate harness; this module's `validate` only checks that a dataset is
//! well-formed enough to hand to one.
//!
//! The on-disk format is JSONL: one [`EvalCase`] per line, with **no
//! enclosing envelope** (no top-level array, no wrapper object). This is
//! deliberate — it lets a dataset be streamed, appended to, and diffed one
//! case at a time. Because there is no envelope to carry document-level
//! metadata, every line repeats its own `schema_version`, which is what keeps
//! a file self-describing even after being truncated, concatenated with
//! another dataset, or appended to by hand.
//!
//! This module is deliberately inert: no network access, no subprocess
//! spawning, and no filesystem access beyond what a caller hands it as a
//! `&str`. It exists to define and check a shape, nothing more.

use serde::{Deserialize, Serialize};

/// The eval-dataset schema version this build of adept understands.
///
/// Deliberately **independent of `adept_score::prompts::PROMPT_VERSION`**
/// (and of `adept_agent`'s prompt versions): prompt wording drifts routinely
/// as generation is tuned, and none of that drift should look like a
/// breaking change to a harness consuming this schema. `SCHEMA_VERSION`
/// changes only when the *shape* of a dataset line changes — rarely, and
/// loudly, the same way a lint rule code is never reused.
pub const SCHEMA_VERSION: u32 = 1;

/// One deterministic, offline-checkable assertion about a case's expected
/// outcome.
///
/// This is the complete vocabulary adept defines — four kinds, taken from
/// huggingface/upskill's graders. It is intentionally small: a dataset that
/// only uses these four kinds is unambiguous to grade the same way by any
/// two independent harnesses. See `docs/EVALS.md` for the full semantics of
/// each kind, in particular `Command`'s exit-code-only contract.
///
/// An unknown `kind` on deserialization produces a `serde_json` error (via
/// [`EvalError::Parse`]), never a panic — this is what lets the schema grow
/// a fifth assertion kind later without corrupting an old reader's behavior
/// on a new file: it will report a clear per-line error instead of silently
/// misinterpreting it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Assertion {
    /// The harness-produced output contains `value` as a substring.
    Contains {
        /// The substring that must appear in the output.
        value: String,
    },
    /// A file at `path` exists.
    FileExists {
        /// Path to the file, relative to a location the harness defines
        /// (see `docs/EVALS.md`).
        path: String,
    },
    /// A file at `path` exists and contains `value` as a substring.
    FileContains {
        /// Path to the file, relative to a location the harness defines.
        path: String,
        /// The substring that must appear in the file's contents.
        value: String,
    },
    /// A shell command whose **exit code alone** decides pass (`0`) or fail
    /// (non-zero). adept never runs this; see `docs/EVALS.md` for the exact
    /// contract a harness must honor (working directory, what is and is not
    /// captured).
    Command {
        /// The shell command to run.
        command: String,
    },
}

/// One test case in an eval dataset: a prompt the skill should handle, plus
/// the assertions a harness checks the response against.
///
/// Carries its own `schema_version` (see [`SCHEMA_VERSION`] and the module
/// docs) so a dataset stays self-describing without a JSONL envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalCase {
    /// The schema version this line was written against.
    pub schema_version: u32,
    /// The prompt the skill under test should handle.
    pub prompt: String,
    /// The assertions a harness checks the response against. May be empty
    /// on an individual line (see [`EvalError::Empty`] for the
    /// dataset-level non-emptiness check, which is separate).
    pub assertions: Vec<Assertion>,
}

/// Errors from parsing or validating an eval dataset.
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    /// Line `line` (1-indexed) failed to parse as an [`EvalCase`].
    #[error("line {line}: {source}")]
    Parse {
        /// The 1-indexed line number that failed to parse.
        line: usize,
        /// The underlying JSON error.
        #[source]
        source: serde_json::Error,
    },
    /// Line `line` declared a `schema_version` this build of adept does not
    /// understand.
    #[error(
        "line {line}: unsupported schema_version {found} (this build of adept understands {SCHEMA_VERSION})"
    )]
    UnsupportedSchemaVersion {
        /// The 1-indexed line number.
        line: usize,
        /// The `schema_version` found on that line.
        found: u32,
    },
    /// The dataset contained no cases (after skipping blank lines).
    #[error("eval dataset is empty: at least one case is required")]
    Empty,
}

/// Parse a JSONL eval dataset from `text`, one [`EvalCase`] per line.
///
/// Blank lines are skipped (a common artifact of hand-edited or
/// newline-terminated files). On the first line that fails to parse,
/// returns [`EvalError::Parse`] naming the 1-indexed line number.
///
/// This function only parses; it does not check `schema_version` or
/// non-emptiness. Use [`validate`] for the full set of dataset-level checks.
///
/// # Errors
/// Returns [`EvalError::Parse`] if any non-blank line is not a valid
/// [`EvalCase`].
pub fn parse_jsonl(text: &str) -> Result<Vec<EvalCase>, EvalError> {
    Ok(parse_jsonl_with_lines(text)?
        .into_iter()
        .map(|(_, case)| case)
        .collect())
}

/// Like [`parse_jsonl`], but pairs each case with its 1-indexed source line
/// number (blank lines are skipped, so line numbers may be non-contiguous).
/// Used internally by [`validate`] so a `schema_version` error can point at
/// the real line rather than the case's position in the parsed `Vec`.
fn parse_jsonl_with_lines(text: &str) -> Result<Vec<(usize, EvalCase)>, EvalError> {
    let mut cases = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let case: EvalCase = serde_json::from_str(line).map_err(|source| EvalError::Parse {
            line: idx + 1,
            source,
        })?;
        cases.push((idx + 1, case));
    }
    Ok(cases)
}

/// Serialize `cases` back to JSONL: one compact JSON object per line,
/// newline-terminated, no enclosing envelope.
///
/// # Panics
/// Panics if a case fails to serialize, which should not happen for a
/// well-formed [`EvalCase`] (all fields are plain strings/enums with no
/// fallible custom serialization).
#[must_use]
pub fn to_jsonl(cases: &[EvalCase]) -> String {
    let mut out = String::new();
    for case in cases {
        let line = serde_json::to_string(case).expect("EvalCase serialization cannot fail");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Validate a JSONL eval dataset given as `text`.
///
/// Enforces, in order:
/// - every non-blank line parses as an [`EvalCase`] (line number reported on
///   failure — unknown assertion `kind`s surface here as a parse error);
/// - every case's `schema_version` is one this build of adept understands
///   (currently only [`SCHEMA_VERSION`] itself);
/// - the dataset is non-empty.
///
/// Deliberately does not check whether assertions are *satisfiable* — that
/// is a harness's job when it runs the dataset, not adept's when it defines
/// the shape.
///
/// # Errors
/// Returns the first [`EvalError`] encountered.
pub fn validate(text: &str) -> Result<(), EvalError> {
    let cases = parse_jsonl_with_lines(text)?;
    for (line, case) in &cases {
        if case.schema_version != SCHEMA_VERSION {
            return Err(EvalError::UnsupportedSchemaVersion {
                line: *line,
                found: case.schema_version,
            });
        }
    }
    if cases.is_empty() {
        return Err(EvalError::Empty);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_case() -> EvalCase {
        EvalCase {
            schema_version: SCHEMA_VERSION,
            prompt: "Summarize the attached report.".to_string(),
            assertions: vec![
                Assertion::Contains {
                    value: "summary".to_string(),
                },
                Assertion::FileExists {
                    path: "out/summary.md".to_string(),
                },
                Assertion::FileContains {
                    path: "out/summary.md".to_string(),
                    value: "conclusion".to_string(),
                },
                Assertion::Command {
                    command: "test -s out/summary.md".to_string(),
                },
            ],
        }
    }

    #[test]
    fn round_trips_a_single_case() {
        let case = sample_case();
        let jsonl = to_jsonl(std::slice::from_ref(&case));
        let parsed = parse_jsonl(&jsonl).unwrap();
        assert_eq!(parsed, vec![case]);
    }

    #[test]
    fn parses_multiple_lines_skipping_blanks() {
        let cases = vec![sample_case(), sample_case()];
        let mut jsonl = to_jsonl(&cases);
        jsonl.push('\n'); // trailing blank line
        let parsed = parse_jsonl(&jsonl).unwrap();
        assert_eq!(parsed, cases);
    }

    #[test]
    fn parse_reports_the_offending_line_number() {
        let good = serde_json::to_string(&sample_case()).unwrap();
        let text = format!("{good}\nnot valid json\n{good}\n");
        let err = parse_jsonl(&text).unwrap_err();
        match err {
            EvalError::Parse { line, .. } => assert_eq!(line, 2),
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_assertion_kind_is_a_clear_error_not_a_panic() {
        let text =
            r#"{"schema_version":1,"prompt":"p","assertions":[{"kind":"unheard_of","value":"x"}]}"#;
        let err = parse_jsonl(text).unwrap_err();
        assert!(matches!(err, EvalError::Parse { line: 1, .. }));
    }

    #[test]
    fn validate_rejects_unsupported_schema_version() {
        let text = r#"{"schema_version":999,"prompt":"p","assertions":[]}"#;
        let err = validate(text).unwrap_err();
        match err {
            EvalError::UnsupportedSchemaVersion { line, found } => {
                assert_eq!(line, 1);
                assert_eq!(found, 999);
            }
            other => panic!("expected UnsupportedSchemaVersion, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_empty_dataset() {
        let err = validate("").unwrap_err();
        assert!(matches!(err, EvalError::Empty));
    }

    #[test]
    fn validate_accepts_well_formed_dataset() {
        let jsonl = to_jsonl(&[sample_case()]);
        validate(&jsonl).unwrap();
    }

    #[test]
    fn missing_required_field_is_a_parse_error() {
        let text = r#"{"schema_version":1,"assertions":[]}"#; // missing `prompt`
        let err = parse_jsonl(text).unwrap_err();
        assert!(matches!(err, EvalError::Parse { line: 1, .. }));
    }
}
