//! LLM-assisted lint autofix (`adept fix`) for Agent Skills.
//!
//! Like `adept_score`, the async seam is [`adept_score::LlmClient`]:
//! everything here that talks to a model goes through a `&dyn LlmClient`,
//! so callers can pass `adept_score::OpenAiCompatClient` for real fixing or
//! `adept_score::MockLlmClient` for fully offline tests. The single public
//! entry point, [`fix_skill`], is `async fn`; callers are expected to drive
//! it from a `tokio` runtime, same as `adept_score::score_skill`.
//!
//! [`fix_skill`] only *computes* a candidate rewrite (a [`FixReport`] whose
//! [`FixReport::files`] are the pending writes, if accepted) — it never
//! writes to disk. Callers that want to apply the result pass those files to
//! [`writer::write_all_transactionally`]. This keeps the whole fix loop
//! testable without touching the filesystem for anything but reading the
//! original skill's companion files, once, up front.
//!
//! Only [`adept::FixKind::Llm`] diagnostics from single-skill
//! ([`adept::SkillRule`]) checks are ever attempted: today that is `SL301`
//! (`description-tokens-over-budget`), `SL206` (`no-negative-guidance`) —
//! both tagged with [`adept::FixRegion::Description`] and so batched into
//! one description-scoped request — and `SL302` (`body-tokens-over-budget`),
//! tagged [`adept::FixRegion::Body`] and sent as a second, body-scoped
//! request. Which region a rule's diagnostics belong to is metadata on the
//! rule itself (`Rule::fix_kind`); this crate hard-codes no rule-code list.
//! Cross-skill (`SetRule`) findings are never rewritten.

mod candidate;
pub mod diff;
mod options;
mod prompts;
pub mod relocate;
pub mod writer;

pub use candidate::{resolve_companion_path, CompanionEdit, FixCandidate, FixResponse};
pub use options::{FixOptions, DEFAULT_MAX_ROUNDS};
pub use prompts::{
    BODY_FIX_SYSTEM, BODY_FIX_USER_TEMPLATE, DESCRIPTION_FIX_SYSTEM, DESCRIPTION_FIX_USER_TEMPLATE,
};
pub use relocate::{conserves_content, ConservationError, CONTENT_TOLERANCE};
pub use writer::{write_all_transactionally, write_atomically};

use std::collections::BTreeMap;
use std::path::PathBuf;

use adept::{
    discover_companion_files, AdeptError, AnthropicSkillParser, Diagnostic, FixKind, FixRegion,
    Linter, Skill, SkillParser, TokenCounter,
};
use adept_score::{ChatMessage, ChatRequest, LlmClient, LlmError};

