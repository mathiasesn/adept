//! Argument parsing for the `adept` binary.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// An extremely fast linter and formatter for Agent Skills.
#[derive(Debug, Parser)]
#[command(name = "adept", version, about, long_about = None)]
pub struct Cli {
    /// Path to a specific `adept.toml` config file (skips discovery).
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Disable colored output.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Suppress non-essential output (summary lines, progress).
    #[arg(short, long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Lint one or more SKILL.md files or directories of skills.
    Check(CheckArgs),
    /// Format SKILL.md files in place.
    Fmt(FmtArgs),
    /// Score a skill's triggering accuracy, token bloat, and overlaps using an LLM.
    Score(ScoreArgs),
    /// Run adept as an MCP server over stdio.
    Mcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

#[derive(Debug, Parser)]
pub struct CheckArgs {
    /// Files or directories to check.
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,

    /// Only run these rules (rule codes or kebab-case names, comma-separated
    /// or repeated).
    #[arg(long, value_delimiter = ',')]
    pub select: Vec<String>,

    /// Disable these rules (rule codes or kebab-case names, comma-separated
    /// or repeated).
    #[arg(long, value_delimiter = ',')]
    pub ignore: Vec<String>,

    /// Print per-rule diagnostic counts instead of (in addition to) the
    /// diagnostics themselves.
    #[arg(long)]
    pub statistics: bool,

    /// Always exit 0, even if diagnostics were found.
    #[arg(long)]
    pub exit_zero: bool,
}

#[derive(Debug, Parser)]
pub struct FmtArgs {
    /// Files or directories to format.
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,

    /// Don't write any files; exit 1 if any file would be reformatted and
    /// print a unified diff.
    #[arg(long)]
    pub check: bool,

    /// Print a unified diff of what would change, without writing files.
    #[arg(long)]
    pub diff: bool,

    /// Target line width for prose reflow.
    #[arg(long)]
    pub line_width: Option<usize>,
}

#[derive(Debug, Parser)]
pub struct ScoreArgs {
    /// Path to the skill (SKILL.md file or skill directory) to score.
    pub path: PathBuf,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,

    /// The model to use for scoring (falls back to `ADEPT_MODEL`).
    #[arg(long)]
    pub model: Option<String>,

    /// The OpenAI-compatible base URL (falls back to `ADEPT_BASE_URL`).
    #[arg(long)]
    pub base_url: Option<String>,

    /// Number of candidate triggering prompts to generate.
    #[arg(long)]
    pub num_prompts: Option<usize>,

    /// Sampling seed for reproducible prompt generation.
    #[arg(long)]
    pub seed: Option<u64>,

    /// Number of independent judge samples per prompt (majority vote).
    #[arg(long)]
    pub judge_samples: Option<usize>,
}
