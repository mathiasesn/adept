//! `adept fmt`.

use adept::SkillSet;
use adept_fmt::{check_skill, format_skill, FmtConfig};

use crate::cli::FmtArgs;
use crate::config::AdeptConfig;

pub const EXIT_OK: i32 = 0;
pub const EXIT_UNFORMATTED: i32 = 1;
pub const EXIT_USAGE_ERROR: i32 = 2;

/// Run `adept fmt`, writing to `stdout`/`stderr`, and return the process
/// exit code.
pub fn run(args: &FmtArgs, config: &AdeptConfig, quiet: bool) -> i32 {
    let mut fmt_config = config.fmt.clone();
    if let Some(width) = args.line_width {
        fmt_config.line_width = width;
    }

    let mut reformatted = 0usize;
    let mut unchanged = 0usize;
    let mut any_would_change = false;
    let mut had_error = false;

    for path in &args.paths {
        if !path.exists() {
            eprintln!("adept: error: path not found: {}", path.display());
            had_error = true;
            continue;
        }
        let set = match SkillSet::discover(path) {
            Ok(set) => set,
            Err(err) => {
                eprintln!("adept: error: {err}");
                had_error = true;
                continue;
            }
        };
        for (err_path, err) in &set.errors {
            eprintln!("adept: error: {}: {err}", err_path.display());
            had_error = true;
        }
        for skill in &set.skills {
            match handle_skill(skill, &fmt_config, args) {
                Ok(true) => {
                    reformatted += 1;
                    any_would_change = true;
                }
                Ok(false) => unchanged += 1,
                Err(err) => {
                    eprintln!("adept: error: {}: {err}", skill.path.display());
                    had_error = true;
                }
            }
        }
    }

    if had_error {
        return EXIT_USAGE_ERROR;
    }

    if !(quiet || args.check || args.diff) {
        println!(
            "{reformatted} file{} reformatted, {unchanged} file{} unchanged",
            if reformatted == 1 { "" } else { "s" },
            if unchanged == 1 { "" } else { "s" },
        );
    }

    if (args.check || args.diff) && any_would_change {
        EXIT_UNFORMATTED
    } else {
        EXIT_OK
    }
}

/// An error handling a single skill during `adept fmt`: either a
/// formatting error (malformed frontmatter) or an I/O error writing the
/// result back out.
#[derive(Debug, thiserror::Error)]
enum HandleError {
    #[error("{0}")]
    Fmt(#[from] adept_fmt::FmtError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Handle a single skill: returns `Ok(true)` if it was (or would be)
/// reformatted, `Ok(false)` if it was already formatted.
fn handle_skill(
    skill: &adept::Skill,
    config: &FmtConfig,
    args: &FmtArgs,
) -> Result<bool, HandleError> {
    let result = check_skill(skill, config)?;
    if result.formatted {
        return Ok(false);
    }

    if args.check || args.diff {
        print!("{}", result.diff);
        return Ok(true);
    }

    let formatted = format_skill(skill, config)?;
    adept_agent::write_atomically(&skill.path, &formatted)?;
    Ok(true)
}