/// Errors from attempting to fix a skill: LLM transport failures,
/// malformed LLM-produced JSON, an unsafe companion-file path, a
/// conservation-guard rejection, or an underlying lint/format/I/O failure.
#[derive(Debug, thiserror::Error)]
pub enum FixError {
    /// The LLM client returned an error (network, non-2xx status, timeout).
    #[error("LLM request failed: {0}")]
    Llm(#[from] LlmError),

    /// `adept_fmt` failed to canonicalize a candidate's rewritten SKILL.md.
    #[error("formatting failed: {0}")]
    Fmt(#[from] adept_fmt::FmtError),

    /// The core crate failed to construct a [`Linter`]/[`TokenCounter`], or
    /// a candidate's rewritten SKILL.md failed to re-parse.
    #[error(transparent)]
    Adept(#[from] AdeptError),

    /// A response that should have been the documented JSON shape wasn't.
    #[error("malformed LLM response ({0})")]
    MalformedResponse(String),

    /// A `companion_edits[].path` in a model response was rejected by
    /// [`resolve_companion_path`] as unsafe (absolute, escaping the
    /// skill's directory, or targeting SKILL.md itself).
    #[error("unsafe companion path: {path}")]
    UnsafeCompanionPath {
        /// The rejected, model-supplied path, as given.
        path: String,
    },

    /// An I/O error reading an original companion file while assembling or
    /// checking a candidate.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// The outcome of attempting to fix a skill's LLM-fixable lint diagnostics:
/// the single source of truth for whether a fix landed, replacing what used
/// to be three hand-kept-consistent [`FixReport`] fields (`accepted`,
/// `rejected_reason`, `files`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FixOutcome {
    /// `attempted` was empty: there was nothing to fix.
    NothingToDo,
    /// At least one round produced a candidate that struck diagnostics
    /// strictly smaller than what it started from. `files` are the pending
    /// writes: full new contents for SKILL.md and every touched companion
    /// file, keyed by absolute path. Never written by this crate — pass to
    /// [`writer::write_all_transactionally`] to apply.
    Accepted { files: BTreeMap<PathBuf, String> },
    /// Every round attempted was rejected, either because no candidate
    /// improved on the original or because a candidate tripped the
    /// [`relocate::conserves_content`] guard.
    Rejected { reason: String },
}

/// The result of attempting to fix a skill's LLM-fixable lint diagnostics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FixReport {
    /// The skill's name, at the time fixing started.
    pub skill_name: String,
    /// Every LLM-fixable diagnostic found on the original skill (before any
    /// rounds ran), after `select`/`ignore` filtering. Empty means there
    /// was nothing to do.
    pub attempted: Vec<Diagnostic>,
    /// The subset of `attempted` that the accepted candidate (if any)
    /// cleared.
    pub resolved: Vec<Diagnostic>,
    /// Diagnostics still present after fixing: the final candidate's
    /// remaining LLM-fixable diagnostics if a candidate was accepted, or
    /// all of `attempted` if none was.
    pub residual: Vec<Diagnostic>,
    /// How many rounds were run (0 if `attempted` was empty).
    pub rounds_used: usize,
    /// A unified diff across every changed/created file. Empty when
    /// nothing changed.
    pub diff: String,
    /// Whether a fix was accepted, rejected, or never attempted, and the
    /// data specific to each case.
    pub outcome: FixOutcome,
}

impl FixReport {
    /// Whether a candidate was accepted (equivalent to matching
    /// `FixOutcome::Accepted`).
    #[must_use]
    pub fn accepted(&self) -> bool {
        matches!(self.outcome, FixOutcome::Accepted { .. })
    }

    /// The pending writes if a candidate was accepted, `None` otherwise.
    #[must_use]
    pub fn files(&self) -> Option<&BTreeMap<PathBuf, String>> {
        match &self.outcome {
            FixOutcome::Accepted { files } => Some(files),
            _ => None,
        }
    }

    /// Render a short human-readable summary (per-rule resolved/residual
    /// status) followed by the unified diff, if any.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = format!("adept fix: {}\n", self.skill_name);

        if self.attempted.is_empty() {
            out.push_str("no LLM-fixable diagnostics found\n");
            return out;
        }

        out.push_str(&format!(
            "{} round{} used\n",
            self.rounds_used,
            if self.rounds_used == 1 { "" } else { "s" }
        ));
        for d in &self.resolved {
            out.push_str(&format!("  resolved  {} {}\n", d.code, d.message));
        }
        for d in &self.residual {
            out.push_str(&format!("  residual  {} {}\n", d.code, d.message));
        }
        match &self.outcome {
            FixOutcome::Accepted { .. } => out.push_str("accepted\n"),
            FixOutcome::Rejected { reason } => out.push_str(&format!("rejected: {reason}\n")),
            FixOutcome::NothingToDo => {}
        }

        if !self.diff.is_empty() {
            out.push('\n');
            out.push_str(&self.diff);
        }

        out
    }
}

/// Whether `code`/`name` is selected for fixing under `options`: not
/// excluded by `ignore`, and either `select` is empty or contains it.
fn is_selected(code: &str, name: &str, options: &FixOptions) -> bool {
    let matches = |list: &[String]| list.iter().any(|s| s == code || s == name);
    if matches(&options.ignore) {
        return false;
    }
    options.select.is_empty() || matches(&options.select)
}

/// Filter `diagnostics` down to the ones eligible for LLM-assisted fixing:
/// `FixKind::Llm`, and not excluded by `options.select`/`options.ignore`.
///
/// Which rule codes carry `FixKind::Llm` (and which region they touch) is
/// entirely rule-side metadata (see `adept::Rule::fix_kind`); this crate
/// hard-codes no rule-code list of its own.
fn fixable(diagnostics: &[Diagnostic], linter: &Linter, options: &FixOptions) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .filter(|d| {
            let Some(meta) = linter.registry().by_code(d.code) else {
                return false;
            };
            if !matches!(meta.fix_kind, FixKind::Llm(_)) {
                return false;
            }
            is_selected(meta.code, meta.name, options)
        })
        .cloned()
        .collect()
}

