//! `adept eval`'s LLM-assisted analyses: triggering accuracy, token-bloat
//! analysis, and cross-skill overlap/conflict detection for Agent Skills.
//!
//! The async seam is [`crate::llm::LlmClient`]: everything in this module
//! that talks to a model goes through a `&dyn LlmClient`, so callers can
//! pass [`crate::llm::OpenAiCompatClient`] for real evaluation or
//! [`crate::llm::MockLlmClient`] for offline tests. All public entry points
//! here (`triggering::score_triggering`, `tokens::analyze_token_bloat`,
//! `overlap::detect_overlaps`, and [`eval_skill`]) are `async fn`; callers
//! (e.g. `adept_cli`) are expected to drive them from a `tokio` runtime
//! (`#[tokio::main]` or `Runtime::block_on`) — this crate does not spin up
//! its own runtime.

mod overlap;
pub mod prompts;
mod report;
mod tokens;
mod triggering;

pub use overlap::{
    description_similarity, detect_overlaps, shortlist_candidates, OverlapAdjudication,
    OverlapCandidate, DEFAULT_SIMILARITY_THRESHOLD,
};
pub use prompts::{
    GENERATE_TRIGGER_PROMPTS_SYSTEM, GENERATE_TRIGGER_PROMPTS_USER_TEMPLATE, JUDGE_TRIGGER_SYSTEM,
    JUDGE_TRIGGER_USER_TEMPLATE, OVERLAP_ADJUDICATION_SYSTEM, OVERLAP_ADJUDICATION_USER_TEMPLATE,
    PROMPT_VERSION, TOKEN_BLOAT_SUGGESTIONS_SYSTEM, TOKEN_BLOAT_SUGGESTIONS_USER_TEMPLATE,
};
pub use report::EvalReport;
pub use tokens::{analyze_token_bloat, discover_companion_files, TokenBloatReport};
pub use triggering::{
    precision_recall_f1, score_triggering, CandidatePrompt, Metrics, PromptJudgement, PromptLabel,
    TriggeringOptions, DEFAULT_NUM_PROMPTS,
};

use adept::{Skill, TokenCounter};

use crate::llm::LlmClient;

