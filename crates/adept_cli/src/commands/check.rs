//! `adept check`.

use std::collections::HashMap;

use adept::{Diagnostic, LintConfig, Linter, Registry, Severity, SkillSet};

use crate::cli::{CheckArgs, OutputFormat};
use crate::config::AdeptConfig;

/// Exit code contract: 0 = clean, 1 = diagnostics found, 2 = usage/IO error.
pub const EXIT_OK: i32 = 0;
pub const EXIT_DIAGNOSTICS: i32 = 1;
pub const EXIT_USAGE_ERROR: i32 = 2;

/// Run `adept check`, writing to `stdout`/`stderr`, and return the process
/// exit code.
pub fn run(args: &CheckArgs, config: &AdeptConfig, color: bool, quiet: bool) -> i32 {
    let mut lint_config = config.lint.clone();
    apply_select_ignore(&mut lint_config, &args.select, &args.ignore);

    let linter = Linter::new(lint_config);
    let mut all_diagnostics: Vec<Diagnostic> = Vec::new();
    let mut had_error = false;

    for path in &args.paths {
        if !path.exists() {
            eprintln!("adept: error: path not found: {}", path.display());
            had_error = true;
            continue;
        }
        match SkillSet::discover(path) {
            Ok(set) => all_diagnostics.extend(linter.lint_set(&set)),
            Err(err) => {
                eprintln!("adept: error: {err}");
                had_error = true;
            }
        }
    }

    if had_error {
        return EXIT_USAGE_ERROR;
    }

    all_diagnostics.sort_by(|a, b| {
        (&a.path, a.line, a.column, a.code).cmp(&(&b.path, b.line, b.column, b.code))
    });

    match args.format {
        OutputFormat::Human => {
            print!(
                "{}",
                adept::reporting::render_human_colored(&all_diagnostics, color)
            );
            if args.statistics {
                print_statistics(&all_diagnostics);
            }
            if !quiet {
                print_summary(&all_diagnostics);
            }
        }
        OutputFormat::Json => match adept::reporting::render_json(&all_diagnostics) {
            Ok(json) => println!("{json}"),
            Err(err) => {
                eprintln!("adept: error: failed to render JSON: {err}");
                return EXIT_USAGE_ERROR;
            }
        },
    }

    if args.exit_zero || all_diagnostics.is_empty() {
        EXIT_OK
    } else {
        EXIT_DIAGNOSTICS
    }
}

fn print_statistics(diagnostics: &[Diagnostic]) {
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for d in diagnostics {
        *counts.entry(d.code).or_insert(0) += 1;
    }
    let mut counts: Vec<(&'static str, usize)> = counts.into_iter().collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    if !counts.is_empty() {
        println!();
        println!("Statistics:");
        for (code, count) in counts {
            println!("  {code:<8} {count}");
        }
    }
}

fn print_summary(diagnostics: &[Diagnostic]) {
    println!();
    if diagnostics.is_empty() {
        println!("Found 0 problems");
    } else {
        let errors = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count();
        let warnings = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count();
        let infos = diagnostics.len() - errors - warnings;
        println!(
            "Found {} problems ({errors} error{}, {warnings} warning{}, {infos} info{})",
            diagnostics.len(),
            if errors == 1 { "" } else { "s" },
            if warnings == 1 { "" } else { "s" },
            if infos == 1 { "" } else { "s" },
        );
    }
}

/// Apply `--select`/`--ignore` on top of the config-file-derived
/// [`LintConfig::disabled`] set. `--select` (if non-empty) disables every
/// rule *except* those named; `--ignore` then additionally disables the
/// named rules.
fn apply_select_ignore(config: &mut LintConfig, select: &[String], ignore: &[String]) {
    if !select.is_empty() {
        let registry = Registry::new();
        let selected: std::collections::HashSet<&str> = select.iter().map(String::as_str).collect();
        for meta in registry.all_meta() {
            if !selected.contains(meta.code) && !selected.contains(meta.name) {
                config.disabled.insert(meta.code.to_string());
            }
        }
    }
    for code in ignore {
        config.disabled.insert(code.clone());
    }
}
