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
//! **adept never executes a dataset**: it never spawns a subprocess and
//! never runs a case itself. But adept is the reference *grader* for one,
//! invoked by a separate harness that executes each case and hands the
//! results back. The dataset half of this module (parsing, validating) has
//! no filesystem access beyond what a caller hands it as a `&str`; the
//! grading half ([`grade`]) does read files — but only ones the harness
//! names via a supplied working directory, never anything it spawns or
//! discovers on its own.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The eval-dataset schema version this build of adept understands.
///
/// Deliberately **independent of `adept_agent::eval::prompts::PROMPT_VERSION`**
/// (and of `adept_agent`'s prompt versions): prompt wording drifts routinely
/// as generation is tuned, and none of that drift should look like a
/// breaking change to a harness consuming this schema. `SCHEMA_VERSION`
/// changes only when the *shape* of a dataset line changes — rarely, and
/// loudly, the same way a lint rule code is never reused.
pub const SCHEMA_VERSION: u32 = 2;

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

impl Assertion {
    /// The `kind` discriminant this assertion serializes under (`"contains"`,
    /// `"file_exists"`, `"file_contains"`, or `"command"`), matching the
    /// `#[serde(tag = "kind", ...)]` on this enum. Callers that need the
    /// discriminant as a string (e.g. to summarize a case's assertions)
    /// should use this rather than re-encoding the mapping by hand.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Assertion::Contains { .. } => "contains",
            Assertion::FileExists { .. } => "file_exists",
            Assertion::FileContains { .. } => "file_contains",
            Assertion::Command { .. } => "command",
        }
    }
}

/// One test case in an eval dataset: a prompt the skill should handle, plus
/// the assertions a harness checks the response against.
///
/// Carries its own `schema_version` (see [`SCHEMA_VERSION`] and the module
/// docs) so a dataset stays self-describing without a JSONL envelope.
///
/// Identified by `id`, not by its position in the file: a [`CaseResult`]
/// references a case by this `id`, so reordering, filtering, or regenerating
/// a subset of a dataset never changes which case a result grades against.
/// `id` must be non-empty and unique across the dataset — see [`validate`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalCase {
    /// The schema version this line was written against.
    pub schema_version: u32,
    /// This case's identity, stable across reordering of the dataset file.
    /// Must be non-empty and unique across the dataset (see [`validate`]).
    /// `adept create` generates a deterministic kebab-case slug; a
    /// hand-written dataset may use any non-empty string.
    ///
    /// `#[serde(default)]` (rather than a hard deserialization requirement)
    /// is deliberate: it lets a `schema_version: 1` line — which predates
    /// this field and so never has it — still deserialize far enough for
    /// [`validate`]'s `schema_version` check to run first and report
    /// [`EvalError::UnsupportedSchemaVersion`], instead of a generic missing-
    /// field [`EvalError::Parse`] that would obscure the real reason the
    /// line is rejected. A `schema_version: 2` line without `id` still gets
    /// rejected — by [`validate`]'s empty-id check, which runs right after
    /// the schema_version check.
    #[serde(default)]
    pub id: String,
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
    /// Line `line` has an empty `id`.
    #[error("line {line}: case id must not be empty")]
    EmptyId {
        /// The 1-indexed line number of the offending case.
        line: usize,
    },
    /// Line `line` repeats an `id` already used earlier in the dataset.
    #[error("line {line}: duplicate case id {id:?}")]
    DuplicateId {
        /// The 1-indexed line number of the duplicate.
        line: usize,
        /// The repeated id.
        id: String,
    },
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
    Ok(parse_jsonl_lines(text)?
        .into_iter()
        .map(|(_, case)| case)
        .collect())
}

/// Parse a JSONL document of `T`s from `text`, one value per line, pairing
/// each with its 1-indexed source line number. Blank lines are skipped. The
/// shared core behind [`parse_jsonl`]/[`validate`]/[`parse_and_validate`]
/// (over [`EvalCase`]) and [`parse_results_jsonl`] (over [`CaseResult`]), so
/// the blank-line-skip behaviour and the 1-indexed [`EvalError::Parse`]
/// contract live in exactly one place.
fn parse_jsonl_lines<T: DeserializeOwned>(text: &str) -> Result<Vec<(usize, T)>, EvalError> {
    let mut values = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: T = serde_json::from_str(line).map_err(|source| EvalError::Parse {
            line: idx + 1,
            source,
        })?;
        values.push((idx + 1, value));
    }
    Ok(values)
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
/// - every case's `id` is non-empty;
/// - every case's `id` is unique across the dataset;
/// - the dataset is non-empty.
///
/// Deliberately does not check whether assertions are *satisfiable* — that
/// is a harness's job when it runs the dataset, not adept's when it defines
/// the shape.
///
/// # Errors
/// Returns the first [`EvalError`] encountered.
pub fn validate(text: &str) -> Result<(), EvalError> {
    let cases = parse_jsonl_lines(text)?;
    validate_parsed(&cases)
}

/// Parse `text` once and validate the result, returning the parsed cases.
///
/// Equivalent to calling [`validate`] then [`parse_jsonl`], but without the
/// double parse/allocation that pair would otherwise cause — every line is
/// deserialized exactly once. `schema_version` errors still name the real
/// 1-indexed source line, same as [`validate`].
///
/// # Errors
/// Returns the first [`EvalError`] encountered (a parse failure, an
/// unsupported `schema_version`, or an empty dataset).
pub fn parse_and_validate(text: &str) -> Result<Vec<EvalCase>, EvalError> {
    let numbered = parse_jsonl_lines(text)?;
    validate_parsed(&numbered)?;
    Ok(numbered.into_iter().map(|(_, case)| case).collect())
}