/// Errors from evaluating a skill: LLM transport failures or malformed
/// LLM-produced JSON. Distinct from [`adept::AdeptError`] (parsing/I/O of
/// the skill itself), which callers surface separately, and from
/// [`adept::evals::EvalError`] (eval-*dataset* parse errors) — these are two
/// separate types in two separate crates and must not be conflated.
#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    /// The LLM client returned an error (network, non-2xx status, timeout).
    #[error("LLM request failed: {0}")]
    Llm(#[from] crate::llm::LlmError),

    /// A response that should have been the documented JSON shape wasn't.
    #[error("malformed LLM response ({0})")]
    MalformedLlmJson(String),

    /// The core crate failed to construct a [`adept::TokenCounter`] (its
    /// `tiktoken-rs` encoding tables failed to load).
    #[error(transparent)]
    Adept(#[from] adept::AdeptError),
}

/// Options controlling which analyses [`eval_skill`] runs and how.
#[derive(Debug, Clone)]
pub struct EvalOptions {
    /// The model to use for all LLM calls.
    pub model: String,
    /// Options for the triggering-accuracy analysis. `None` skips it.
    pub triggering: Option<TriggeringOptions>,
    /// Whether to run token-bloat analysis.
    pub token_bloat: bool,
    /// The Jaccard-similarity threshold for shortlisting overlap
    /// candidates against `skillset`. Only used if `skillset` is non-empty.
    pub overlap_similarity_threshold: f64,
    /// Which `tiktoken-rs` BPE encoding to use for token-bloat analysis.
    /// Defaults to `o200k_base`; CLI-wireable to `cl100k_base`.
    pub tokenizer: adept::Tokenizer,
}

impl EvalOptions {
    /// The default options for evaluating with `model`.
    ///
    /// The model name has to reach both [`EvalOptions::model`] and
    /// [`TriggeringOptions::model`]; this is the one place that wiring
    /// lives, so the `eval` CLI and the MCP `eval_skill` tool can't drift
    /// apart on defaults.
    #[must_use]
    pub fn for_model(model: impl Into<String>, tokenizer: adept::Tokenizer) -> Self {
        let model = model.into();
        Self {
            triggering: Some(TriggeringOptions {
                model: model.clone(),
                ..TriggeringOptions::default()
            }),
            model,
            tokenizer,
            ..Self::default()
        }
    }
}

impl Default for EvalOptions {
    fn default() -> Self {
        Self {
            model: String::new(),
            triggering: Some(TriggeringOptions::default()),
            token_bloat: true,
            overlap_similarity_threshold: DEFAULT_SIMILARITY_THRESHOLD,
            tokenizer: adept::Tokenizer::default(),
        }
    }
}

/// Run all requested analyses for `skill` (per `options`), against the
/// wider `skillset` for overlap detection (pass a slice containing `skill`
/// plus its siblings; results are filtered down to pairs involving `skill`).
///
/// This is the single entry point `adept_cli` is expected to call for
/// `adept eval <path>`.
///
/// # Errors
/// Returns [`EvalError`] if any LLM call fails or returns malformed JSON.
pub async fn eval_skill(
    client: &dyn LlmClient,
    skill: &Skill,
    skillset: &[Skill],
    options: &EvalOptions,
) -> Result<EvalReport, EvalError> {
    let mut report = EvalReport::new(skill.frontmatter.name.clone());

    if let Some(trigger_options) = &options.triggering {
        let mut trigger_options = trigger_options.clone();
        if trigger_options.model.is_empty() {
            trigger_options.model = options.model.clone();
        }
        report.triggering = Some(score_triggering(client, skill, &trigger_options).await?);
    }

    if options.token_bloat {
        let counter = TokenCounter::new(options.tokenizer)?;
        report.token_bloat =
            Some(analyze_token_bloat(client, skill, &counter, &options.model).await?);
    }

    if !skillset.is_empty() {
        report.overlaps = detect_overlaps(
            client,
            skillset,
            &options.model,
            options.overlap_similarity_threshold,
        )
        .await?
        .into_iter()
        .filter(|adjudication| {
            adjudication.skill_a == skill.frontmatter.name
                || adjudication.skill_b == skill.frontmatter.name
        })
        .collect();
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlmClient;
    use std::io::Write;

    fn write_skill(dir: &std::path::Path, name: &str, description: &str) -> std::path::PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("SKILL.md");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            "---\nname: {name}\ndescription: {description}\n---\nBody text for {name}."
        )
        .unwrap();
        path
    }

    fn tempdir(tag: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let dir = std::env::temp_dir().join(format!(
            "adept_agent_eval_lib_test_{tag}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn eval_skill_end_to_end_with_mock_client() {
        let dir = tempdir("e2e");
        let path = write_skill(
            &dir,
            "pdf-filler",
            "Fills PDF forms with user-supplied data",
        );
        let skill = adept::parse_skill(&path).unwrap();

        let mock = MockLlmClient::with_texts(vec![
            // 1. generate 2 trigger prompts
            r#"{"prompts": [{"text": "Fill out this W-9", "label": "positive"}, {"text": "What's the weather?", "label": "negative"}]}"#,
            // 2. judge prompt 1
            r#"{"would_trigger": true, "reasoning": "matches"}"#,
            // 3. judge prompt 2
            r#"{"would_trigger": false, "reasoning": "unrelated"}"#,
            // 4. token bloat suggestions
            r#"{"suggestions": []}"#,
        ]);

        let mut trigger_options = TriggeringOptions {
            num_prompts: 2,
            ..Default::default()
        };
        trigger_options.model = "test-model".to_string();

        let options = EvalOptions {
            model: "test-model".to_string(),
            triggering: Some(trigger_options),
            token_bloat: true,
            overlap_similarity_threshold: DEFAULT_SIMILARITY_THRESHOLD,
            tokenizer: adept::Tokenizer::default(),
        };

        let report = eval_skill(&mock, &skill, &[], &options).await.unwrap();

        assert_eq!(report.skill_name, "pdf-filler");
        assert_eq!(report.prompt_version, PROMPT_VERSION);
        let triggering = report.triggering.unwrap();
        assert_eq!(triggering.metrics.correct, 2);
        assert_eq!(triggering.metrics.precision, 1.0);
        assert_eq!(triggering.metrics.recall, 1.0);
        assert!(report.token_bloat.is_some());
        assert!(report.overlaps.is_empty());
        assert_eq!(mock.call_count(), 4);

        std::fs::remove_dir_all(&dir).ok();
    }
}
