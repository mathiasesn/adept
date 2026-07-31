//! Pins the "adept spawns no subprocess, ever" invariant (`docs/ARCHI.md`
//! Invariants list) the same way `tests/tracing.rs` pins MCP stdout purity:
//! a source-level scan that fails the build if a future change introduces
//! subprocess spawning anywhere in `create` (or anywhere else in the
//! workspace), rather than relying on nobody noticing in review.
//!
//! `command`-kind eval assertions are defined and validated by `adept`'s
//! evals machinery, but per the spec adept itself never *executes* them —
//! that is left to whatever harness consumes the generated dataset. This
//! test proves the shipped source contains no `std::process::Command`
//! construction (the only way to spawn a subprocess in Rust) anywhere in
//! the workspace's own crates.
//!
//! `std::process::exit` (used legitimately in `adept_cli::main` for exit
//! codes, a documented public contract) is a different, unrelated call and
//! is not matched by the patterns scanned for here, so no allowlisting is
//! needed for it.

use std::path::Path;

/// Strip line comments, block comments, and string/char literal contents
/// from a line of Rust source, so a comment or a string literal mentioning
/// "Command::new" (such as this very test file) can never produce a false
/// positive or, symmetrically, hide a real one behind a lookalike string.
///
/// This is a small hand-rolled scanner, not a full Rust lexer; it is
/// sufficient for detecting the specific token sequences this test looks
/// for and is not used for anything else.
fn strip_comments_and_strings(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_block_comment_depth: u32 = 0;

    while let Some(c) = chars.next() {
        if in_block_comment_depth > 0 {
            if c == '/' && chars.peek() == Some(&'*') {
                chars.next();
                in_block_comment_depth += 1;
            } else if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment_depth -= 1;
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            // Line comment: drop the rest of *this line* only, then keep
            // scanning subsequent lines.
            for sc in chars.by_ref() {
                if sc == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            in_block_comment_depth = 1;
            continue;
        }
        if c == '"' {
            // String literal (including raw-ish `r"..."`/`r#"..."#` handled
            // approximately: we just skip to the next unescaped `"`, which
            // is conservative enough here since we only care about
            // stripping content, not perfectly re-lexing it).
            out.push(' ');
            while let Some(sc) = chars.next() {
                if sc == '\\' {
                    chars.next();
                    continue;
                }
                if sc == '"' {
                    break;
                }
            }
            continue;
        }
        if c == '\'' {
            // Could be a char literal or a lifetime; either way, none of
            // our search patterns can appear inside one, so just copy it
            // through unchanged rather than trying to distinguish them.
            out.push(c);
            continue;
        }
        out.push(c);
    }
    out
}

/// Returns every `.rs` file under `dir`, recursively, skipping `target/`
/// and any `tests/` directory.
///
/// Test suites are deliberately excluded from the scan: this repo's CLI
/// integration tests (`crates/adept_cli/tests/common/mod.rs`) legitimately
/// spawn the *built `adept` binary itself* via `assert_cmd`/
/// `std::process::Command` to drive it as a black box over stdio — that is
/// test infrastructure exercising the shipped binary from the outside, not
/// the shipped binary spawning anything itself. The invariant this test
/// pins is about `adept`'s own source, so only non-test crate source is
/// scanned.
fn rust_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str());
            if name == Some("target") || name == Some("tests") {
                continue;
            }
            files.extend(rust_files(&path));
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    files
}

#[test]
fn no_crate_source_spawns_a_subprocess() {
    let workspace_crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../")
        .canonicalize()
        .expect("crates/ directory should exist");

    let mut offenders = Vec::new();
    for file in rust_files(&workspace_crates) {
        // Exclude this test file itself: it legitimately mentions the
        // pattern in comments and doc-comments, both of which are already
        // stripped above, but excluding it too is belt-and-braces against
        // this file being renamed/refactored in a way that confuses the
        // stripper.
        if file
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == "no_subprocess.rs")
        {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&file) else {
            continue;
        };
        let stripped = strip_comments_and_strings(&contents);
        if stripped.contains("Command::new") || stripped.contains("process::Command") {
            offenders.push(file);
        }
    }

    assert!(
        offenders.is_empty(),
        "adept spawns no subprocess, ever (docs/ARCHI.md invariant); found subprocess \
         construction in: {offenders:?}"
    );
}

#[test]
fn scanner_detects_a_real_construction_when_present() {
    // Pins the scanner's own ability to fire, so a future edit that
    // accidentally makes `strip_comments_and_strings` swallow real code
    // (not just comments/strings) can't silently defang the test above.
    let source = r#"
        // Command::new("ls") in a comment must not count.
        let s = "Command::new(\"ls\")"; // nor in a string literal
        let cmd = std::process::Command::new("ls");
    "#;
    let stripped = strip_comments_and_strings(source);
    assert!(!stripped.contains("Command::new(\"ls\")"));
    assert!(stripped.contains("process::Command::new"));
}