/// Validate already-parsed `cases` in memory, without serializing them to
/// JSONL and reparsing — the same checks [`validate`] performs (every
/// `schema_version` understood, every `id` non-empty and unique, dataset
/// non-empty), for a caller that just built or already parsed its cases and
/// doesn't need the string round trip.
/// Line numbers are reported as 1-indexed positions in `cases` (there is no
/// source text to point at).
///
/// [`validate`] delegates to the same underlying check
/// ([`validate_parsed`]), so the two can never disagree about what is valid.
///
/// # Errors
/// Returns the first [`EvalError`] encountered.
pub fn validate_cases(cases: &[EvalCase]) -> Result<(), EvalError> {
    let numbered: Vec<(usize, &EvalCase)> =
        cases.iter().enumerate().map(|(i, c)| (i + 1, c)).collect();
    validate_parsed_refs(&numbered)
}

/// Shared check used by both [`validate`] and [`validate_cases`]: every
/// case's `schema_version` is understood, every `id` is non-empty and
/// unique, and the dataset is non-empty.
fn validate_parsed(cases: &[(usize, EvalCase)]) -> Result<(), EvalError> {
    let numbered: Vec<(usize, &EvalCase)> =
        cases.iter().map(|(line, case)| (*line, case)).collect();
    validate_parsed_refs(&numbered)
}

/// Reference-based core of [`validate_parsed`] / [`validate_cases`].
fn validate_parsed_refs(cases: &[(usize, &EvalCase)]) -> Result<(), EvalError> {
    for (line, case) in cases {
        if case.schema_version != SCHEMA_VERSION {
            return Err(EvalError::UnsupportedSchemaVersion {
                line: *line,
                found: case.schema_version,
            });
        }
    }
    for (line, case) in cases {
        if case.id.is_empty() {
            return Err(EvalError::EmptyId { line: *line });
        }
    }
    let mut seen_ids: HashSet<&str> = HashSet::new();
    for (line, case) in cases {
        if !seen_ids.insert(case.id.as_str()) {
            return Err(EvalError::DuplicateId {
                line: *line,
                id: case.id.clone(),
            });
        }
    }
    if cases.is_empty() {
        return Err(EvalError::Empty);
    }
    Ok(())
}

/// Which arm of a comparison a [`CaseResult`] was produced under.
///
/// `Skill` (the default, so a results file that never mentions arms just
/// works) is the run under test; `Baseline` is what makes skill lift
/// computable in [`grade`] — omitted, not zeroed, when no baseline results
/// are present at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Arm {
    /// The skill under test.
    #[default]
    Skill,
    /// A baseline run (e.g. the same prompt without the skill available),
    /// used only to compute lift.
    Baseline,
}

/// Token counts reported by a harness for one case run, as `{"in": N, "out": N}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Input (prompt) tokens.
    #[serde(rename = "in")]
    pub input: u64,
    /// Output (completion) tokens.
    #[serde(rename = "out")]
    pub output: u64,
}

/// One line of a harness-produced `results.jsonl` sidecar: what actually
/// happened when a harness ran one [`EvalCase`].
///
/// This is a separate, separately-versioned format from the eval dataset
/// itself (see the module docs) — it is not an [`EvalCase`] and carries no
/// `schema_version`, since it is produced fresh by a harness for each run
/// rather than authored and maintained like a dataset.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CaseResult {
    /// Which dataset case this result is for, by [`EvalCase::id`]. An `id`
    /// not present in the dataset is reported (see
    /// [`EvalBenchmarkReport::unknown_result_ids`]), never graded against an
    /// arbitrary case.
    pub id: String,
    /// `"skill"` (default) or `"baseline"`.
    #[serde(default)]
    pub arm: Arm,
    /// The agent's response text, graded by [`Assertion::Contains`].
    pub response: String,
    /// Working directory the case ran in. `file_exists`/`file_contains`
    /// paths resolve against it; absent means those assertions are
    /// [`AssertionOutcome::Skipped`].
    #[serde(default)]
    pub cwd: Option<String>,
    /// Map of command string to observed exit code, as reported by the
    /// harness (adept never runs a `command` assertion itself). A
    /// `command` assertion with no entry here is
    /// [`AssertionOutcome::Skipped`].
    #[serde(default)]
    pub command_exit_codes: HashMap<String, i32>,
    /// Token usage for this run, when the harness reports it.
    #[serde(default)]
    pub tokens: Option<TokenUsage>,
}

/// Parse a JSONL `results.jsonl` sidecar from `text`, one [`CaseResult`] per
/// line. Blank lines are skipped, matching [`parse_jsonl`]'s behaviour for
/// datasets.
///
/// # Errors
/// Returns [`EvalError::Parse`] naming the 1-indexed line number of the
/// first non-blank line that fails to parse as a [`CaseResult`].
pub fn parse_results_jsonl(text: &str) -> Result<Vec<CaseResult>, EvalError> {
    Ok(parse_jsonl_lines(text)?
        .into_iter()
        .map(|(_, result)| result)
        .collect())
}

/// The outcome of grading a single assertion against a [`CaseResult`].
///
/// A [`Skipped`](AssertionOutcome::Skipped) outcome is never a pass — it
/// means adept could not check the assertion at all (no `cwd`, no reported
/// exit code), which is different from checking it and finding it false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionOutcome {
    /// The assertion was checked and held.
    Pass,
    /// The assertion was checked and did not hold.
    Fail,
    /// The assertion could not be checked; see the paired reason.
    Skipped,
}

