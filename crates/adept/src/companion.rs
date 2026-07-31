//! Shared companion-file discovery: finding the non-`SKILL.md` files that
//! live alongside a skill.
//!
//! Used by [`crate::rules::tokens::CompanionFileBloat`] (`SL303`) and by
//! `adept_score`'s token-bloat analysis, which previously each implemented
//! their own (subtly different) version of this walk. Both callers want
//! the same set of files, so this is the single shared implementation;
//! callers still apply their own thresholds/analysis on top.

use std::path::PathBuf;

use crate::skill::Skill;

/// Discover companion files: every regular file in `skill`'s directory
/// other than `SKILL.md` itself. Non-recursive, since companion files live
/// alongside SKILL.md by convention, not in subdirectories.
///
/// Returns an empty, sorted `Vec` if the skill's directory cannot be read
/// (e.g. the skill was parsed from a path with no accessible parent); this
/// is a soft degradation, not a hard error, since the callers that use this
/// (token-bloat rules/analysis) still have something meaningful to report
/// without companion files. Sorted by path for deterministic output.
#[must_use]
pub fn discover_companion_files(skill: &Skill) -> Vec<PathBuf> {
    let Some(dir) = skill.path.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path != &skill.path)
        .collect();
    files.sort();
    files
}

/// Returns true if `name` (a bare file name, not a path) is a recognized
/// license file: `LICENSE`, `LICENCE`, `COPYING`, `COPYRIGHT` (any
/// extension), or a name whose stem starts with `LICENSE-` / `LICENCE-`
/// (e.g. `LICENSE-APACHE`). Matching is case-insensitive on the stem.
///
/// Lives here beside [`discover_companion_files`] because recognizing a
/// license file is a companion-file naming concern; callers decide what to
/// do with the classification. `SL303` uses it to exempt bundled license
/// boilerplate from its token-budget check.
#[must_use]
pub(crate) fn is_license_file(name: &str) -> bool {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let stem = stem.to_ascii_lowercase();
    matches!(
        stem.as_str(),
        "license" | "licence" | "copying" | "copyright"
    ) || stem.starts_with("license-")
        || stem.starts_with("licence-")
}

/// Returns true if `path` (a companion file's path, as discovered by
/// [`discover_companion_files`]) sits under a top-level `evals/` directory
/// within the skill directory.
///
/// Matches **by directory name only** — no filename pattern, and explicitly
/// no content sniffing. `adept create` writes a synthetic eval dataset to
/// `evals/evals.jsonl`, and parsing candidate files during discovery would
/// put I/O and JSON parsing on the fast, offline `check` path, which must
/// stay fast. A dataset a user keeps somewhere else counts as ordinary skill
/// content, since adept did not put it there.
///
/// Lives here beside [`is_license_file`] for the same reason: recognizing an
/// eval dataset is a companion-file naming concern; callers decide what to
/// do with the classification.
///
/// **Note on today's behavior:** [`discover_companion_files`] is
/// non-recursive, so a file under a nested `evals/` directory is never
/// discovered in the first place — this predicate can never fire against
/// current output of that function, and applying it is a no-op today. It is
/// kept as cheap defence-in-depth: if discovery ever becomes recursive (e.g.
/// to support `adept create`'s `evals/evals.jsonl`), the exemption is
/// already wired into its two applicable consumers (`SL303` in
/// `rules/tokens.rs`, and `adept_score`'s token-bloat view) rather than
/// needing to be added later, possibly incompletely, across every
/// consumer — including ones (like `adept_agent`'s fix conservation guard)
/// that must never see it change.
#[must_use]
pub fn is_eval_dataset(path: &std::path::Path) -> bool {
    path.components().any(|c| c.as_os_str() == "evals")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_skill;
    use std::io::Write;

    #[test]
    fn is_license_file_matches_bare_license_names() {
        assert!(is_license_file("LICENSE"));
        assert!(is_license_file("LICENSE.txt"));
        assert!(is_license_file("license.md"));
        assert!(is_license_file("COPYING"));
        assert!(is_license_file("LICENSE-APACHE"));
    }

    #[test]
    fn is_license_file_rejects_lookalikes() {
        assert!(!is_license_file("reference.md"));
        assert!(!is_license_file("licenses.md"));
        assert!(!is_license_file("my-license-guide.md"));
    }

    fn write_skill(dir: &std::path::Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("SKILL.md");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            "---\nname: demo\ndescription: A demo skill for tests\n---\nBody."
        )
        .unwrap();
        path
    }

    #[test]
    fn discovers_companion_files_sorted_excluding_skill_md() {
        let dir =
            std::env::temp_dir().join(format!("adept_companion_test_{}_{}", std::process::id(), {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            }));
        let skill_path = write_skill(&dir);
        std::fs::write(dir.join("b.md"), "b").unwrap();
        std::fs::write(dir.join("a.md"), "a").unwrap();

        let skill = parse_skill(&skill_path).unwrap();
        let files = discover_companion_files(&skill);
        let names: Vec<_> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.md", "b.md"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_directory_returns_empty() {
        let skill = crate::Skill {
            path: PathBuf::from("/nonexistent/does/not/exist/SKILL.md"),
            frontmatter: crate::Frontmatter {
                name: "x".into(),
                name_line: 1,
                description: "x".into(),
                description_line: 1,
                license: None,
                license_line: None,
                extra: Default::default(),
            },
            body: String::new(),
            body_line_offset: 1,
            source: String::new(),
        };
        assert!(discover_companion_files(&skill).is_empty());
    }
}
