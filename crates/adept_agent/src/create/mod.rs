//! LLM-assisted skill generation (`adept create`) for Agent Skills.
//!
//! Mirrors [`crate::fix`]'s shape: everything that talks to a model goes
//! through a `&dyn crate::LlmClient`, and the single public entry
//! point, [`create_skill`], only *computes* a candidate — it never writes to
//! disk. Callers that want to apply the result pass [`CreateReport::files`]
//! to [`crate::writer::write_all_transactionally`].
//!
//! Unlike `fix`, `create` has no "before": there is no existing skill to
//! compare a candidate against, only a brief. The loop is generate → screen
//! → repair:
//!
//! 1. **Generate** one candidate skill from the brief (one LLM call).
//! 2. **Screen** it in memory: insert it into a [`adept::SkillSet`] alongside
//!    the siblings discovered at [`adept::sibling_root`], canonicalize with
//!    `adept_fmt`, and lint the whole set — so both [`adept::SkillRule`] and
//!    [`adept::SetRule`] findings are visible.
//! 3. **Repair**, for up to `max_rounds`: the gate is zero `Error`/`Warning`
//!    diagnostics on the candidate itself, *and* no diagnostic newly
//!    appearing on a sibling relative to a pre-generation baseline lint of
//!    the siblings alone (see [`crate::gate::passes_severity_gate`] and
//!    [`crate::gate::improves_on`], the same comparison machinery `fix`
//!    uses). A sibling's own pre-existing findings never block emission and
//!    are never rewritten — only the candidate is. If no round reaches the
//!    gate, the best-scoring candidate seen is carried forward rather than
//!    discarded; [`CreateOutcome`] tells the caller which happened.
//!
//! A second, independent LLM call then generates a synthetic eval dataset
//! (`evals/evals.jsonl`) for the accepted candidate, given both the
//! candidate and the original brief — validated against
//! `adept::evals::validate` before being returned; a dataset that fails
//! validation fails the whole run.

pub mod candidate;
mod options;

pub use options::{CreateOptions, DEFAULT_EVAL_CASES, DEFAULT_MAX_ROUNDS};

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use crate::{ChatMessage, ChatRequest, LlmClient, LlmError};
use adept::{
    evals, AdeptError, AnthropicSkillParser, Diagnostic, ExtraField, Frontmatter, Linter, Skill,
    SkillParser, SkillSet,
};

use crate::gate;
use crate::prompts;
use candidate::{EvalGenerationResponse, GenerateResponse};

