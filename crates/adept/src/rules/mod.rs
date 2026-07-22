//! The rule engine: [`Rule`], [`Registry`], [`LintConfig`], and [`Linter`].
//!
//! Rules come in two flavors: [`SkillRule`]s that check a single [`Skill`]
//! in isolation, and [`SetRule`]s that check a whole [`SkillSet`] for
//! cross-skill issues (duplicates, overlapping descriptions, etc). Both
//! flavors share the base [`Rule`] metadata (code, name, default severity).

mod cross;
mod description;
mod frontmatter;
mod structure;
mod tokens;

use std::collections::{HashMap, HashSet};

use crate::diagnostic::{Diagnostic, Severity};
use crate::error::AdeptError;
use crate::skill::Skill;
use crate::skillset::SkillSet;
use crate::token::TokenCounter;

/// Shared metadata every rule exposes, regardless of whether it checks a
/// single [`Skill`] or a whole [`SkillSet`].
pub trait Rule {
    /// The stable rule code, e.g. `"SL001"`.
    fn code(&self) -> &'static str;
    /// The kebab-case rule name, e.g. `"missing-description"`.
    fn name(&self) -> &'static str;
    /// The severity this rule reports at unless overridden by [`LintConfig`].
    fn default_severity(&self) -> Severity;
}

/// A rule that checks a single [`Skill`] in isolation.
pub trait SkillRule: Rule {
    /// Check `skill`, returning any diagnostics found. Implementations
    /// should use [`Rule::default_severity`] for the diagnostics they build;
    /// [`Linter`] applies any configured severity override afterwards.
    /// `tokens` is a shared [`TokenCounter`], provided so token-budget rules
    /// don't each construct their own BPE tables.
    fn check(&self, skill: &Skill, config: &LintConfig, tokens: &TokenCounter) -> Vec<Diagnostic>;
}

/// A rule that checks a whole [`SkillSet`] for cross-skill issues.
pub trait SetRule: Rule {
    /// Check `set`, returning any diagnostics found.
    fn check(&self, set: &SkillSet, config: &LintConfig, tokens: &TokenCounter) -> Vec<Diagnostic>;
}

/// Static metadata about a registered rule, independent of how (or whether)
/// it is directly invocable — used for listing, docs, and config lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleMeta {
    /// The stable rule code, e.g. `"SL001"`.
    pub code: &'static str,
    /// The kebab-case rule name, e.g. `"missing-description"`.
    pub name: &'static str,
    /// The default severity for this rule.
    pub default_severity: Severity,
}

/// The set of all known rules.
///
/// [`Registry::new`] registers every built-in rule. Note that `SL003`
/// (`malformed-frontmatter`) has no [`SkillRule`] implementation: it is
/// synthesized directly from [`SkillSet::errors`] by [`Linter::lint_set`],
/// since a skill that failed to parse has no [`Skill`] to run rules against.
/// It is still listed in [`Registry::all_meta`] so it can be looked up,
/// documented, and enabled/disabled like any other rule.
pub struct Registry {
    skill_rules: Vec<Box<dyn SkillRule>>,
    set_rules: Vec<Box<dyn SetRule>>,
    parse_error_meta: Vec<RuleMeta>,
}