/// The [`FixRegion`] a diagnostic's rule is tagged with, or `None` if it
/// isn't `FixKind::Llm` (shouldn't happen for anything that survived
/// [`fixable`], but keeps this total rather than panicking).
fn region_of(code: &str, linter: &Linter) -> Option<FixRegion> {
    match linter.registry().by_code(code)?.fix_kind {
        FixKind::Llm(region) => Some(region),
        _ => None,
    }
}

/// Render a bullet-list "violations" block for a group of diagnostics,
/// including the concrete numeric budget they were checked against.
fn render_violations(diagnostics: &[&Diagnostic], budget_line: &str) -> String {
    let mut out = String::new();
    out.push_str(budget_line);
    out.push('\n');
    for d in diagnostics {
        out.push_str(&format!("- {}: {}", d.code, d.message));
        if let Some(hint) = &d.fix_suggestion {
            out.push_str(&format!(" — hint: {hint}"));
        }
        out.push('\n');
    }
    out
}

/// Send one region-scoped fix request — description-scope batches
/// `SL301`/`SL206` diagnostics, body-scope sends `SL302` — and return the
/// model's parsed response. The two request shapes differ only in system
/// prompt, user template, and the budget wording, so both share this one
/// implementation, parameterized by [`FixRegion`].
async fn request_region_fix(
    client: &dyn LlmClient,
    skill: &Skill,
    region: FixRegion,
    diagnostics: &[&Diagnostic],
    options: &FixOptions,
) -> Result<FixResponse, FixError> {
    let (system, template, budget_line) = match region {
        FixRegion::Description => (
            prompts::DESCRIPTION_FIX_SYSTEM,
            prompts::DESCRIPTION_FIX_USER_TEMPLATE,
            format!(
                "The description MUST be at most {} {} tokens.",
                options.lint_config.description_max_tokens, options.tokenizer
            ),
        ),
        FixRegion::Body => (
            prompts::BODY_FIX_SYSTEM,
            prompts::BODY_FIX_USER_TEMPLATE,
            format!(
                "The body MUST be at most {} {} tokens.",
                options.lint_config.body_max_tokens, options.tokenizer
            ),
        ),
    };

    let user = adept_score::prompts::render(
        template,
        &[
            ("skill_name", skill.frontmatter.name.as_str()),
            ("description", skill.frontmatter.description.as_str()),
            ("body", skill.body.as_str()),
            ("violations", &render_violations(diagnostics, &budget_line)),
        ],
    );
    let request = ChatRequest::new(
        options.model.clone(),
        vec![ChatMessage::system(system), ChatMessage::user(user)],
    )
    .with_temperature(0.0)
    .with_json_response(true);

    let response = client.chat(request).await?;
    FixResponse::parse(&response.content)
}

/// Read `path`'s contents, treating "file not found" as an empty string —
/// the state of a companion file a fix round is about to create for the
/// first time. Other I/O errors are propagated.
fn read_or_empty(path: &std::path::Path) -> Result<String, FixError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(FixError::Io(err)),
    }
}

/// Build a candidate SKILL.md source from `working` with `new_description`
/// and `new_body` substituted in, canonicalized by `adept_fmt`.
fn build_candidate_source(
    working: &Skill,
    new_description: String,
    new_body: String,
    fmt_config: &adept_fmt::FmtConfig,
) -> Result<String, FixError> {
    let mut candidate_frontmatter = working.frontmatter.clone();
    candidate_frontmatter.description = new_description;
    let candidate_skill_for_fmt = Skill {
        path: working.path.clone(),
        frontmatter: candidate_frontmatter,
        body: new_body,
        body_line_offset: working.body_line_offset,
        source: working.source.clone(),
    };
    Ok(adept_fmt::format_skill(
        &candidate_skill_for_fmt,
        fmt_config,
    )?)
}

