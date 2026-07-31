//! Options controlling [`crate::create_skill`].

use adept::{LintConfig, Tokenizer};
use adept_fmt::FmtConfig;

/// Default value for [`CreateOptions::max_rounds`].
///
/// Mirrors [`crate::fix::DEFAULT_MAX_ROUNDS`]'s rationale: round 1 generates,
/// round 2 gives the model one repair attempt informed by the first round's
/// diagnostics. Beyond that, further rounds have sharply diminishing returns
/// relative to their LLM-call cost.
pub const DEFAULT_MAX_ROUNDS: usize = 2;

/// Default number of synthetic eval cases [`crate::create_skill`] generates.
///
/// Ten is a round default large enough to cover a typical skill's branches
/// without doubling generation cost again.
pub const DEFAULT_EVAL_CASES: usize = 10;

/// Options controlling [`crate::create_skill`]: which model to call, the
/// lint/format configuration a candidate is screened and canonicalized
/// against, and how many repair rounds / eval cases to produce.
#[derive(Debug, Clone)]
pub struct CreateOptions {
    /// The model to use for both the authoring and eval-generation calls.
    pub model: String,
    /// Which `tiktoken-rs` BPE encoding to count tokens with. Should match
    /// `lint_config.tokenizer`.
    pub tokenizer: Tokenizer,
    /// The maximum number of authoring rounds (initial generation plus
    /// repair attempts) before giving up and carrying forward the
    /// best-scoring candidate seen.
    pub max_rounds: usize,
    /// How many synthetic eval cases to request in the eval-dataset
    /// generation call.
    pub eval_cases: usize,
    /// The lint configuration diagnostics are found under and severity is
    /// resolved by. This is also what defines the repair gate: zero `Error`
    /// and zero `Warning` diagnostics, whatever this configuration resolves
    /// each rule's severity to.
    pub lint_config: LintConfig,
    /// The formatter configuration used to canonicalize the candidate's
    /// SKILL.md source before it is linted, diffed, or emitted.
    pub fmt_config: FmtConfig,
}

impl CreateOptions {
    /// The default options for creating with `model`, using `tokenizer` for
    /// both token counting and the embedded [`LintConfig`].
    #[must_use]
    pub fn for_model(model: impl Into<String>, tokenizer: Tokenizer) -> Self {
        Self {
            model: model.into(),
            tokenizer,
            max_rounds: DEFAULT_MAX_ROUNDS,
            eval_cases: DEFAULT_EVAL_CASES,
            lint_config: LintConfig {
                tokenizer,
                ..LintConfig::default()
            },
            fmt_config: FmtConfig::default(),
        }
    }
}
