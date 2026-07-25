//! Options controlling which diagnostics [`crate::fix_skill`] attempts to
//! fix and how.

use adept::{LintConfig, Tokenizer};
use adept_fmt::FmtConfig;

/// Default value for [`FixOptions::max_rounds`].
///
/// Rationale: round 1 attempts the fix; round 2 gives the model a single
/// retry if round 1 made progress (strictly shrank the fixable set) without
/// fully resolving it. Beyond that, further rounds have sharply
/// diminishing returns relative to their LLM-call cost.
pub const DEFAULT_MAX_ROUNDS: usize = 2;

/// Options controlling [`crate::fix_skill`]: which model to call, which
/// diagnostics are eligible, and the lint/format configuration candidates
/// are judged and canonicalized against.
#[derive(Debug, Clone)]
pub struct FixOptions {
    /// The model to use for all LLM calls.
    pub model: String,
    /// Which `tiktoken-rs` BPE encoding to count tokens with. Should match
    /// `lint_config.tokenizer`; kept as a separate field (mirroring
    /// `adept_score::ScoreOptions`) so callers can construct a
    /// [`FixOptions`] without first building a full [`LintConfig`].
    pub tokenizer: Tokenizer,
    /// The maximum number of fix rounds to attempt before giving up.
    pub max_rounds: usize,
    /// The lint configuration diagnostics are found and candidates are
    /// re-checked against. Must match the configuration the caller
    /// originally linted `skill` with, or "resolved"/"residual" will not
    /// line up with what the caller already knows about.
    pub lint_config: LintConfig,
    /// The formatter configuration used to canonicalize each candidate's
    /// rewritten SKILL.md source before it is re-linted or diffed.
    pub fmt_config: FmtConfig,
    /// Rule codes (e.g. `"SL301"`) or kebab-case names (e.g.
    /// `"description-tokens-over-budget"`) to restrict fixing to. Empty
    /// means no restriction (every LLM-fixable diagnostic is eligible).
    pub select: Vec<String>,
    /// Rule codes or kebab-case names to exclude from fixing, applied after
    /// `select`.
    pub ignore: Vec<String>,
}

impl FixOptions {
    /// The default options for fixing with `model`, using `tokenizer` for
    /// both token counting and the embedded [`LintConfig`].
    #[must_use]
    pub fn for_model(model: impl Into<String>, tokenizer: Tokenizer) -> Self {
        Self {
            model: model.into(),
            tokenizer,
            max_rounds: DEFAULT_MAX_ROUNDS,
            lint_config: LintConfig {
                tokenizer,
                ..LintConfig::default()
            },
            fmt_config: FmtConfig::default(),
            select: Vec::new(),
            ignore: Vec::new(),
        }
    }
}
