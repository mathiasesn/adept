//! Lints the vendored `anthropics/skills` corpus (see
//! `tests/fixtures/corpus/README.md` for provenance) and snapshots the
//! rendered diagnostics.
//!
//! The corpus is real, third-party skill content, so this is a broad-input
//! regression net rather than a rule-behaviour spec: the snapshot enshrines
//! whatever diagnostics the current ruleset produces against real skills. A
//! diff here means *something* in the lexer, a rule, or the renderer changed
//! behaviour — it does not by itself mean the corpus skills got worse or
//! better. A shrinking diagnostic count is a win, not a regression to chase
//! back to zero.

use std::path::{Path, PathBuf};

use adept::reporting::render_human_colored;
use adept::{LintConfig, Linter, SkillSet};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corpus")
}

/// Rewrites `path` to be relative to `root`, with forward slashes, so the
/// rendered diagnostic is machine-independent. `render_human_colored` embeds
/// `Diagnostic::path` verbatim via `Path::display`, and `SkillSet::discover`
/// records the absolute path it was given, so without this rewrite the
/// snapshot would contain this machine's absolute path.
fn relativize(path: &Path, root: &Path) -> PathBuf {
    let rel = path.strip_prefix(root).unwrap_or_else(|_| {
        panic!(
            "{} should be under corpus root {}",
            path.display(),
            root.display()
        )
    });
    // Force forward slashes regardless of host OS, so the snapshot is
    // identical on Windows and Unix.
    let joined = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    PathBuf::from(joined)
}

#[test]
fn corpus_lint_snapshot() {
    let root = corpus_dir();
    let set = SkillSet::discover(&root).expect("corpus should discover");
    assert!(
        !set.skills.is_empty(),
        "expected the vendored corpus to contain skills"
    );
    assert!(
        set.errors.is_empty(),
        "expected every corpus skill to parse, got errors:\n{:#?}",
        set.errors
    );

    let linter = Linter::new(LintConfig::default()).expect("default tokenizer should load");
    let mut diagnostics = linter.lint_set(&set);
    for d in &mut diagnostics {
        d.path = relativize(&d.path, &root);
    }

    let rendered = render_human_colored(&diagnostics, false);
    insta::assert_snapshot!(rendered);
}