impl Registry {
    /// Build the registry containing every built-in rule.
    #[must_use]
    #[allow(clippy::vec_init_then_push)]
    pub fn new() -> Self {
        let mut skill_rules: Vec<Box<dyn SkillRule>> = Vec::new();
        skill_rules.push(Box::new(frontmatter::MissingDescription));
        skill_rules.push(Box::new(frontmatter::MissingName));
        skill_rules.push(Box::new(frontmatter::NameMismatch));
        skill_rules.push(Box::new(frontmatter::InvalidNameFormat));

        skill_rules.push(Box::new(structure::EmptyBody));
        skill_rules.push(Box::new(structure::MissingH1));
        skill_rules.push(Box::new(structure::HeadingLevelSkip));
        skill_rules.push(Box::new(structure::BrokenFileReference));

        skill_rules.push(Box::new(description::TooShort));
        // SL202 (description-too-long) is retired: see rules/description.rs.
        skill_rules.push(Box::new(description::MissingTriggerPhrase));
        skill_rules.push(Box::new(description::FirstPerson));
        skill_rules.push(Box::new(description::RestatesName));
        skill_rules.push(Box::new(description::NoNegativeGuidance));

        skill_rules.push(Box::new(tokens::DescriptionTokenBudget));
        skill_rules.push(Box::new(tokens::BodyTokenBudget));
        skill_rules.push(Box::new(tokens::CompanionFileBloat));

        let mut set_rules: Vec<Box<dyn SetRule>> = Vec::new();
        set_rules.push(Box::new(cross::DuplicateSkillName));
        set_rules.push(Box::new(cross::SimilarDescription));
        set_rules.push(Box::new(cross::OverlappingTriggerPhrasing));

        let parse_error_meta = vec![RuleMeta {
            code: "SL003",
            name: "malformed-frontmatter",
            default_severity: Severity::Error,
        }];

        Self {
            skill_rules,
            set_rules,
            parse_error_meta,
        }
    }

    /// The single-skill rules, in registration order.
    #[must_use]
    pub fn skill_rules(&self) -> &[Box<dyn SkillRule>] {
        &self.skill_rules
    }

    /// The cross-skill rules, in registration order.
    #[must_use]
    pub fn set_rules(&self) -> &[Box<dyn SetRule>] {
        &self.set_rules
    }

    /// Metadata for every registered rule (including `SL003`, which has no
    /// directly-invocable check), sorted by code.
    #[must_use]
    pub fn all_meta(&self) -> Vec<RuleMeta> {
        let mut meta: Vec<RuleMeta> = self
            .skill_rules
            .iter()
            .map(|r| RuleMeta {
                code: r.code(),
                name: r.name(),
                default_severity: r.default_severity(),
            })
            .chain(self.set_rules.iter().map(|r| RuleMeta {
                code: r.code(),
                name: r.name(),
                default_severity: r.default_severity(),
            }))
            .chain(self.parse_error_meta.iter().copied())
            .collect();
        meta.sort_by_key(|m| m.code);
        meta
    }

    /// Look up a rule's metadata by its code (e.g. `"SL001"`).
    pub fn by_code(&self, code: &str) -> Option<RuleMeta> {
        self.all_meta().into_iter().find(|m| m.code == code)
    }

