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
    ///
    /// Independent of `-v`: `--quiet` only trims *stdout* results, while
    /// `-v` only adds *stderr* diagnostics, so `-q -vv` is meaningful.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Increase logging verbosity on stderr (`-v` info, `-vv` debug,
    /// `-vvv` trace). Off by default; `ADEPT_LOG` overrides this with
    /// `EnvFilter` directive syntax.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Lint one or more SKILL.md files or directories of skills.
    Check(CheckArgs),
    /// Format SKILL.md files in place.
    Fmt(FmtArgs),
    /// Evaluate a skill's triggering accuracy, token bloat, overlaps, and
    /// eval-dataset performance.
    Eval(EvalArgs),
    /// Fix LLM-fixable lint diagnostics in one or more SKILL.md files using an LLM.
    Fix(FixArgs),
    /// Generate a new skill (plus a synthetic eval dataset) from a brief, using an LLM.
    Create(CreateArgs),
    /// Run adept as an MCP server over stdio.
    Mcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

/// CLI-facing mirror of [`adept::Tokenizer`] (clap's `ValueEnum` can't be
/// derived on a type owned by another crate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TokenizerArg {
    /// The `o200k_base` encoding (GPT-4o family). The default.
    O200kBase,
    /// The `cl100k_base` encoding (GPT-4/GPT-3.5 era).
    Cl100kBase,
}

impl From<TokenizerArg> for adept::Tokenizer {
    fn from(value: TokenizerArg) -> Self {
        match value {
            TokenizerArg::O200kBase => adept::Tokenizer::O200kBase,
            TokenizerArg::Cl100kBase => adept::Tokenizer::Cl100kBase,
        }
    }
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

    /// Which `tiktoken-rs` BPE encoding to count tokens with (default
    /// `o200k-base`; overrides the config file's `[lint] tokenizer`).
    #[arg(long, value_enum)]
    pub tokenizer: Option<TokenizerArg>,
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
pub struct EvalArgs {
    /// Path to the skill (SKILL.md file or skill directory) to evaluate.
    pub path: PathBuf,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,

    /// The model to use for the triggering/token-bloat/overlap analyses
    /// (falls back to `ADEPT_MODEL`).
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

    /// Which `tiktoken-rs` BPE encoding to use for token-bloat analysis
    /// (default `o200k-base`; overrides the config file's `[eval]
    /// tokenizer`).
    #[arg(long, value_enum)]
    pub tokenizer: Option<TokenizerArg>,

    /// Write verbatim request/response artifacts for every LLM call into a
    /// new timestamped subfolder of this directory. Overrides the config
    /// file's `[eval] capture_dir`; a relative path resolves against the
    /// current working directory.
    #[arg(long, value_name = "DIR")]
    pub capture_dir: Option<PathBuf>,

    /// Path to a `results.jsonl` sidecar (harness-produced run results) to
    /// grade the skill's `evals/evals.jsonl` dataset against. Enables the
    /// `evals` analysis by default when supplied.
    #[arg(long, value_name = "PATH")]
    pub results: Option<PathBuf>,

    /// Override the eval dataset path (defaults to `evals/evals.jsonl`
    /// relative to the skill directory).
    #[arg(long, value_name = "PATH")]
    pub evals: Option<PathBuf>,

    /// Only run these analyses (`triggering`, `token-bloat`, `overlap`,
    /// `evals`; comma-separated or repeated).
    #[arg(long, value_delimiter = ',')]
    pub select: Vec<String>,

    /// Skip these analyses (`triggering`, `token-bloat`, `overlap`,
    /// `evals`; comma-separated or repeated).
    #[arg(long, value_delimiter = ',')]
    pub ignore: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct FixArgs {
    /// Files or directories to fix.
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,

    /// Write the fixed files to disk.
    #[arg(short, long, conflicts_with = "check")]
    pub write: bool,

    /// Don't write any files; exit 1 if anything would change.
    #[arg(long, conflicts_with = "write")]
    pub check: bool,

    /// Print a unified diff of what would change, without writing files.
    #[arg(long)]
    pub diff: bool,

    /// Only run these rules (rule codes or kebab-case names, comma-separated
    /// or repeated).
    #[arg(long, value_delimiter = ',')]
    pub select: Vec<String>,

    /// Disable these rules (rule codes or kebab-case names, comma-separated
    /// or repeated).
    #[arg(long, value_delimiter = ',')]
    pub ignore: Vec<String>,

    /// The model to use for fixing (falls back to `ADEPT_MODEL`).
    #[arg(long)]
    pub model: Option<String>,

    /// The OpenAI-compatible base URL (falls back to `ADEPT_BASE_URL`).
    #[arg(long)]
    pub base_url: Option<String>,

    /// The maximum number of fix rounds to attempt before giving up.
    #[arg(long)]
    pub max_rounds: Option<usize>,

    /// Which `tiktoken-rs` BPE encoding to count tokens with (default
    /// `o200k-base`; overrides the config file's `[fix] tokenizer`).
    #[arg(long, value_enum)]
    pub tokenizer: Option<TokenizerArg>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,

    /// Write verbatim request/response artifacts for every LLM call into a
    /// new timestamped subfolder of this directory. Overrides the config
    /// file's `[fix] capture_dir`; a relative path resolves against the
    /// current working directory.
    #[arg(long, value_name = "DIR")]
    pub capture_dir: Option<PathBuf>,
}

#[derive(Debug, Parser)]
pub struct CreateArgs {
    /// Read the task brief from a file instead of stdin/the interactive
    /// prompt. Takes precedence over stdin and the interactive prompt.
    #[arg(long, value_name = "PATH")]
    pub from_file: Option<PathBuf>,

    /// Destination directory for the new skill (defaults to the current
    /// directory).
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,

    /// Override the skill name the model derives from the brief.
    #[arg(long)]
    pub name: Option<String>,

    /// Write the generated skill and eval dataset to disk. Preview (compute
    /// and print, write nothing) is the default.
    #[arg(short, long)]
    pub write: bool,

    /// Allow writing into an existing skill directory (one already
    /// containing a `SKILL.md`). Without this, `create` refuses to clobber
    /// it and exits 2.
    #[arg(long)]
    pub overwrite: bool,

    /// The model to use for generation (falls back to `ADEPT_MODEL`).
    #[arg(long)]
    pub model: Option<String>,

    /// The OpenAI-compatible base URL (falls back to `ADEPT_BASE_URL`).
    #[arg(long)]
    pub base_url: Option<String>,

    /// Which `tiktoken-rs` BPE encoding to count tokens with (default
    /// `o200k-base`; overrides the config file's `[create] tokenizer`).
    #[arg(long, value_enum)]
    pub tokenizer: Option<TokenizerArg>,

    /// The maximum number of authoring rounds to attempt before giving up.
    #[arg(long)]
    pub max_rounds: Option<usize>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,

    /// Write verbatim request/response artifacts for every LLM call into a
    /// new timestamped subfolder of this directory. Overrides the config
    /// file's `[create] capture_dir`; a relative path resolves against the
    /// current working directory.
    #[arg(long, value_name = "DIR")]
    pub capture_dir: Option<PathBuf>,
}