/// The graded outcome of one [`Assertion`] within a case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssertionResult {
    /// The assertion's `kind` discriminant (see [`Assertion::kind`]).
    pub kind: String,
    /// Pass, fail, or skipped.
    pub outcome: AssertionOutcome,
    /// Present when `outcome` is [`Skipped`](AssertionOutcome::Skipped)
    /// (why it could not be checked, e.g. "no cwd supplied") or when a
    /// [`Fail`](AssertionOutcome::Fail) needs more explanation than a bare
    /// boolean gives (e.g. a `path` that escaped `cwd`).
    pub detail: Option<String>,
}

/// The graded outcome of one [`CaseResult`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseReport {
    /// The dataset case this result graded, by [`EvalCase::id`].
    pub case: String,
    /// Which arm this result was for.
    pub arm: Arm,
    /// `true` only if every non-skipped assertion passed **and** at least
    /// one assertion was actually checked (see the module-level grading
    /// rules) — an all-skipped case is never reported as passing.
    pub pass: bool,
    /// Per-assertion outcomes, in dataset order.
    pub assertions: Vec<AssertionResult>,
}

/// The full report produced by [`grade`]: per-case outcomes plus aggregate
/// metrics, in the spirit of huggingface/upskill's `upskill eval`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EvalBenchmarkReport {
    /// One entry per graded [`CaseResult`] (not per dataset case — a case
    /// with both a skill and a baseline result produces two entries).
    pub cases: Vec<CaseReport>,
    /// Fraction of `skill`-arm results that passed (0.0 if there were none).
    pub pass_rate: f64,
    /// Assertions met divided by assertions checked, across all graded
    /// results. Skipped assertions are excluded from both numerator and
    /// denominator.
    pub assertion_success_rate: f64,
    /// Total assertions actually checked (i.e. not skipped).
    pub assertions_checked: usize,
    /// Of `assertions_checked`, how many passed.
    pub assertions_met: usize,
    /// Total assertions skipped, across all graded results.
    pub assertions_skipped: usize,
    /// Skip reason to count, so a run that silently graded nothing is
    /// visible rather than looking like a perfect score. A `BTreeMap` so
    /// rendering iterates in a stable (sorted-by-reason) order rather than
    /// whatever order a `HashMap` happens to produce.
    pub skipped_reasons: BTreeMap<String, usize>,
    /// Baseline arm's pass rate, present only when at least one `baseline`
    /// result was graded.
    pub baseline_pass_rate: Option<f64>,
    /// `pass_rate - baseline_pass_rate`, in percentage points. Omitted
    /// (not zeroed) when there is no baseline arm.
    pub lift_percentage_points: Option<f64>,
    /// Sum of input tokens across results that reported [`TokenUsage`].
    pub tokens_in: Option<u64>,
    /// Sum of output tokens across results that reported [`TokenUsage`].
    pub tokens_out: Option<u64>,
    /// [`CaseResult::id`] values from results that named an id absent from
    /// the dataset. Such a result is reported here, not graded against an
    /// arbitrary case. Sorted and deduplicated, so output is stable
    /// regardless of `results` order and repeated bad ids collapse to one
    /// entry.
    pub unknown_result_ids: Vec<String>,
    /// Dataset case ids that had no `skill`-arm result at all.
    pub unmatched_cases: Vec<String>,
    /// Dataset case ids that are ambiguous — either empty, or shared by more
    /// than one case — as found among the `cases` passed to [`grade`]. A
    /// result naming one of these ids is never graded against an arbitrary
    /// member of the ambiguous set; it is reported here instead. Sorted and
    /// deduplicated, so output is stable regardless of `cases` order. This
    /// is distinct from [`Self::unknown_result_ids`]: "no such id" and "more
    /// than one such id" are different diagnoses a harness author needs to
    /// tell apart.
    pub ambiguous_case_ids: Vec<String>,
}