    /// Look up a rule's metadata by its kebab-case name.
    pub fn by_name(&self, name: &str) -> Option<RuleMeta> {
        self.all_meta().into_iter().find(|m| m.name == name)
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration for the [`Linter`]: per-rule enable/disable, severity
/// overrides, and the numeric thresholds used by individual rules.
///
/// Deserializable so a future config file or CLI flags can populate it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LintConfig {
    /// Rule codes or kebab-case names to disable entirely. Matching is
    /// case-sensitive and exact (e.g. `"SL001"` or `"missing-description"`).
    pub disabled: HashSet<String>,

    /// Per-rule severity overrides, keyed by rule code or kebab-case name.
    pub severity_overrides: HashMap<String, Severity>,

    /// Minimum token count for a `description` field.
    ///
    /// Rationale: a description below this is almost certainly too terse to
    /// state both what the skill does and when to use it, which is the two
    /// jobs a description has to do; 6 tokens is roughly "extracts data from
    /// PDF files" with nothing about triggering.
    pub description_min_tokens: usize,

    /// Maximum token count for a `description` field.
    ///
    /// Rationale: descriptions are read by the agent on every turn to decide
    /// whether to trigger a skill; Anthropic's own guidance keeps these to
    /// roughly one or two sentences. 75 `o200k_base` tokens is generously
    /// above two long sentences, so anything beyond it is very likely bloat
    /// rather than useful triggering detail.
    pub description_max_tokens: usize,

    /// Maximum token count for the SKILL.md body (everything after the
    /// frontmatter).
    ///
    /// Rationale: the body is loaded into context in full once a skill
    /// triggers; 1500 `o200k_base` tokens (roughly 1000-1200 words) is
    /// generous for a focused skill while still catching bodies that have
    /// accreted into a dumping ground.
    pub body_max_tokens: usize,

    /// Maximum token count for any single companion file (a file other than
    /// SKILL.md in the skill's directory) before it is flagged as bloat.
    ///
    /// Rationale: companion files (scripts, references) are meant to be
    /// loaded selectively, not all at once; 2000 tokens per file is a loose
    /// ceiling that still flags reference docs that have grown unwieldy.
    pub companion_file_max_tokens: usize,

    /// Jaccard similarity threshold (0.0-1.0) over description word
    /// shingles above which two skills' descriptions are flagged as
    /// suspiciously similar.
    ///
    /// Rationale: 0.6 catches near-duplicate descriptions (paraphrases of
    /// the same trigger conditions) while tolerating skills in the same
    /// domain that legitimately share some vocabulary.
    pub similar_description_threshold: f64,

    /// Jaccard similarity threshold (0.0-1.0) over extracted trigger
    /// phrases above which two skills are flagged as having overlapping
    /// triggering conditions.
    ///
    /// Rationale: trigger phrases are a small, high-signal set of words;
    /// 0.5 overlap between two skills' trigger vocabularies is a strong
    /// signal they'll compete to trigger on the same user requests.
    pub trigger_overlap_threshold: f64,

    /// Which `tiktoken-rs` BPE encoding to count tokens with.
    ///
    /// Rationale: the spec calls for `o200k_base` (GPT-4o family) by
    /// default with `cl100k_base` (GPT-4/GPT-3.5 era) selectable, since
    /// different downstream models tokenize differently and a mismatched
    /// tokenizer under- or over-counts against the real budget.
    pub tokenizer: crate::token::Tokenizer,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            disabled: HashSet::new(),
            severity_overrides: HashMap::new(),
            description_min_tokens: 6,
            description_max_tokens: 75,
            body_max_tokens: 1500,
            companion_file_max_tokens: 2000,
            similar_description_threshold: 0.6,
            trigger_overlap_threshold: 0.5,
            tokenizer: crate::token::Tokenizer::default(),
        }
    }
}

impl LintConfig {
    fn is_enabled(&self, rule: &dyn Rule) -> bool {
        !self.disabled.contains(rule.code()) && !self.disabled.contains(rule.name())
    }

    fn resolve_severity(&self, rule: &dyn Rule) -> Severity {
        self.severity_overrides
            .get(rule.code())
            .or_else(|| self.severity_overrides.get(rule.name()))
            .copied()
            .unwrap_or_else(|| rule.default_severity())
    }

    fn apply_overrides(
        &self,
        rule: &dyn Rule,
        mut diagnostics: Vec<Diagnostic>,
    ) -> Vec<Diagnostic> {
        let severity = self.resolve_severity(rule);
        for d in &mut diagnostics {
            d.severity = severity;
        }
        diagnostics
    }
}

/// The lint entry point: runs every enabled rule and returns sorted
/// diagnostics.
pub struct Linter {
    config: LintConfig,
    registry: Registry,
    token_counter: TokenCounter,
}

impl Linter {
    /// Construct a linter with the given configuration and the default rule
    /// registry, building its [`TokenCounter`] from `config.tokenizer`.
    ///
    /// # Errors
    /// Returns [`AdeptError::TokenizerLoad`] if the configured tokenizer's
    /// `tiktoken-rs` encoding tables fail to load.
    pub fn new(config: LintConfig) -> Result<Self, AdeptError> {
        let token_counter = TokenCounter::new(config.tokenizer)?;
        Ok(Self {
            config,
            registry: Registry::new(),
            token_counter,
        })
    }

    /// The rule registry this linter uses.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The configuration this linter uses.
    #[must_use]
    pub fn config(&self) -> &LintConfig {
        &self.config
    }