/// Errors from attempting to create a skill: LLM transport failures,
/// malformed LLM-produced JSON, an unsafe companion-file path, a generated
/// eval dataset that fails schema validation, or an underlying
/// lint/format/I/O failure.
#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    /// The LLM client returned an error (network, non-2xx status, timeout).
    #[error("LLM request failed: {0}")]
    Llm(#[from] LlmError),

    /// `adept_fmt` failed to canonicalize the candidate's SKILL.md.
    #[error("formatting failed: {0}")]
    Fmt(#[from] adept_fmt::FmtError),

    /// The core crate failed to construct a [`Linter`], or the candidate's
    /// SKILL.md source failed to re-parse.
    #[error(transparent)]
    Adept(#[from] AdeptError),

    /// A response that should have been the documented JSON shape wasn't.
    #[error("malformed LLM response ({0})")]
    MalformedResponse(String),

    /// A `companion_files[].path` in a model response was rejected by
    /// [`crate::resolve_companion_path`] as unsafe.
    #[error("unsafe companion path: {path}")]
    UnsafeCompanionPath {
        /// The rejected, model-supplied path, as given.
        path: String,
    },

    /// The generated eval dataset failed `adept::evals::validate`. adept
    /// does not write cases it cannot vouch for the shape of.
    #[error("generated eval dataset failed validation: {0}")]
    InvalidEvalDataset(#[from] evals::EvalError),
}

impl From<crate::candidate::UnsafeCompanionPath> for CreateError {
    fn from(err: crate::candidate::UnsafeCompanionPath) -> Self {
        CreateError::UnsafeCompanionPath { path: err.path }
    }
}

/// The outcome of a `create` run's repair loop: whether the emitted
/// candidate reached a clean lint, or is the best effort carried forward
/// after `max_rounds` was exhausted.
///
/// This is the single source of truth the CLI (a later phase) maps to exit
/// codes: `Clean` -> `0`, `BestEffort` -> `1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CreateOutcome {
    /// The candidate reached the gate: zero `Error`/`Warning` diagnostics on
    /// it, and no diagnostic newly appeared on a sibling.
    Clean,
    /// `max_rounds` was exhausted without reaching the gate. The candidate
    /// emitted is the best-scoring one seen across all rounds, not
    /// discarded.
    BestEffort,
}

/// The result of a `create` run: the accepted candidate's pending files, the
/// diagnostics remaining on it, and the generated eval dataset.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CreateReport {
    /// The candidate's frontmatter `name`, at the time of emission.
    pub skill_name: String,
    /// How many authoring rounds were run (at least 1).
    pub rounds_used: usize,
    /// Whether any sibling skills were discovered at `adept::sibling_root`
    /// before generation started. `false` means the cross-skill screen was
    /// vacuous (nothing to compare against), which the caller should
    /// distinguish from a screen that ran and found nothing.
    pub siblings_found: bool,
    /// Diagnostics remaining on the candidate itself, from the final round.
    pub candidate_diagnostics: Vec<Diagnostic>,
    /// Diagnostics that newly appeared on a sibling (relative to the
    /// pre-generation baseline) in the final round, and were not resolved by
    /// the end of the loop. Always empty when `outcome` is `Clean`.
    pub new_sibling_diagnostics: Vec<Diagnostic>,
    /// The generated eval dataset, already validated.
    pub eval_cases: Vec<evals::EvalCase>,
    /// How many generated cases were dropped because their `prompt` +
    /// `assertions` duplicated an earlier case's (ids are content-addressed,
    /// so two identical cases would otherwise mint the same id and fail
    /// `adept::evals::validate`; see [`dedup_eval_cases`]).
    pub duplicate_eval_cases_dropped: usize,
    /// Pending writes: the candidate's `SKILL.md`, every companion file, and
    /// `evals/evals.jsonl`, keyed by absolute path. Never written by this
    /// crate — pass to [`crate::writer::write_all_transactionally`] to
    /// apply.
    pub files: BTreeMap<PathBuf, String>,
    /// Whether the candidate reached a clean lint or is a best-effort
    /// carry-forward.
    pub outcome: CreateOutcome,
}

impl CreateReport {
    /// Whether [`Self::outcome`] is [`CreateOutcome::Clean`].
    #[must_use]
    pub fn is_clean(&self) -> bool {
        matches!(self.outcome, CreateOutcome::Clean)
    }
}

/// A diagnostic's identity for baseline comparison: same path, same code,
/// same message. Used to tell a *newly appeared* sibling finding (not in the
/// pre-generation baseline) from one that already existed before `create`
/// ran.
fn diag_key(d: &Diagnostic) -> (PathBuf, &'static str, String) {
    (d.path.clone(), d.code, d.message.clone())
}

/// Overwrite `response.name` with `name_override`, if set, before the
/// response is screened. Applied at candidate-construction time (every
/// round, not just once at the end) so a caller-supplied name is what the
/// gate lints and the repair loop repairs against, rather than a post-hoc
/// patch on an already-accepted candidate that could silently reintroduce a
/// directory-name mismatch (`SL004`) or a sibling collision
/// (`SL401`/`SL402`).
fn apply_name_override(response: &mut GenerateResponse, name_override: Option<&str>) {
    if let Some(name) = name_override {
        response.name = name.to_string();
    }
}

/// Build a candidate [`Skill`] and its companion files from a
/// [`GenerateResponse`], canonicalizing the SKILL.md source via `adept_fmt`
/// and re-parsing it so line numbers are correct.
fn build_candidate(
    out_dir: &Path,
    response: &GenerateResponse,
    fmt_config: &adept_fmt::FmtConfig,
) -> Result<(Skill, BTreeMap<PathBuf, String>), CreateError> {
    let skill_md_path = out_dir.join("SKILL.md");

    let mut extra = BTreeMap::new();
    extra.insert(
        "disable-model-invocation".to_string(),
        ExtraField {
            value: serde_yaml::Value::Bool(response.disable_model_invocation),
            line: 0,
        },
    );
    let frontmatter = Frontmatter {
        name: response.name.clone(),
        name_line: 2,
        description: response.description.clone(),
        description_line: 3,
        license: None,
        license_line: None,
        extra,
    };
    let unformatted = Skill {
        path: skill_md_path.clone(),
        frontmatter,
        body: response.body.clone(),
        body_line_offset: 0,
        source: String::new(),
    };
    let formatted = adept_fmt::format_skill(&unformatted, fmt_config)?;
    let skill = AnthropicSkillParser.parse_str(&skill_md_path, &formatted)?;

    let mut companions = BTreeMap::new();
    for file in &response.companion_files {
        let resolved =
            crate::candidate::resolve_companion_path(out_dir, &file.path, &skill_md_path)?;
        companions.insert(resolved, file.content.clone());
    }

    Ok((skill, companions))
}

/// Render a bullet-list of sibling skills' name/description, or a line
/// stating none were found.
fn render_siblings(siblings: &[Skill]) -> String {
    if siblings.is_empty() {
        return "(no sibling skills found)".to_string();
    }
    let mut out = String::new();
    for s in siblings {
        out.push_str(&format!(
            "- {}: {}\n",
            s.frontmatter.name, s.frontmatter.description
        ));
    }
    out
}

/// Render a bullet-list of diagnostics for a repair request.
fn render_diagnostics(diagnostics: &[Diagnostic]) -> String {
    prompts::render_diagnostic_bullets(diagnostics, true)
}

/// Send the initial authoring request.
async fn request_generate(
    client: &dyn LlmClient,
    brief: &str,
    siblings: &[Skill],
    model: &str,
) -> Result<GenerateResponse, CreateError> {
    let user = crate::eval::prompts::render(
        prompts::CREATE_AUTHORING_USER_TEMPLATE,
        &[("brief", brief), ("siblings", &render_siblings(siblings))],
    );
    let request = ChatRequest::new(
        model.to_string(),
        vec![
            ChatMessage::system(prompts::CREATE_AUTHORING_SYSTEM),
            ChatMessage::user(user),
        ],
    )
    .with_temperature(0.0)
    .with_json_response(true);

    let response = client.chat(request).await?;
    GenerateResponse::parse(&response.content)
}

/// Send a repair request: the model's own previous candidate plus the
/// diagnostics it needs to resolve.
async fn request_repair(
    client: &dyn LlmClient,
    brief: &str,
    previous: &GenerateResponse,
    diagnostics: &[Diagnostic],
    model: &str,
) -> Result<GenerateResponse, CreateError> {
    let user = crate::eval::prompts::render(
        prompts::CREATE_REPAIR_USER_TEMPLATE,
        &[
            ("brief", brief),
            ("name", previous.name.as_str()),
            ("description", previous.description.as_str()),
            ("body", previous.body.as_str()),
            ("diagnostics", &render_diagnostics(diagnostics)),
        ],
    );
    let request = ChatRequest::new(
        model.to_string(),
        vec![
            ChatMessage::system(prompts::CREATE_AUTHORING_SYSTEM),
            ChatMessage::user(user),
        ],
    )
    .with_temperature(0.0)
    .with_json_response(true);

    let response = client.chat(request).await?;
    GenerateResponse::parse(&response.content)
}

/// Generate and validate a synthetic eval dataset for `skill`, given the
/// original task `brief` (the only record of intent the skill's own
/// name/description/body may have omitted).
///
/// This is the single implementation of the eval-generation step: both
/// [`create_skill`] (after its own repair loop accepts a candidate) and
/// `adept_cli`'s `generate_evals` MCP tool (given an already-authored skill)
/// call this rather than each re-implementing the request/parse/stamp/
/// validate sequence, so the two surfaces cannot drift against
/// `docs/EVALS.md`'s published contract.
///
/// `options.model` selects the model and `options.eval_cases` the number of
/// cases requested; the rest of `options` is unused here.
///
/// # Errors
/// Returns [`CreateError::Llm`] if the request fails, [`CreateError::MalformedResponse`]
/// if the model's response isn't the documented JSON shape, or
/// [`CreateError::InvalidEvalDataset`] if the generated dataset fails
/// `adept::evals::validate`.
pub async fn generate_evals(
    client: &dyn LlmClient,
    skill: &Skill,
    brief: &str,
    options: &CreateOptions,
) -> Result<Vec<evals::EvalCase>, CreateError> {
    let (cases, _dropped) = generate_evals_with_drop_count(client, skill, brief, options).await?;
    Ok(cases)
}

/// Drop later cases whose `prompt` + `assertions` duplicate an earlier
/// case's, preserving order and keeping the first occurrence. Two cases
/// identical in both fields are the same case, not two tests — and since ids
/// are now content-addressed (see [`candidate::slugify_prompt`]), letting
/// both through would mint the same id twice and fail
/// `adept::evals::validate`'s uniqueness check. Returns the survivors plus
/// how many were dropped.
fn dedup_eval_cases(cases: Vec<evals::EvalCase>) -> (Vec<evals::EvalCase>, usize) {
    // Deliberately compares `prompt` + `assertions` field-wise rather than
    // keying on the (already-computed, content-addressed) `id`. Id equality is
    // *digest* equality, and the digest is truncated to `DIGEST_HEX_LEN` hex
    // chars — so id-keyed dedup would silently discard a genuinely distinct
    // case that happened to collide. Comparing the content itself keeps
    // collisions loud: two differing cases both survive to `validate_cases`,
    // which rejects the dataset on the duplicate id.
    //
    // `evals::Assertion` derives `PartialEq` but not `Hash`/`Eq`, and dataset
    // sizes here are small (bounded by `options.eval_cases`, tens of cases at
    // most), so a linear membership check against what's been kept so far is
    // simpler than introducing a hashable proxy key.
    let total = cases.len();
    let mut kept: Vec<evals::EvalCase> = Vec::with_capacity(total);
    for case in cases {
        let is_duplicate = kept
            .iter()
            .any(|k| k.prompt == case.prompt && k.assertions == case.assertions);
        if !is_duplicate {
            kept.push(case);
        }
    }
    let dropped = total - kept.len();
    (kept, dropped)
}

/// The implementation behind [`generate_evals`], additionally reporting how
/// many generated cases were dropped as content-duplicates (see
/// [`dedup_eval_cases`]) so [`create_skill`] can surface the count on
/// [`CreateReport::duplicate_eval_cases_dropped`]. [`generate_evals`] itself
/// discards the count so its public signature stays unchanged for other
/// callers (e.g. the `generate_evals` MCP tool) that have no report to put it
/// on.
async fn generate_evals_with_drop_count(
    client: &dyn LlmClient,
    skill: &Skill,
    brief: &str,
    options: &CreateOptions,
) -> Result<(Vec<evals::EvalCase>, usize), CreateError> {
    let user = crate::eval::prompts::render(
        prompts::CREATE_EVAL_USER_TEMPLATE,
        &[
            ("brief", brief),
            ("skill_name", skill.frontmatter.name.as_str()),
            ("description", skill.frontmatter.description.as_str()),
            ("body", skill.body.as_str()),
            ("n", &options.eval_cases.to_string()),
        ],
    );
    let request = ChatRequest::new(
        options.model.clone(),
        vec![
            ChatMessage::system(prompts::CREATE_EVAL_SYSTEM),
            ChatMessage::user(user),
        ],
    )
    .with_temperature(0.0)
    .with_json_response(true);

    let response = client.chat(request).await?;
    let parsed = EvalGenerationResponse::parse(&response.content)?;
    let cases: Vec<evals::EvalCase> = parsed
        .cases
        .into_iter()
        .map(|c| evals::EvalCase {
            schema_version: evals::SCHEMA_VERSION,
            id: candidate::slugify_prompt(&c.prompt, &c.assertions),
            prompt: c.prompt,
            assertions: c.assertions,
        })
        .collect();

    let (cases, dropped) = dedup_eval_cases(cases);

    evals::validate_cases(&cases)?;

    Ok((cases, dropped))
}

/// One round's screening result: the candidate itself, its own diagnostics,
/// and any sibling diagnostics newly appeared relative to the baseline.
struct RoundResult {
    response: GenerateResponse,
    skill: Skill,
    companions: BTreeMap<PathBuf, String>,
    candidate_diagnostics: Vec<Diagnostic>,
    new_sibling_diagnostics: Vec<Diagnostic>,
}

impl RoundResult {
    /// All diagnostics this round is judged on: the candidate's own plus any
    /// newly-appeared sibling ones. Used by [`gate::passes_severity_gate`]
    /// for candidate-only acceptance, and (via [`Self::combined_counts`]) by
    /// [`gate::improves_on_counts`] to compare rounds.
    fn combined(&self) -> Vec<Diagnostic> {
        self.candidate_diagnostics
            .iter()
            .cloned()
            .chain(self.new_sibling_diagnostics.iter().cloned())
            .collect()
    }

    /// The per-severity tally [`Self::combined`] would produce, without
    /// building it — the round-over-round acceptance comparison only ever
    /// needs the counts.
    fn combined_counts(&self) -> gate::Counts {
        gate::Counts::of_severities(
            self.candidate_diagnostics
                .iter()
                .chain(self.new_sibling_diagnostics.iter())
                .map(|d| d.severity),
        )
    }

    fn gate_passes(&self) -> bool {
        gate::passes_severity_gate(&self.candidate_diagnostics)
            && self.new_sibling_diagnostics.is_empty()
    }
}

/// Screen `response` in memory: build the candidate, insert it into
/// `combined_skills` (which already holds the siblings, plus — from the
/// second round on — the previous round's candidate as its last element) in
/// place, and lint the whole set.
///
/// `combined_skills` is owned by the caller across rounds so the sibling
/// portion is built once, not re-cloned every round: this function only ever
/// pushes (round 1) or overwrites in place (later rounds) the one candidate
/// slot, and temporarily moves the vector into the [`SkillSet`] it lints
/// (moving it back out afterwards) rather than cloning it.
///
/// Note: this omits `siblings`' own parse errors from the combined set,
/// since the cross-skill (`SetRule`) rules only ever look at `set.skills`
/// and a sibling's parse-failure diagnostics (`SL001`-`SL003`) are already
/// fully captured once, up front, in the caller's baseline lint —
/// `adept::AdeptError` isn't `Clone`, so re-attaching them per round would
/// require cloning them, and doing so would add nothing to the gate
/// comparison that the baseline doesn't already cover.
fn screen_round(
    out_dir: &Path,
    response: GenerateResponse,
    combined_skills: &mut Vec<Skill>,
    siblings_len: usize,
    baseline_keys: &HashSet<(PathBuf, &'static str, String)>,
    linter: &Linter,
    fmt_config: &adept_fmt::FmtConfig,
) -> Result<RoundResult, CreateError> {
    let (skill, companions) = build_candidate(out_dir, &response, fmt_config)?;

    if combined_skills.len() > siblings_len {
        *combined_skills.last_mut().expect("checked above") = skill.clone();
    } else {
        combined_skills.push(skill.clone());
    }

    let skills = std::mem::take(combined_skills);
    let combined_set = SkillSet {
        skills,
        errors: Vec::new(),
    };
    let all_diagnostics = linter.lint_set(&combined_set);
    *combined_skills = combined_set.skills;

    let (candidate_diagnostics, sibling_diagnostics): (Vec<_>, Vec<_>) = all_diagnostics
        .into_iter()
        .partition(|d| d.path == skill.path);
    let new_sibling_diagnostics: Vec<Diagnostic> = sibling_diagnostics
        .into_iter()
        .filter(|d| !baseline_keys.contains(&diag_key(d)))
        .collect();

    Ok(RoundResult {
        response,
        skill,
        companions,
        candidate_diagnostics,
        new_sibling_diagnostics,
    })
}

/// Generate a new skill from `brief`, screen it against the siblings
/// discovered at `adept::sibling_root(out_dir)`, and repair it for up to
/// `options.max_rounds` rounds. Then generate and validate its eval dataset.
///
/// `out_dir` is the directory the skill *would* be written to (its `SKILL.md`
/// path and companion-file paths are resolved relative to it); this function
/// never writes there. Pass [`CreateReport::files`] to
/// [`crate::writer::write_all_transactionally`] to apply the result.
///
/// # Errors
/// Returns [`CreateError`] if an LLM call fails, a response is malformed, a
/// companion path is unsafe, the generated eval dataset fails schema
/// validation, or an underlying lint/format/I/O operation fails.
pub async fn create_skill(
    client: &dyn LlmClient,
    brief: &str,
    out_dir: &Path,
    options: &CreateOptions,
) -> Result<CreateReport, CreateError> {
    let linter = Linter::new(options.lint_config.clone())?;

    let skill_md_path = out_dir.join("SKILL.md");
    // `adept::sibling_root` expects a path to the skill's own SKILL.md (or
    // an *existing* skill directory it can `is_dir()`-detect); `out_dir`
    // itself may not exist yet, so pass the file path, whose parent is
    // always `out_dir` regardless of whether either exists on disk.
    let sibling_set = SkillSet::discover(adept::sibling_root(&skill_md_path)).unwrap_or_default();
    let siblings: Vec<Skill> = sibling_set
        .skills
        .into_iter()
        .filter(|s| s.path != skill_md_path)
        .collect();
    let siblings_found = !siblings.is_empty();

    let baseline_set = SkillSet {
        skills: siblings.clone(),
        errors: sibling_set.errors,
    };
    let baseline_diagnostics = linter.lint_set(&baseline_set);
    let baseline_keys: HashSet<(PathBuf, &'static str, String)> =
        baseline_diagnostics.iter().map(diag_key).collect();

    let mut best: Option<RoundResult> = None;
    let mut rounds_used = 0;
    let mut response = request_generate(client, brief, &siblings, &options.model).await?;
    apply_name_override(&mut response, options.name_override.as_deref());

    let siblings_len = siblings.len();
    let mut combined_skills: Vec<Skill> = siblings.to_vec();

    loop {
        rounds_used += 1;
        let round = screen_round(
            out_dir,
            response,
            &mut combined_skills,
            siblings_len,
            &baseline_keys,
            &linter,
            &options.fmt_config,
        )?;
        let gate_passes = round.gate_passes();

        let is_better = match &best {
            None => true,
            Some(current_best) => {
                gate::improves_on_counts(current_best.combined_counts(), round.combined_counts())
            }
        };

        let will_break = gate_passes || rounds_used >= options.max_rounds;
        // The repair prompt must describe *this* round's candidate, not
        // whichever candidate is currently winning: `best` may be a
        // different (better-scoring) candidate from an earlier round, and
        // its diagnostics were produced by a different response than the
        // one we are about to show the model for repair. Only built when a
        // repair round will actually be sent — the final round never needs
        // it.
        let repair_input = if will_break {
            None
        } else {
            Some((round.response.clone(), round.combined()))
        };

        if is_better {
            best = Some(round);
        }

        if will_break {
            break;
        }

        let (round_response_for_repair, diagnostics_for_repair) =
            repair_input.expect("not breaking: repair_input was built above");
        response = request_repair(
            client,
            brief,
            &round_response_for_repair,
            &diagnostics_for_repair,
            &options.model,
        )
        .await?;
        apply_name_override(&mut response, options.name_override.as_deref());
    }

    let best = best.expect("the loop always runs at least one round");
    let outcome = if best.gate_passes() {
        CreateOutcome::Clean
    } else {
        CreateOutcome::BestEffort
    };

    let (eval_cases, duplicate_eval_cases_dropped) =
        generate_evals_with_drop_count(client, &best.skill, brief, options).await?;
    let eval_jsonl = evals::to_jsonl(&eval_cases);

    let mut files: BTreeMap<PathBuf, String> = BTreeMap::new();
    files.insert(best.skill.path.clone(), best.skill.source.clone());
    for (path, contents) in &best.companions {
        files.insert(path.clone(), contents.clone());
    }
    files.insert(out_dir.join("evals").join("evals.jsonl"), eval_jsonl);

    Ok(CreateReport {
        skill_name: best.skill.frontmatter.name.clone(),
        rounds_used,
        siblings_found,
        candidate_diagnostics: best.candidate_diagnostics,
        new_sibling_diagnostics: best.new_sibling_diagnostics,
        eval_cases,
        duplicate_eval_cases_dropped,
        files,
        outcome,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockLlmClient;
    use adept::Tokenizer;

    fn base_options() -> CreateOptions {
        CreateOptions::for_model("test-model", Tokenizer::O200kBase)
    }

    fn valid_generate_json(name: &str, description: &str, body: &str) -> String {
        serde_json::json!({
            "name": name,
            "description": description,
            "disable_model_invocation": false,
            "body": body,
            "companion_files": [],
        })
        .to_string()
    }

    fn valid_eval_json(n: usize) -> String {
        let cases: Vec<_> = (0..n)
            .map(|i| {
                serde_json::json!({
                    "prompt": format!("prompt {i}"),
                    "assertions": [{"kind": "contains", "value": "ok"}],
                })
            })
            .collect();
        serde_json::json!({ "cases": cases }).to_string()
    }

    fn clean_body() -> &'static str {
        "# Demo Skill\n\n## Overview\n\nDoes the one thing this skill is for.\n\n## Steps\n\n1. Read the input.\n2. Produce the output.\n"
    }

    #[tokio::test]
    async fn first_response_fails_lint_second_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("demo-skill");

        // Round 1: description too short (SL201) — an Error/Warning finding.
        let bad = valid_generate_json("demo-skill", "short", clean_body());
        // Round 2: a description satisfying the token/trigger-phrase rules.
        let good = valid_generate_json(
            "demo-skill",
            "Extracts structured data from PDF forms. Use when the user needs form fields pulled out programmatically. Do not use for scanned image-only PDFs.",
            clean_body(),
        );
        let eval = valid_eval_json(10);
        let mock = MockLlmClient::with_texts(vec![bad, good, eval]);

        let options = base_options();
        let report = create_skill(&mock, "Extract PDF form data", &out_dir, &options)
            .await
            .unwrap();

        assert!(report.is_clean(), "{report:?}");
        assert_eq!(report.rounds_used, 2);
        assert!(report.candidate_diagnostics.is_empty());
        assert!(!report.siblings_found);
        assert_eq!(report.eval_cases.len(), 10);
        assert!(report.files.contains_key(&out_dir.join("SKILL.md")));
        assert!(report
            .files
            .contains_key(&out_dir.join("evals").join("evals.jsonl")));
    }

    #[tokio::test]
    async fn never_passing_mock_still_yields_best_effort() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("demo-skill");

        // Every round: a description far too short to pass SL201/SL105-adjacent
        // checks (kept identical so neither round improves on the other).
        let bad = valid_generate_json("demo-skill", "short", clean_body());
        let eval = valid_eval_json(10);
        let mock = MockLlmClient::with_texts(vec![bad.clone(), bad, eval]);

        let options = base_options();
        let report = create_skill(&mock, "Extract PDF form data", &out_dir, &options)
            .await
            .unwrap();

        assert!(!report.is_clean());
        assert_eq!(report.outcome, CreateOutcome::BestEffort);
        assert_eq!(report.rounds_used, 2);
        assert!(!report.candidate_diagnostics.is_empty());
        // Still carries the candidate forward, distinguishably from clean.
        assert!(report.files.contains_key(&out_dir.join("SKILL.md")));
    }

    #[tokio::test]
    async fn info_only_candidate_is_accepted_not_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("demo-skill");

        // No top-level H1 (SL102, Warning by default) is the only expected
        // finding; `create`'s gate applies whatever severity the effective
        // `LintConfig` resolves, so demoting SL102 to Info here is exactly
        // the config contract this pins: a user who demotes a rule moves
        // the gate with it, without a `create`-specific threshold.
        let body = "## Overview\n\nDoes the one thing this skill is for.\n\n## Steps\n\n1. Read the input.\n2. Produce the output.\n";
        let good = valid_generate_json(
            "demo-skill",
            "Extracts structured data from PDF forms. Use when the user needs form fields pulled out programmatically. Do not use for scanned image-only PDFs.",
            body,
        );
        let eval = valid_eval_json(10);
        let mock = MockLlmClient::with_texts(vec![good, eval]);

        let mut options = base_options();
        options
            .lint_config
            .severity_overrides
            .insert("SL102".to_string(), adept::Severity::Info);
        let report = create_skill(&mock, "Extract PDF form data", &out_dir, &options)
            .await
            .unwrap();

        assert!(report.is_clean(), "{report:?}");
        assert_eq!(report.rounds_used, 1);
        assert!(report
            .candidate_diagnostics
            .iter()
            .any(|d| d.code == "SL102"));
        assert!(report
            .candidate_diagnostics
            .iter()
            .all(|d| d.severity == adept::Severity::Info));
    }

    fn write_sibling(dir: &std::path::Path, name: &str, description: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: {description}\n---\n## Overview\n\nSibling body.\n"
            ),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn duplicate_sibling_name_triggers_cross_skill_rule_and_repair_clears_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let sibling_dir = root.join("existing-skill");
        write_sibling(
            &sibling_dir,
            "existing-skill",
            "Handles the existing task. Use when asked about the existing thing.",
        );
        let out_dir = root.join("new-skill");

        // Round 1: same name as the sibling -> SL401 duplicate-skill-name.
        let colliding = valid_generate_json(
            "existing-skill",
            "Extracts structured data from PDF forms. Use when the user needs form fields pulled out programmatically. Do not use for scanned image-only PDFs.",
            clean_body(),
        );
        // Round 2: a distinct name, clearing the collision.
        let fixed = valid_generate_json(
            "new-skill",
            "Extracts structured data from PDF forms. Use when the user needs form fields pulled out programmatically. Do not use for scanned image-only PDFs.",
            clean_body(),
        );
        let eval = valid_eval_json(10);
        let mock = MockLlmClient::with_texts(vec![colliding, fixed, eval]);

        let options = base_options();
        let report = create_skill(&mock, "Extract PDF form data", &out_dir, &options)
            .await
            .unwrap();

        assert!(report.siblings_found);
        assert!(report.is_clean(), "{report:?}");
        assert_eq!(report.rounds_used, 2);
        assert!(report.new_sibling_diagnostics.is_empty());
    }

    #[tokio::test]
    async fn defective_sibling_does_not_block_emission_and_is_never_fixed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let sibling_dir = root.join("broken-skill");
        // A sibling with a description far too short (SL201-adjacent) —
        // pre-existing, deliberately defective.
        write_sibling(&sibling_dir, "broken-skill", "short");
        let sibling_path = sibling_dir.join("SKILL.md");
        let sibling_source_before = std::fs::read_to_string(&sibling_path).unwrap();

        let out_dir = root.join("new-skill");
        let good = valid_generate_json(
            "new-skill",
            "Extracts structured data from PDF forms. Use when the user needs form fields pulled out programmatically. Do not use for scanned image-only PDFs.",
            clean_body(),
        );
        let eval = valid_eval_json(10);
        let mock = MockLlmClient::with_texts(vec![good, eval]);

        let options = base_options();
        let report = create_skill(&mock, "Extract PDF form data", &out_dir, &options)
            .await
            .unwrap();

        assert!(report.is_clean(), "{report:?}");
        assert!(report.new_sibling_diagnostics.is_empty());
        // The sibling file on disk is untouched — create never writes, and
        // never targets a sibling in its own `files` batch either.
        assert!(!report.files.contains_key(&sibling_path));
        assert_eq!(
            std::fs::read_to_string(&sibling_path).unwrap(),
            sibling_source_before
        );
    }

    #[tokio::test]
    async fn eval_dataset_failing_validation_fails_the_run() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("demo-skill");

        let good = valid_generate_json(
            "demo-skill",
            "Extracts structured data from PDF forms. Use when the user needs form fields pulled out programmatically. Do not use for scanned image-only PDFs.",
            clean_body(),
        );
        // Zero cases -> fails adept::evals::validate's non-emptiness check.
        let empty_eval = serde_json::json!({ "cases": [] }).to_string();
        let mock = MockLlmClient::with_texts(vec![good, empty_eval]);

        let options = base_options();
        let err = create_skill(&mock, "Extract PDF form data", &out_dir, &options)
            .await
            .unwrap_err();
        assert!(matches!(err, CreateError::InvalidEvalDataset(_)));
    }

    /// Regression test for the `--name` fix: overriding the name to collide
    /// with a sibling must be caught by the gate every round (the override
    /// is re-applied before each round's screening, so the model can never
    /// escape the collision by choosing a different name) and therefore must
    /// never be silently reported clean.
    #[tokio::test]
    async fn name_override_colliding_with_sibling_is_never_silently_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let sibling_dir = root.join("existing-skill");
        write_sibling(
            &sibling_dir,
            "existing-skill",
            "Handles the existing task. Use when asked about the existing thing.",
        );
        let out_dir = root.join("new-skill");

        // Every round: the model proposes a distinct name, but the override
        // forces it back to the sibling's name every time, so the collision
        // (SL401) can never actually be cleared by repair.
        let candidate = valid_generate_json(
            "new-skill",
            "Extracts structured data from PDF forms. Use when the user needs form fields pulled out programmatically. Do not use for scanned image-only PDFs.",
            clean_body(),
        );
        let eval = valid_eval_json(10);
        let mock = MockLlmClient::with_texts(vec![candidate.clone(), candidate, eval]);

        let mut options = base_options();
        options.name_override = Some("existing-skill".to_string());
        let report = create_skill(&mock, "Extract PDF form data", &out_dir, &options)
            .await
            .unwrap();

        assert!(
            !report.is_clean(),
            "a forced sibling-name collision must never be reported clean: {report:?}"
        );
        assert_eq!(report.outcome, CreateOutcome::BestEffort);
        let all_diagnostics: Vec<_> = report
            .candidate_diagnostics
            .iter()
            .chain(report.new_sibling_diagnostics.iter())
            .collect();
        assert!(
            all_diagnostics.iter().any(|d| d.code == "SL401"),
            "expected SL401 duplicate-skill-name to be reported: {all_diagnostics:?}"
        );
        assert_eq!(report.skill_name, "existing-skill");
    }

    /// Acceptance test for #29 / the content-addressed eval case ids spec:
    /// a dataset written by `create_skill` (via the existing `MockLlmClient`
    /// path, no network I/O) must be `schema_version: 2` with unique,
    /// non-empty ids, and must pass `adept::evals::validate` byte-for-byte —
    /// not just the in-memory `validate_cases` check `generate_evals` already
    /// runs internally.
    #[tokio::test]
    async fn generated_dataset_is_schema_v2_and_passes_validate() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("demo-skill");

        let good = valid_generate_json(
            "demo-skill",
            "Extracts structured data from PDF forms. Use when the user needs form fields pulled out programmatically. Do not use for scanned image-only PDFs.",
            clean_body(),
        );
        let eval = valid_eval_json(5);
        let mock = MockLlmClient::with_texts(vec![good, eval]);

        let options = base_options();
        let report = create_skill(&mock, "Extract PDF form data", &out_dir, &options)
            .await
            .unwrap();

        assert!(report.is_clean(), "{report:?}");
        assert_eq!(report.eval_cases.len(), 5);
        assert!(report
            .eval_cases
            .iter()
            .all(|c| c.schema_version == evals::SCHEMA_VERSION));
        assert!(report.eval_cases.iter().all(|c| !c.id.is_empty()));
        let unique_ids: HashSet<&str> = report.eval_cases.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(unique_ids.len(), report.eval_cases.len());

        let jsonl = report
            .files
            .get(&out_dir.join("evals").join("evals.jsonl"))
            .expect("evals.jsonl written");
        evals::validate(jsonl).expect("generated dataset must satisfy adept::evals::validate");
    }

    /// Content-addressed ids (issue #29): a generated response containing
    /// two identical cases (same `prompt` and `assertions`) must not be
    /// written as two eval cases sharing one id — `create` drops the
    /// duplicate, reports the drop count on the report, and the written
    /// dataset must still pass `adept::evals::validate`.
    #[tokio::test]
    async fn duplicate_generated_cases_are_deduped_and_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let out_dir = tmp.path().join("demo-skill");

        let good = valid_generate_json(
            "demo-skill",
            "Extracts structured data from PDF forms. Use when the user needs form fields pulled out programmatically. Do not use for scanned image-only PDFs.",
            clean_body(),
        );
        let eval = serde_json::json!({
            "cases": [
                {"prompt": "extract the fields", "assertions": [{"kind": "contains", "value": "ok"}]},
                {"prompt": "extract the fields", "assertions": [{"kind": "contains", "value": "ok"}]},
                {"prompt": "summarize it", "assertions": [{"kind": "contains", "value": "ok"}]},
            ]
        })
        .to_string();
        let mock = MockLlmClient::with_texts(vec![good, eval]);

        let options = base_options();
        let report = create_skill(&mock, "Extract PDF form data", &out_dir, &options)
            .await
            .unwrap();

        assert!(report.is_clean(), "{report:?}");
        assert_eq!(report.eval_cases.len(), 2);
        assert_eq!(report.duplicate_eval_cases_dropped, 1);

        let jsonl = report
            .files
            .get(&out_dir.join("evals").join("evals.jsonl"))
            .expect("evals.jsonl written");
        evals::validate(jsonl).expect("deduped dataset must satisfy adept::evals::validate");
    }

    /// Regression test for the `--name` fix: when the override is applied
    /// *before* screening (at candidate-construction time, every round)
    /// rather than patched onto the finished report, a name matching the
    /// output directory produces no `SL004` name-mismatch finding — proving
    /// the override actually participates in the gate the loop already runs,
    /// rather than being invisible to it.
    #[tokio::test]
    async fn name_override_matching_out_dir_produces_no_sl004() {
        let tmp = tempfile::tempdir().unwrap();
        // The model's own candidate name would be "demo-skill", not
        // matching this directory; only the override does.
        let out_dir = tmp.path().join("custom-skill-name");

        let candidate = valid_generate_json(
            "demo-skill",
            "Extracts structured data from PDF forms. Use when the user needs form fields pulled out programmatically. Do not use for scanned image-only PDFs.",
            clean_body(),
        );
        let eval = valid_eval_json(10);
        let mock = MockLlmClient::with_texts(vec![candidate, eval]);

        let mut options = base_options();
        options.name_override = Some("custom-skill-name".to_string());
        let report = create_skill(&mock, "Extract PDF form data", &out_dir, &options)
            .await
            .unwrap();

        assert!(report.is_clean(), "{report:?}");
        assert!(!report
            .candidate_diagnostics
            .iter()
            .any(|d| d.code == "SL004"));
        assert_eq!(report.skill_name, "custom-skill-name");
    }
}
