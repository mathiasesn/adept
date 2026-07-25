//! `adept`: an extremely fast linter and formatter for Agent Skills.
//!
//! This binary wires together the `adept` (core/rules), `adept_fmt`,
//! `adept_score`, and `adept_fix` library crates into five subcommands:
//! `check`, `fmt`, `score`, `fix`, and `mcp`.

mod cli;
mod commands;
mod config;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use clap::Parser;

use cli::{Cli, Command};
use config::AdeptConfig;

fn main() {
    let cli = Cli::parse();
    let exit_code = run(&cli);
    std::process::exit(exit_code);
}

fn run(cli: &Cli) -> i32 {
    let color = !cli.no_color && std::io::stdout().is_terminal();

    match &cli.command {
        Command::Check(args) => {
            let target = first_path(&args.paths).to_path_buf();
            let config = match load_config(cli.config.as_deref(), &target) {
                Ok(config) => config,
                Err(code) => return code,
            };
            commands::check::run(args, &config, color, cli.quiet)
        }
        Command::Fmt(args) => {
            let target = first_path(&args.paths).to_path_buf();
            let config = match load_config(cli.config.as_deref(), &target) {
                Ok(config) => config,
                Err(code) => return code,
            };
            commands::fmt::run(args, &config, cli.quiet)
        }
        Command::Score(args) => {
            let config = match load_config(cli.config.as_deref(), &args.path) {
                Ok(config) => config,
                Err(code) => return code,
            };
            commands::score::run(args, &config)
        }
        Command::Fix(args) => {
            let target = first_path(&args.paths).to_path_buf();
            let config = match load_config(cli.config.as_deref(), &target) {
                Ok(config) => config,
                Err(code) => return code,
            };
            commands::fix::run(args, &config, cli.quiet)
        }
        Command::Mcp => commands::mcp::serve(),
    }
}

fn first_path(paths: &[PathBuf]) -> &Path {
    paths
        .first()
        .map(PathBuf::as_path)
        .unwrap_or_else(|| Path::new("."))
}

/// Load the effective config, printing a usage error (exit code 2) if an
/// explicit `--config` path fails to load.
fn load_config(explicit: Option<&Path>, target: &Path) -> Result<AdeptConfig, i32> {
    match config::resolve_config(explicit, target) {
        Ok(config) => Ok(config),
        Err(err) => {
            eprintln!("adept: error: {err}");
            Err(2)
        }
    }
}