/// Grade `results` (typically parsed via [`parse_results_jsonl`]) against
/// `cases` (typically parsed via [`parse_jsonl`]/[`validate`]).
///
/// Results are resolved to a dataset case through an id → index map built
/// from `cases`, so reordering `cases` never changes which case a given
/// result grades against. A result whose `id` is absent from the dataset is
/// reported in [`EvalBenchmarkReport::unknown_result_ids`], never graded
/// against an arbitrary case.
///
/// `grade` does not trust its caller to have run [`validate`] first: if
/// `cases` itself contains an empty id, or an id shared by more than one
/// case, that id is ambiguous and is never used to resolve a result. Such
/// ids are collected into [`EvalBenchmarkReport::ambiguous_case_ids`]
/// instead, and any result naming one is reported there rather than graded
/// against an arbitrary member of the duplicate set.
///
/// Purely offline and deterministic: a substring match, filesystem reads
/// resolved against each result's `cwd`, and a lookup into
/// `command_exit_codes` — adept never spawns the `command` itself. See the
/// module docs and `docs/EVALS.md` for the full division of labour.
#[must_use]
pub fn grade(cases: &[EvalCase], results: &[CaseResult]) -> EvalBenchmarkReport {
    let mut report = EvalBenchmarkReport::default();

    // One pass builds the whole id table: `Some(index)` is a usable id,
    // `None` is an ambiguous one (empty, or claimed by more than one case).
    // Ambiguity is sticky — a second claim demotes the entry to `None` and
    // nothing can promote it back, so no result is ever graded against an
    // arbitrary member of a duplicate set.
    let mut id_to_index: HashMap<&str, Option<usize>> = HashMap::new();
    for (idx, case) in cases.iter().enumerate() {
        let id = case.id.as_str();
        let resolved = if id.is_empty() { None } else { Some(idx) };
        id_to_index
            .entry(id)
            .and_modify(|slot| *slot = None)
            .or_insert(resolved);
    }
    report.ambiguous_case_ids = id_to_index
        .iter()
        .filter(|(_, slot)| slot.is_none())
        .map(|(id, _)| (*id).to_string())
        .collect();
    report.ambiguous_case_ids.sort();
    let mut skill_seen = vec![false; cases.len()];
    let (mut skill_pass, mut skill_total) = (0usize, 0usize);
    let (mut baseline_pass, mut baseline_total) = (0usize, 0usize);
    let (mut tokens_in, mut tokens_out) = (0u64, 0u64);
    let mut any_tokens = false;

    for result in results {
        let index = match id_to_index.get(result.id.as_str()) {
            Some(Some(index)) => *index,
            // Ambiguous: already surfaced in `ambiguous_case_ids`, so refuse
            // to grade against an arbitrary member of the duplicate set.
            Some(None) => continue,
            None => {
                report.unknown_result_ids.push(result.id.clone());
                continue;
            }
        };
        let case = &cases[index];
        let (case_report, grading) = grade_case(case, result);

        match result.arm {
            Arm::Skill => {
                skill_seen[index] = true;
                skill_total += 1;
                if case_report.pass {
                    skill_pass += 1;
                }
                report.assertions_checked += grading.checked;
                report.assertions_met += grading.met;
                report.assertions_skipped += grading.skipped;
                for (reason, count) in grading.skipped_reasons {
                    *report.skipped_reasons.entry(reason).or_insert(0) += count;
                }
            }
            Arm::Baseline => {
                baseline_total += 1;
                if case_report.pass {
                    baseline_pass += 1;
                }
            }
        }

        if let Some(tokens) = &result.tokens {
            any_tokens = true;
            tokens_in += tokens.input;
            tokens_out += tokens.output;
        }

        report.cases.push(case_report);
    }

    for (idx, seen) in skill_seen.iter().enumerate() {
        // An ambiguous id is reported as ambiguous, not additionally as
        // unmatched — `id_to_index` holds `None` for exactly those.
        if !seen && id_to_index.get(cases[idx].id.as_str()) != Some(&None) {
            report.unmatched_cases.push(cases[idx].id.clone());
        }
    }

    report.pass_rate = if skill_total > 0 {
        skill_pass as f64 / skill_total as f64
    } else {
        0.0
    };
    report.assertion_success_rate = if report.assertions_checked > 0 {
        report.assertions_met as f64 / report.assertions_checked as f64
    } else {
        0.0
    };
    if baseline_total > 0 {
        let baseline_rate = baseline_pass as f64 / baseline_total as f64;
        report.baseline_pass_rate = Some(baseline_rate);
        report.lift_percentage_points = Some((report.pass_rate - baseline_rate) * 100.0);
    }
    if any_tokens {
        report.tokens_in = Some(tokens_in);
        report.tokens_out = Some(tokens_out);
    }
    report.unknown_result_ids.sort();
    report.unknown_result_ids.dedup();

    report
}

/// Per-result assertion tallies returned by [`grade_case`], folded into the
/// aggregate report by the caller only for `skill`-arm results — the
/// baseline arm exists solely to make lift computable and must not inflate
/// the headline assertion metrics.
struct CaseGrading {
    checked: usize,
    met: usize,
    skipped: usize,
    skipped_reasons: HashMap<String, usize>,
}

/// Grade every assertion of `case` against `result`, returning the per-case
/// outcome plus its checked/met/skipped tallies (left for the caller to
/// fold into the aggregate report, arm-conditionally).
fn grade_case(case: &EvalCase, result: &CaseResult) -> (CaseReport, CaseGrading) {
    let mut assertions = Vec::with_capacity(case.assertions.len());
    let mut checked = 0usize;
    let mut met = 0usize;
    let mut skipped = 0usize;
    let mut skipped_reasons: HashMap<String, usize> = HashMap::new();

    for assertion in &case.assertions {
        let (outcome, detail) = grade_assertion(assertion, result);
        match outcome {
            AssertionOutcome::Pass => {
                checked += 1;
                met += 1;
            }
            AssertionOutcome::Fail => {
                checked += 1;
            }
            AssertionOutcome::Skipped => {
                skipped += 1;
                let reason = detail.clone().unwrap_or_else(|| "skipped".to_string());
                *skipped_reasons.entry(reason).or_insert(0) += 1;
            }
        }
        assertions.push(AssertionResult {
            kind: assertion.kind().to_string(),
            outcome,
            detail,
        });
    }

    // Pass only if every non-skipped assertion passed AND at least one
    // assertion was actually checked — this is what keeps an all-skipped
    // case from silently looking like a perfect pass.
    let pass = checked > 0 && met == checked;

    (
        CaseReport {
            case: result.id.clone(),
            arm: result.arm,
            pass,
            assertions,
        },
        CaseGrading {
            checked,
            met,
            skipped,
            skipped_reasons,
        },
    )
}