/// Assemble full new contents for every companion file a round's response
/// touched: resolve and sandbox each edit's path, then append it to that
/// file's current content.
///
/// "Current content" prefers an in-progress (already-pending) version from
/// an earlier round in `best_files` over `original_companions`, so a second
/// round editing the same companion appends to what round one produced
/// instead of clobbering it. `original_companions` was read once, up front,
/// by the caller — this never touches disk.
fn assemble_companions(
    companion_edits: &[CompanionEdit],
    skill_dir: &std::path::Path,
    skill_path: &std::path::Path,
    best_files: &BTreeMap<PathBuf, String>,
    original_companions: &BTreeMap<PathBuf, String>,
) -> Result<BTreeMap<PathBuf, String>, FixError> {
    let mut companions: BTreeMap<PathBuf, String> = BTreeMap::new();
    for edit in companion_edits {
        let resolved = candidate::resolve_companion_path(skill_dir, &edit.path, skill_path)?;
        let existing = best_files
            .get(&resolved)
            .or_else(|| original_companions.get(&resolved))
            .cloned()
            .unwrap_or_default();
        let merged = format!("{existing}{}", edit.appended_content);
        companions.insert(resolved, merged);
    }
    Ok(companions)
}

/// Attempt to fix `skill`'s LLM-fixable lint diagnostics.
///
/// Only `skill`'s own single-skill (`SkillRule`) diagnostics are ever
/// attempted; cross-skill (`SetRule`) findings are reported by `adept check`
/// but never auto-rewritten here (see the module docs).
///
/// See the module docs for the overall loop. This function never writes to
/// disk (beyond reading pre-existing companion files, once, up front, to
/// assemble full new contents and to run the conservation guard); pass
/// [`FixReport::files`] to [`writer::write_all_transactionally`] to apply an
/// accepted result.
///
/// # Errors
/// Returns [`FixError`] if an LLM call fails, a response is malformed, a
/// companion path is unsafe, the conservation guard rejects a body
/// relocation, or an underlying lint/format/I/O operation fails.
pub async fn fix_skill(
    client: &dyn LlmClient,
    skill: &Skill,
    options: &FixOptions,
) -> Result<FixReport, FixError> {
    let linter = Linter::new(options.lint_config.clone())?;
    let tokens = TokenCounter::new(options.tokenizer)?;

    let initial = linter.lint_skill(skill);
    let attempted = fixable(&initial, &linter, options);

    if attempted.is_empty() {
        return Ok(FixReport {
            skill_name: skill.frontmatter.name.clone(),
            attempted,
            resolved: Vec::new(),
            residual: Vec::new(),
            rounds_used: 0,
            diff: String::new(),
            outcome: FixOutcome::NothingToDo,
        });
    }

    let skill_dir = skill
        .path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // Read every pre-existing companion file exactly once, up front. Every
    // round below (the guard, companion assembly, and the final diff) reads
    // from this map instead of the filesystem, since the original content
    // never changes across rounds.
    let mut original_companions: BTreeMap<PathBuf, String> = BTreeMap::new();
    for path in discover_companion_files(skill) {
        original_companions.insert(path.clone(), read_or_empty(&path)?);
    }

    let mut working = skill.clone();
    let mut current: Vec<Diagnostic> = attempted.clone();
    let mut best_files: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut accepted = false;
    let mut rounds_used = 0;
    let mut rejected_reason: Option<String> = None;

    for round in 1..=options.max_rounds {
        if current.is_empty() {
            break;
        }
        rounds_used = round;

        let description_group: Vec<&Diagnostic> = current
            .iter()
            .filter(|d| region_of(d.code, &linter) == Some(FixRegion::Description))
            .collect();
        let body_group: Vec<&Diagnostic> = current
            .iter()
            .filter(|d| region_of(d.code, &linter) == Some(FixRegion::Body))
            .collect();

        let mut new_description = working.frontmatter.description.clone();
        let mut new_body = working.body.clone();
        let mut companion_edits: Vec<CompanionEdit> = Vec::new();

        if !description_group.is_empty() {
            let response = request_region_fix(
                client,
                &working,
                FixRegion::Description,
                &description_group,
                options,
            )
            .await?;
            if let Some(description) = response.description {
                new_description = description;
            }
        }

        if !body_group.is_empty() {
            let response =
                request_region_fix(client, &working, FixRegion::Body, &body_group, options).await?;
            if let Some(body) = response.body {
                new_body = body;
            }
            if let Some(edits) = response.companion_edits {
                companion_edits = edits;
            }
        }

        let formatted =
            build_candidate_source(&working, new_description, new_body, &options.fmt_config)?;

        let companions = assemble_companions(
            &companion_edits,
            &skill_dir,
            &skill.path,
            &best_files,
            &original_companions,
        )?;

        let fix_candidate = FixCandidate {
            skill_source: formatted.clone(),
            companions,
        };

        if !body_group.is_empty() {
            if let Err(err) =
                relocate::conserves_content(skill, &fix_candidate, &tokens, &original_companions)
            {
                rejected_reason = Some(format!(
                    "candidate lost content instead of relocating it ({err})"
                ));
                break;
            }
        }

        let candidate_skill = AnthropicSkillParser.parse_str(&skill.path, &formatted)?;
        let candidate_diagnostics = linter.lint_skill(&candidate_skill);
        let candidate_fixable = fixable(&candidate_diagnostics, &linter, options);

        if candidate_fixable.len() < current.len() {
            accepted = true;
            working = candidate_skill;
            best_files.insert(skill.path.clone(), formatted);
            for (path, contents) in fix_candidate.companions {
                best_files.insert(path, contents);
            }
            current = candidate_fixable;
        } else {
            rejected_reason = Some("no candidate improved on the original".to_string());
            break;
        }
    }

    let (resolved, residual) = if accepted {
        let resolved: Vec<Diagnostic> = attempted
            .iter()
            .filter(|d| !current.iter().any(|c| c.code == d.code))
            .cloned()
            .collect();
        (resolved, current.clone())
    } else {
        (Vec::new(), attempted.clone())
    };

    let diff = if accepted {
        let mut originals: BTreeMap<PathBuf, String> = BTreeMap::new();
        originals.insert(skill.path.clone(), skill.source.clone());
        for (path, contents) in &original_companions {
            originals.insert(path.clone(), contents.clone());
        }
        diff::render_multi_file_diff(&originals, &best_files)
    } else {
        String::new()
    };

    let outcome = if accepted {
        FixOutcome::Accepted { files: best_files }
    } else {
        FixOutcome::Rejected {
            reason: rejected_reason
                .unwrap_or_else(|| "no candidate improved on the original".to_string()),
        }
    };

    Ok(FixReport {
        skill_name: skill.frontmatter.name.clone(),
        attempted,
        resolved,
        residual,
        rounds_used,
        diff,
        outcome,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use adept::Tokenizer;
    use adept_score::MockLlmClient;
    use std::io::Write;

    fn write_skill(dir: &std::path::Path, description: &str, body: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("SKILL.md");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            "---\nname: demo\ndescription: {description}\n---\n{body}"
        )
        .unwrap();
        path
    }

    fn base_options() -> FixOptions {
        FixOptions::for_model("test-model", Tokenizer::O200kBase)
    }

    #[tokio::test]
    async fn description_rewrite_batches_sl301_and_sl206() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // A description over budget (many repeated words) and with no
        // negative guidance triggers both SL301 and SL206.
        let long_description = "extracts data from PDF files ".repeat(15);
        let path = write_skill(dir, long_description.trim(), "# Demo\n\nBody text.\n");
        let skill = adept::parse_skill(&path).unwrap();

        let short_description =
            "Extracts data from PDF forms. Do not use for scanned image-only PDFs.";
        let mock =
            MockLlmClient::with_texts(vec![format!(r#"{{"description": "{short_description}"}}"#)]);

        let options = base_options();
        let report = fix_skill(&mock, &skill, &options).await.unwrap();

        assert_eq!(mock.call_count(), 1);
        let request = &mock.calls()[0];
        let user_content = &request.messages[1].content;
        assert!(user_content.contains("SL301"));
        assert!(user_content.contains("SL206"));

        assert!(report.accepted());
        assert!(report.residual.is_empty());
        let files = report
            .files()
            .expect("accepted candidate has pending files");
        assert!(files.contains_key(&path));
        assert!(files[&path].contains(short_description));
        // Regression test for B5: a fix fully resolved in round 1 must
        // report 1 round used, not 2.
        assert_eq!(report.rounds_used, 1);
    }

    #[tokio::test]
    async fn body_relocation_moves_content_to_companion() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let long_body = format!("# Demo\n\n{}", "word ".repeat(2000));
        let path = write_skill(
            dir,
            "Does a thing. Do not use for other things.",
            &long_body,
        );
        let skill = adept::parse_skill(&path).unwrap();

        let short_body = "# Demo\n\nSee REFERENCE.md for details.\n";
        let relocated = "word ".repeat(2000);
        let response = serde_json::json!({
            "body": short_body,
            "companion_edits": [
                {"path": "REFERENCE.md", "appended_content": relocated}
            ]
        })
        .to_string();
        let mock = MockLlmClient::with_texts(vec![response]);

        let options = base_options();
        let report = fix_skill(&mock, &skill, &options).await.unwrap();

        assert!(report.accepted(), "{report:?}");
        assert!(report.residual.is_empty(), "{:?}", report.residual);
        let files = report
            .files()
            .expect("accepted candidate has pending files");
        assert!(files.contains_key(&path));
        let reference_path = dir.join("REFERENCE.md");
        assert!(files.contains_key(&reference_path));
        assert!(files[&reference_path].contains("word word"));
    }

    /// Regression test for B1: a model response that truncates the body to
    /// clear `SL302` but proposes zero `companion_edits` must be rejected by
    /// the conservation guard, not silently accepted just because there are
    /// no companions to check against.
    #[tokio::test]
    async fn body_truncation_with_no_companion_edits_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let long_body = format!("# Demo\n\n{}", "word ".repeat(2000));
        let path = write_skill(
            dir,
            "Does a thing. Do not use for other things.",
            &long_body,
        );
        let skill = adept::parse_skill(&path).unwrap();

        // Well under budget, so the candidate would otherwise be accepted —
        // but it deletes almost all the content instead of relocating it,
        // and proposes no companion_edits at all.
        let short_body = "# Demo\n\nShort.\n";
        let response = serde_json::json!({ "body": short_body }).to_string();
        let mock = MockLlmClient::with_texts(vec![response]);

        let options = base_options();
        let report = fix_skill(&mock, &skill, &options).await.unwrap();

        assert!(!report.accepted());
        assert!(report.files().is_none());
        assert!(!report.residual.is_empty());
        let reason = match &report.outcome {
            FixOutcome::Rejected { reason } => reason.as_str(),
            other => panic!("expected a rejection, got {other:?}"),
        };
        assert!(
            reason.contains("lost content") || reason.contains("relocat"),
            "{reason}"
        );
    }

    /// Regression test for B3: a second round that edits the same companion
    /// file an earlier round already relocated content into must build on
    /// that in-progress content, not re-read the (still nonexistent, since
    /// nothing has been written to disk yet) file and clobber round one's
    /// work.
    #[tokio::test]
    async fn two_round_companion_edit_carries_forward_prior_round_content() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // No negative guidance (SL206) and a heavily over-budget body
        // (SL302), so both a description-scope and a body-scope request
        // fire in round 1.
        let long_body = format!("# Demo\n\n{}", "word ".repeat(3000));
        let path = write_skill(
            dir,
            "Does a thing with many words for testing purposes here",
            &long_body,
        );
        let skill = adept::parse_skill(&path).unwrap();

        // Round 1: resolves SL206, but only partially relocates the body —
        // still over budget, so SL302 survives into round 2.
        let round1_description =
            r#"{"description": "Does a thing. Do not use for other things."}"#.to_string();
        let round1_body = serde_json::json!({
            "body": format!("# Demo\n\n{}", "word ".repeat(1600)),
            "companion_edits": [
                {"path": "REFERENCE.md", "appended_content": "roundone ".repeat(1400)}
            ]
        })
        .to_string();
        // Round 2: fully resolves SL302 by relocating the rest, appending
        // to the same companion file.
        let round2_body = serde_json::json!({
            "body": "# Demo\n\nSee REFERENCE.md for details.\n",
            "companion_edits": [
                {"path": "REFERENCE.md", "appended_content": "roundtwo ".repeat(1600)}
            ]
        })
        .to_string();

        let mock = MockLlmClient::with_texts(vec![round1_description, round1_body, round2_body]);

        let options = base_options();
        let report = fix_skill(&mock, &skill, &options).await.unwrap();

        assert!(report.accepted(), "{report:?}");
        assert!(report.residual.is_empty(), "{:?}", report.residual);
        assert_eq!(report.rounds_used, 2);

        let reference_path = dir.join("REFERENCE.md");
        let contents = report
            .files()
            .expect("accepted candidate has pending files")
            .get(&reference_path)
            .expect("REFERENCE.md pending");
        assert!(
            contents.contains("roundone"),
            "round 2 clobbered round 1's companion content: {contents}"
        );
        assert!(contents.contains("roundtwo"));
    }

    #[tokio::test]
    async fn model_makes_it_worse_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let long_body = format!("# Demo\n\n{}", "word ".repeat(2000));
        let path = write_skill(
            dir,
            "Does a thing. Do not use for other things.",
            &long_body,
        );
        let skill = adept::parse_skill(&path).unwrap();

        // Still-over-budget rewrite, every round.
        let still_long_body = format!("# Demo\n\n{}", "word ".repeat(1900));
        let response = serde_json::json!({ "body": still_long_body }).to_string();
        let mock = MockLlmClient::with_texts(vec![response.clone(), response]);

        let options = base_options();
        let report = fix_skill(&mock, &skill, &options).await.unwrap();

        assert!(!report.accepted());
        assert!(report.files().is_none());
        assert!(!report.residual.is_empty());
    }

    #[tokio::test]
    async fn select_and_ignore_restrict_attempted_set() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let long_description = "extracts data from PDF files ".repeat(15);
        let path = write_skill(dir, long_description.trim(), "# Demo\n\nBody text.\n");
        let skill = adept::parse_skill(&path).unwrap();

        let mut options = base_options();
        options.select = vec!["SL206".to_string()];
        let mock = MockLlmClient::with_texts(vec![
            r#"{"description": "Extracts data. Do not use for anything else."}"#,
        ]);
        let report = fix_skill(&mock, &skill, &options).await.unwrap();
        assert_eq!(report.attempted.len(), 1);
        assert_eq!(report.attempted[0].code, "SL206");

        let mut options = base_options();
        options.ignore = vec!["SL206".to_string()];
        let mock = MockLlmClient::with_texts(vec![
            r#"{"description": "Extracts data from PDF forms reliably every time now."}"#,
        ]);
        let report = fix_skill(&mock, &skill, &options).await.unwrap();
        assert!(report.attempted.iter().all(|d| d.code != "SL206"));
        assert!(report.attempted.iter().any(|d| d.code == "SL301"));
    }

    #[tokio::test]
    async fn no_fixable_diagnostics_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let path = write_skill(
            dir,
            "Extracts data from PDF forms. Do not use for scanned images.",
            "# Demo\n\nShort body.\n",
        );
        let skill = adept::parse_skill(&path).unwrap();
        let mock = MockLlmClient::with_texts(Vec::<String>::new());

        let options = base_options();
        let report = fix_skill(&mock, &skill, &options).await.unwrap();

        assert!(report.attempted.is_empty());
        assert!(!report.accepted());
        assert!(report.files().is_none());
        assert_eq!(mock.call_count(), 0);
    }
}