    /// Lint a single [`Skill`], running every enabled [`SkillRule`].
    ///
    /// Diagnostics are sorted by `(path, line, column, code)`.
    pub fn lint_skill(&self, skill: &Skill) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for rule in self.registry.skill_rules() {
            if !self.config.is_enabled(rule.as_ref()) {
                continue;
            }
            let found = rule.check(skill, &self.config, &self.token_counter);
            diagnostics.extend(self.config.apply_overrides(rule.as_ref(), found));
        }
        sort_diagnostics(&mut diagnostics);
        diagnostics
    }

    /// Lint a whole [`SkillSet`]: runs [`Self::lint_skill`] over every
    /// successfully parsed skill, every enabled [`SetRule`] over the set as
    /// a whole, and surfaces `set.errors` (skills that failed to parse) as
    /// diagnostics (`SL001`/`SL002`/`SL003`) rather than dropping them.
    ///
    /// Diagnostics are sorted by `(path, line, column, code)`.
    pub fn lint_set(&self, set: &SkillSet) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for skill in &set.skills {
            diagnostics.extend(self.lint_skill(skill));
        }

        for rule in self.registry.set_rules() {
            if !self.config.is_enabled(rule.as_ref()) {
                continue;
            }
            let found = rule.check(set, &self.config, &self.token_counter);
            diagnostics.extend(self.config.apply_overrides(rule.as_ref(), found));
        }

        for (path, err) in &set.errors {
            if let Some(d) = parse_error_diagnostic(path, err) {
                if !self.config.disabled.contains(d.code) {
                    let mut d = d;
                    d.severity = self
                        .config
                        .severity_overrides
                        .get(d.code)
                        .copied()
                        .unwrap_or(d.severity);
                    diagnostics.push(d);
                }
            }
        }

        sort_diagnostics(&mut diagnostics);
        diagnostics
    }
}

fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|a, b| {
        (&a.path, a.line, a.column, a.code).cmp(&(&b.path, b.line, b.column, b.code))
    });
}

/// Convert a parse-time [`AdeptError`] into the corresponding lint
/// diagnostic (`SL001`/`SL002`/`SL003`), if it corresponds to one of those.
fn parse_error_diagnostic(path: &std::path::Path, err: &AdeptError) -> Option<Diagnostic> {
    match err {
        AdeptError::MissingField { field, .. } if *field == "description" => Some(Diagnostic::new(
            "SL001",
            "SKILL.md is missing the required `description` frontmatter field",
            Severity::Error,
            path,
            1,
            1,
        )
        .with_fix_suggestion(
            "add a `description` field stating what the skill does and when to use it",
        )),
        AdeptError::MissingField { field, .. } if *field == "name" => Some(Diagnostic::new(
            "SL002",
            "SKILL.md is missing the required `name` frontmatter field",
            Severity::Error,
            path,
            1,
            1,
        )
        .with_fix_suggestion("add a `name` field matching the skill's directory name")),
        AdeptError::MissingFrontmatter { .. } => Some(Diagnostic::new(
            "SL003",
            "SKILL.md must start with a line containing only '---' to open the YAML frontmatter block",
            Severity::Error,
            path,
            1,
            1,
        )
        .with_fix_suggestion("add an opening `---` line as the first line of the file")),
        AdeptError::UnterminatedFrontmatter { .. } => Some(Diagnostic::new(
            "SL003",
            "SKILL.md frontmatter is opened with '---' but never closed",
            Severity::Error,
            path,
            1,
            1,
        )
        .with_fix_suggestion("add a closing `---` line after the frontmatter fields")),
        AdeptError::InvalidYaml { source, .. } => Some(Diagnostic::new(
            "SL003",
            format!("SKILL.md frontmatter is not valid YAML: {source}"),
            Severity::Error,
            path,
            1,
            1,
        )),
        AdeptError::FrontmatterNotMapping { .. } => Some(Diagnostic::new(
            "SL003",
            "SKILL.md frontmatter must be a YAML mapping (key: value pairs)",
            Severity::Error,
            path,
            1,
            1,
        )),
        AdeptError::InvalidFieldType { field, .. } => Some(Diagnostic::new(
            "SL003",
            format!("SKILL.md frontmatter field `{field}` must be a string"),
            Severity::Error,
            path,
            1,
            1,
        )),
        AdeptError::MissingField { .. }
        | AdeptError::Io { .. }
        | AdeptError::WalkDir(_)
        | AdeptError::NotFound(_)
        | AdeptError::TokenizerLoad { .. }
        | AdeptError::Json(_) => None,
    }
}