/// Grade a single assertion against a result, returning its outcome and an
/// optional detail (skip reason, or extra context on a failure).
fn grade_assertion(
    assertion: &Assertion,
    result: &CaseResult,
) -> (AssertionOutcome, Option<String>) {
    match assertion {
        Assertion::Contains { value } => {
            if result.response.contains(value.as_str()) {
                (AssertionOutcome::Pass, None)
            } else {
                (AssertionOutcome::Fail, None)
            }
        }
        Assertion::FileExists { path } => match resolve_case_path(result, path) {
            Ok(None) => (
                AssertionOutcome::Skipped,
                Some("no cwd supplied".to_string()),
            ),
            Ok(Some(full)) => {
                if full.is_file() {
                    (AssertionOutcome::Pass, None)
                } else {
                    (
                        AssertionOutcome::Fail,
                        Some(format!("{} does not exist", path)),
                    )
                }
            }
            Err(reason) => (AssertionOutcome::Fail, Some(reason)),
        },
        Assertion::FileContains { path, value } => match resolve_case_path(result, path) {
            Ok(None) => (
                AssertionOutcome::Skipped,
                Some("no cwd supplied".to_string()),
            ),
            Ok(Some(full)) => match std::fs::read_to_string(&full) {
                Ok(contents) if contents.contains(value.as_str()) => (AssertionOutcome::Pass, None),
                Ok(_) => (
                    AssertionOutcome::Fail,
                    Some(format!("{} does not contain expected value", path)),
                ),
                Err(err) => (
                    AssertionOutcome::Fail,
                    Some(format!("could not read {}: {}", path, err)),
                ),
            },
            Err(reason) => (AssertionOutcome::Fail, Some(reason)),
        },
        Assertion::Command { command } => match result.command_exit_codes.get(command) {
            None => (
                AssertionOutcome::Skipped,
                Some("no exit code reported for command".to_string()),
            ),
            Some(0) => (AssertionOutcome::Pass, None),
            Some(code) => (AssertionOutcome::Fail, Some(format!("exit code {code}"))),
        },
    }
}

/// Resolve `path` against `result.cwd`, if present.
///
/// Returns `Ok(None)` when `result` has no `cwd` (the assertion should be
/// skipped, not failed), `Ok(Some(full_path))` when `path` resolves
/// strictly inside `cwd`, and `Err(reason)` when `path` escapes it (an
/// absolute path, or any `..`/`.` component).
fn resolve_case_path(result: &CaseResult, path: &str) -> Result<Option<PathBuf>, String> {
    match &result.cwd {
        None => Ok(None),
        Some(cwd) => safe_join_within(Path::new(cwd), path).map(Some),
    }
}

/// Join `rel` onto `base`, rejecting anything that could escape `base`.
///
/// This is deliberately **not** `adept_agent::candidate::resolve_companion_path`:
/// that helper requires the target be a *direct child* of its directory,
/// which is the right rule for a skill's companion files but wrong here —
/// eval assertion paths are commonly nested (e.g. `src/out.txt` written by
/// a multi-file skill run). This helper instead allows any number of plain
/// (`Normal`) path components and rejects only absolute paths and `..`/`.`
/// components, so nested paths are fine but nothing can walk outside `cwd`.
fn safe_join_within(base: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel_path = Path::new(rel);
    for component in rel_path.components() {
        match component {
            Component::Normal(_) => {}
            other => {
                return Err(format!(
                    "path {rel:?} is not allowed: contains disallowed component {other:?}"
                ))
            }
        }
    }
    Ok(base.join(rel_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_case() -> EvalCase {
        EvalCase {
            schema_version: SCHEMA_VERSION,
            id: "summarize-the-attached-report".to_string(),
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
        let text = r#"{"schema_version":2,"id":"x","prompt":"p","assertions":[{"kind":"unheard_of","value":"x"}]}"#;
        let err = parse_jsonl(text).unwrap_err();
        assert!(matches!(err, EvalError::Parse { line: 1, .. }));
    }

    #[test]
    fn validate_rejects_unsupported_schema_version() {
        let text = r#"{"schema_version":999,"id":"x","prompt":"p","assertions":[]}"#;
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
    fn validate_cases_agrees_with_validate() {
        let cases = vec![sample_case()];
        validate_cases(&cases).unwrap();
        validate(&to_jsonl(&cases)).unwrap();

        let empty: Vec<EvalCase> = Vec::new();
        assert!(matches!(
            validate_cases(&empty).unwrap_err(),
            EvalError::Empty
        ));

        let mut bad = sample_case();
        bad.schema_version = 999;
        match validate_cases(&[bad]).unwrap_err() {
            EvalError::UnsupportedSchemaVersion { line, found } => {
                assert_eq!(line, 1);
                assert_eq!(found, 999);
            }
            other => panic!("expected UnsupportedSchemaVersion, got {other:?}"),
        }
    }

    #[test]
    fn missing_required_field_is_a_parse_error() {
        let text = r#"{"schema_version":2,"id":"x","assertions":[]}"#; // missing `prompt`
        let err = parse_jsonl(text).unwrap_err();
        assert!(matches!(err, EvalError::Parse { line: 1, .. }));
    }

    #[test]
    fn validate_rejects_schema_version_1_dataset() {
        // Clean break: a v1 dataset (no `id`, and `schema_version: 1`) is
        // rejected via the existing UnsupportedSchemaVersion path, not
        // migrated or given auto-generated ids.
        let text = r#"{"schema_version":1,"prompt":"p","assertions":[]}"#;
        let err = validate(text).unwrap_err();
        match err {
            EvalError::UnsupportedSchemaVersion { line, found } => {
                assert_eq!(line, 1);
                assert_eq!(found, 1);
            }
            other => panic!("expected UnsupportedSchemaVersion, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_empty_id_naming_the_line() {
        let good = serde_json::to_string(&sample_case()).unwrap();
        let mut empty_id = sample_case();
        empty_id.id = String::new();
        let bad = serde_json::to_string(&empty_id).unwrap();
        let text = format!("{good}\n{bad}\n");
        let err = validate(&text).unwrap_err();
        match err {
            EvalError::EmptyId { line } => assert_eq!(line, 2),
            other => panic!("expected EmptyId, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_duplicate_ids_naming_the_line() {
        let mut first = sample_case();
        first.id = "dup".to_string();
        let mut second = sample_case();
        second.id = "dup".to_string();
        let text = to_jsonl(&[first, second]);
        let err = validate(&text).unwrap_err();
        match err {
            EvalError::DuplicateId { line, id } => {
                assert_eq!(line, 2);
                assert_eq!(id, "dup");
            }
            other => panic!("expected DuplicateId, got {other:?}"),
        }
    }

    // --- grading tests ---

    fn contains_case(id: &str, value: &str) -> EvalCase {
        EvalCase {
            schema_version: SCHEMA_VERSION,
            id: id.to_string(),
            prompt: "do the thing".to_string(),
            assertions: vec![Assertion::Contains {
                value: value.to_string(),
            }],
        }
    }

    fn skill_result(id: &str, response: &str) -> CaseResult {
        CaseResult {
            id: id.to_string(),
            arm: Arm::Skill,
            response: response.to_string(),
            cwd: None,
            command_exit_codes: HashMap::new(),
            tokens: None,
        }
    }

    #[test]
    fn parses_results_jsonl_skipping_blanks() {
        let text = "{\"id\":\"a\",\"response\":\"ok\"}\n\n{\"id\":\"b\",\"response\":\"ok2\",\"arm\":\"baseline\"}\n";
        let results = parse_results_jsonl(text).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "a");
        assert_eq!(results[0].arm, Arm::Skill);
        assert_eq!(results[1].arm, Arm::Baseline);
    }

    #[test]
    fn parse_results_jsonl_reports_offending_line() {
        let text = "{\"id\":\"a\",\"response\":\"ok\"}\nnot json\n";
        let err = parse_results_jsonl(text).unwrap_err();
        match err {
            EvalError::Parse { line, .. } => assert_eq!(line, 2),
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn parse_results_jsonl_missing_id_is_a_parse_error() {
        let text = "{\"response\":\"ok\"}\n";
        let err = parse_results_jsonl(text).unwrap_err();
        assert!(matches!(err, EvalError::Parse { line: 1, .. }));
    }

    #[test]
    fn grade_all_pass() {
        let cases = vec![contains_case("a", "hello"), contains_case("b", "world")];
        let results = vec![
            skill_result("a", "hello there"),
            skill_result("b", "big world"),
        ];
        let report = grade(&cases, &results);
        assert_eq!(report.pass_rate, 1.0);
        assert_eq!(report.assertion_success_rate, 1.0);
        assert!(report.unmatched_cases.is_empty());
        assert!(report.unknown_result_ids.is_empty());
    }

    #[test]
    fn grade_all_fail() {
        let cases = vec![contains_case("a", "hello"), contains_case("b", "world")];
        let results = vec![skill_result("a", "nope"), skill_result("b", "nada")];
        let report = grade(&cases, &results);
        assert_eq!(report.pass_rate, 0.0);
        assert_eq!(report.assertion_success_rate, 0.0);
        assert!(report.cases.iter().all(|c| !c.pass));
    }

    #[test]
    fn grade_mixed() {
        let cases = vec![contains_case("a", "hello"), contains_case("b", "world")];
        let results = vec![skill_result("a", "hello there"), skill_result("b", "nada")];
        let report = grade(&cases, &results);
        assert_eq!(report.pass_rate, 0.5);
    }

    #[test]
    fn grade_baseline_and_skill_arms_computes_lift() {
        let cases = vec![contains_case("a", "hello"), contains_case("b", "world")];
        let mut baseline1 = skill_result("a", "nope");
        baseline1.arm = Arm::Baseline;
        let mut baseline2 = skill_result("b", "nada");
        baseline2.arm = Arm::Baseline;
        let results = vec![
            skill_result("a", "hello there"),
            skill_result("b", "big world"),
            baseline1,
            baseline2,
        ];
        let report = grade(&cases, &results);
        assert_eq!(report.pass_rate, 1.0);
        assert_eq!(report.baseline_pass_rate, Some(0.0));
        assert_eq!(report.lift_percentage_points, Some(100.0));
    }

    #[test]
    fn grade_baseline_assertions_excluded_from_aggregate_metrics() {
        // Skill arm: 1 case, 1 assertion, which passes.
        // Baseline arm: 1 case, 1 assertion, which fails (different value
        // than what the skill's `Contains` assertion checks) — if the
        // baseline arm's assertion leaked into the aggregate, the
        // denominator would be 2 instead of 1 and `assertions_met` would
        // still be 1, giving a different (wrong) success rate.
        let cases = vec![contains_case("a", "hello")];
        let mut baseline = skill_result("a", "goodbye");
        baseline.arm = Arm::Baseline;
        let results = vec![skill_result("a", "hello there"), baseline];

        let report = grade(&cases, &results);

        assert_eq!(report.assertions_checked, 1);
        assert_eq!(report.assertions_met, 1);
        assert_eq!(report.assertions_skipped, 0);
        assert!(report.skipped_reasons.is_empty());
        // Baseline-arm detail is still available per-case.
        assert_eq!(report.cases.len(), 2);
        assert!(report
            .cases
            .iter()
            .any(|c| c.arm == Arm::Baseline && !c.pass));
    }

    #[test]
    fn grade_skill_arm_only_omits_lift() {
        let cases = vec![contains_case("a", "hello")];
        let results = vec![skill_result("a", "hello there")];
        let report = grade(&cases, &results);
        assert_eq!(report.baseline_pass_rate, None);
        assert_eq!(report.lift_percentage_points, None);
    }

    #[test]
    fn grade_every_assertion_kind_via_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("out")).unwrap();
        std::fs::write(dir.path().join("out/summary.md"), "the conclusion is X").unwrap();

        let case = EvalCase {
            schema_version: SCHEMA_VERSION,
            id: "a".to_string(),
            prompt: "p".to_string(),
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
        };
        let mut command_exit_codes = HashMap::new();
        command_exit_codes.insert("test -s out/summary.md".to_string(), 0);
        let result = CaseResult {
            id: "a".to_string(),
            arm: Arm::Skill,
            response: "here is your summary".to_string(),
            cwd: Some(dir.path().to_string_lossy().to_string()),
            command_exit_codes,
            tokens: Some(TokenUsage {
                input: 10,
                output: 20,
            }),
        };

        let report = grade(std::slice::from_ref(&case), std::slice::from_ref(&result));
        assert_eq!(report.pass_rate, 1.0);
        assert_eq!(report.assertions_checked, 4);
        assert_eq!(report.assertions_met, 4);
        assert_eq!(report.assertions_skipped, 0);
        assert_eq!(report.tokens_in, Some(10));
        assert_eq!(report.tokens_out, Some(20));
    }

    #[test]
    fn grade_skip_reasons_command_without_exit_code_and_file_without_cwd() {
        let case = EvalCase {
            schema_version: SCHEMA_VERSION,
            id: "a".to_string(),
            prompt: "p".to_string(),
            assertions: vec![
                Assertion::FileExists {
                    path: "out.txt".to_string(),
                },
                Assertion::FileContains {
                    path: "out.txt".to_string(),
                    value: "x".to_string(),
                },
                Assertion::Command {
                    command: "true".to_string(),
                },
            ],
        };
        let result = skill_result("a", "response");
        let report = grade(std::slice::from_ref(&case), std::slice::from_ref(&result));
        assert_eq!(report.assertions_checked, 0);
        assert_eq!(report.assertions_skipped, 3);
        assert_eq!(
            report.skipped_reasons.get("no cwd supplied").copied(),
            Some(2)
        );
        assert_eq!(
            report
                .skipped_reasons
                .get("no exit code reported for command")
                .copied(),
            Some(1)
        );
        // `skipped_reasons` is a `BTreeMap`, so with two or more distinct
        // reasons the iteration order is deterministic (sorted by reason
        // text) rather than whatever a `HashMap` happens to produce — this
        // is what keeps a rendered report's skip-reason lines stable across
        // runs instead of flaking.
        let reasons: Vec<&str> = report.skipped_reasons.keys().map(String::as_str).collect();
        assert_eq!(
            reasons,
            vec!["no cwd supplied", "no exit code reported for command"]
        );
    }

    #[test]
    fn all_skipped_case_is_not_reported_as_passing() {
        // The load-bearing rule: a case whose only assertions were
        // skipped must never come out as `pass == true`, or grading would
        // silently look like a perfect score while checking nothing.
        let case = EvalCase {
            schema_version: SCHEMA_VERSION,
            id: "a".to_string(),
            prompt: "p".to_string(),
            assertions: vec![Assertion::Command {
                command: "true".to_string(),
            }],
        };
        let result = skill_result("a", "response");
        let report = grade(std::slice::from_ref(&case), std::slice::from_ref(&result));
        assert_eq!(report.cases.len(), 1);
        assert!(!report.cases[0].pass);
        assert_eq!(report.pass_rate, 0.0);
    }

    #[test]
    fn grade_reports_unknown_result_id_and_does_not_grade_it() {
        let cases = vec![contains_case("a", "hello")];
        let results = vec![skill_result("nonexistent", "hello")];
        let report = grade(&cases, &results);
        assert_eq!(report.unknown_result_ids, vec!["nonexistent".to_string()]);
        assert!(report.cases.is_empty());
        // Not graded against the one case that does exist either.
        assert_eq!(report.pass_rate, 0.0);
    }

    #[test]
    fn grade_deduplicates_and_sorts_unknown_result_ids() {
        // Ten results naming the same bad id must not yield ten identical
        // entries, and entries must come back sorted regardless of the
        // order results named them in — matching `ambiguous_case_ids`'s
        // guarantee.
        let cases = vec![contains_case("a", "hello")];
        let results = vec![
            skill_result("zzz", "hello"),
            skill_result("nonexistent", "hello"),
            skill_result("nonexistent", "hello"),
            skill_result("aaa", "hello"),
            skill_result("zzz", "hello"),
        ];
        let report = grade(&cases, &results);
        assert_eq!(
            report.unknown_result_ids,
            vec![
                "aaa".to_string(),
                "nonexistent".to_string(),
                "zzz".to_string()
            ]
        );
    }

    #[test]
    fn grade_reports_dataset_case_with_no_result() {
        let cases = vec![contains_case("a", "hello"), contains_case("b", "world")];
        let results = vec![skill_result("a", "hello there")];
        let report = grade(&cases, &results);
        assert_eq!(report.unmatched_cases, vec!["b".to_string()]);
    }

    #[test]
    fn reordering_dataset_lines_does_not_change_any_cases_grade() {
        // The central regression test for this change: identity is by `id`,
        // not position, so grading the same results against a dataset whose
        // case order has been shuffled must produce the exact same
        // per-case, per-id outcomes.
        let forward = vec![
            contains_case("a", "hello"),
            contains_case("b", "world"),
            contains_case("c", "goodbye"),
        ];
        let reordered = vec![
            contains_case("c", "goodbye"),
            contains_case("a", "hello"),
            contains_case("b", "world"),
        ];
        let results = vec![
            skill_result("a", "hello there"),
            skill_result("b", "nada"), // fails
            skill_result("c", "goodbye now"),
        ];

        let forward_report = grade(&forward, &results);
        let reordered_report = grade(&reordered, &results);

        let as_map = |report: &EvalBenchmarkReport| -> BTreeMap<String, bool> {
            report
                .cases
                .iter()
                .map(|c| (c.case.clone(), c.pass))
                .collect()
        };
        assert_eq!(as_map(&forward_report), as_map(&reordered_report));
        assert_eq!(forward_report.pass_rate, reordered_report.pass_rate);
        assert!(forward_report.unmatched_cases.is_empty());
        assert!(reordered_report.unmatched_cases.is_empty());

        // Full per-assertion outcome vectors, not just the pass bit, so a
        // reordering bug that flipped an individual assertion while
        // preserving the overall pass/fail result would still be caught.
        let assertions_by_case =
            |report: &EvalBenchmarkReport| -> BTreeMap<String, Vec<AssertionResult>> {
                report
                    .cases
                    .iter()
                    .map(|c| (c.case.clone(), c.assertions.clone()))
                    .collect()
            };
        assert_eq!(
            assertions_by_case(&forward_report),
            assertions_by_case(&reordered_report)
        );
    }

    #[test]
    fn grade_reports_duplicate_ids_as_ambiguous_and_grades_neither_case() {
        // Two cases sharing an id (a dataset that skipped `validate`): both
        // must be reported as ambiguous, and a result naming that id must
        // not be silently graded against whichever one `HashMap::insert`
        // happened to keep last.
        let cases = vec![contains_case("a", "hello"), contains_case("a", "goodbye")];
        let results = vec![skill_result("a", "hello there")];
        let report = grade(&cases, &results);
        assert_eq!(report.ambiguous_case_ids, vec!["a".to_string()]);
        assert!(report.cases.is_empty());
        assert_eq!(report.pass_rate, 0.0);
        assert!(report.unmatched_cases.is_empty());
        assert!(report.unknown_result_ids.is_empty());
    }

    #[test]
    fn grade_reports_empty_ids_as_ambiguous_and_grades_neither_case() {
        // Even a single case with an empty id is not a safe grading target:
        // it is not a case-identifying id at all.
        let cases = vec![contains_case("", "hello"), contains_case("b", "world")];
        let results = vec![skill_result("", "hello there"), skill_result("b", "world")];
        let report = grade(&cases, &results);
        assert_eq!(report.ambiguous_case_ids, vec!["".to_string()]);
        assert_eq!(report.cases.len(), 1);
        assert_eq!(report.cases[0].case, "b");
        assert!(report.unknown_result_ids.is_empty());
    }

    #[test]
    fn grade_normal_path_is_unaffected_by_ambiguity_defense() {
        let cases = vec![contains_case("a", "hello"), contains_case("b", "world")];
        let results = vec![
            skill_result("a", "hello there"),
            skill_result("b", "world wide"),
        ];
        let report = grade(&cases, &results);
        assert!(report.ambiguous_case_ids.is_empty());
        assert_eq!(report.cases.len(), 2);
        assert_eq!(report.pass_rate, 1.0);
        assert!(report.unmatched_cases.is_empty());
        assert!(report.unknown_result_ids.is_empty());
    }

    #[test]
    fn grade_rejects_path_escaping_cwd_with_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let case = EvalCase {
            schema_version: SCHEMA_VERSION,
            id: "a".to_string(),
            prompt: "p".to_string(),
            assertions: vec![Assertion::FileExists {
                path: "/etc/passwd".to_string(),
            }],
        };
        let result = CaseResult {
            id: "a".to_string(),
            arm: Arm::Skill,
            response: "r".to_string(),
            cwd: Some(dir.path().to_string_lossy().to_string()),
            command_exit_codes: HashMap::new(),
            tokens: None,
        };
        let report = grade(std::slice::from_ref(&case), std::slice::from_ref(&result));
        assert!(!report.cases[0].pass);
        let assertion = &report.cases[0].assertions[0];
        assert_eq!(assertion.outcome, AssertionOutcome::Fail);
        assert!(assertion.detail.is_some());
    }

    #[test]
    fn grade_rejects_path_escaping_cwd_with_dotdot() {
        let dir = tempfile::tempdir().unwrap();
        let case = EvalCase {
            schema_version: SCHEMA_VERSION,
            id: "a".to_string(),
            prompt: "p".to_string(),
            assertions: vec![Assertion::FileExists {
                path: "../escape.txt".to_string(),
            }],
        };
        let result = CaseResult {
            id: "a".to_string(),
            arm: Arm::Skill,
            response: "r".to_string(),
            cwd: Some(dir.path().to_string_lossy().to_string()),
            command_exit_codes: HashMap::new(),
            tokens: None,
        };
        let report = grade(std::slice::from_ref(&case), std::slice::from_ref(&result));
        assert!(!report.cases[0].pass);
        assert_eq!(
            report.cases[0].assertions[0].outcome,
            AssertionOutcome::Fail
        );
    }

    #[test]
    fn grade_allows_nested_paths_within_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/out.txt"), "content").unwrap();
        let case = EvalCase {
            schema_version: SCHEMA_VERSION,
            id: "a".to_string(),
            prompt: "p".to_string(),
            assertions: vec![Assertion::FileExists {
                path: "src/out.txt".to_string(),
            }],
        };
        let result = CaseResult {
            id: "a".to_string(),
            arm: Arm::Skill,
            response: "r".to_string(),
            cwd: Some(dir.path().to_string_lossy().to_string()),
            command_exit_codes: HashMap::new(),
            tokens: None,
        };
        let report = grade(std::slice::from_ref(&case), std::slice::from_ref(&result));
        assert!(report.cases[0].pass);
    }
}
