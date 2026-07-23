//! Discovering and parsing all skills under a path.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::AdeptError;
use crate::parser::{AnthropicSkillParser, SkillParser};
use crate::skill::Skill;

const SKILL_FILE_NAME: &str = "SKILL.md";
const EXCLUDED_DIR_NAMES: [&str; 2] = ["target", "node_modules"];

/// A collection of skills discovered by walking a directory tree.
///
/// `root` may be a single SKILL.md file, a single skill directory
/// (containing a SKILL.md), or a directory tree containing many skill
/// directories anywhere within it. Hidden directories (names starting with
/// `.`) and `target`/`node_modules` directories are skipped.
///
/// Skills that fail to parse are recorded in `errors` rather than causing
/// discovery as a whole to fail, so that e.g. `adept check` can still report
/// on the skills that *did* parse.
#[derive(Debug, Default)]
pub struct SkillSet {
    /// Successfully parsed skills.
    pub skills: Vec<Skill>,
    /// Paths that were found but failed to parse, paired with the error.
    pub errors: Vec<(PathBuf, AdeptError)>,
}

impl SkillSet {
    /// Discover and parse all skills under `root`, using the default
    /// [`AnthropicSkillParser`].
    pub fn discover(root: impl AsRef<Path>) -> Result<Self, AdeptError> {
        Self::discover_with_parser(root, &AnthropicSkillParser)
    }

    /// Discover and parse all skills under `root`, using a custom
    /// [`SkillParser`] (e.g. for a non-Anthropic skill format).
    pub fn discover_with_parser(
        root: impl AsRef<Path>,
        parser: &dyn SkillParser,
    ) -> Result<Self, AdeptError> {
        let root = root.as_ref();
        if !root.exists() {
            return Err(AdeptError::NotFound(root.to_path_buf()));
        }

        let mut skill_paths = Vec::new();
        if root.is_file() {
            skill_paths.push(root.to_path_buf());
        } else {
            for entry in WalkDir::new(root)
                .into_iter()
                .filter_entry(|e| !is_excluded(e))
            {
                let entry = entry?;
                if entry.file_type().is_file() && entry.file_name() == SKILL_FILE_NAME {
                    skill_paths.push(entry.into_path());
                }
            }
        }

        let mut skills = Vec::new();
        let mut errors = Vec::new();
        for path in skill_paths {
            match parser.parse(&path) {
                Ok(skill) => skills.push(skill),
                Err(err) => errors.push((path, err)),
            }
        }

        Ok(SkillSet { skills, errors })
    }
}

/// The skill's own directory, given a path to its `SKILL.md` file or the
/// directory itself. Falls back to `.` when a file path has no parent.
///
/// The file-or-directory half of [`sibling_root`]; kept as its own function
/// so that rule reads as "the parent of the skill's own directory".
fn skill_directory(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

/// The root under which a skill's *siblings* live, given a path to its
/// `SKILL.md` file or the skill's own directory: the parent of
/// [`skill_directory`], falling back to the skill's own directory when it
/// has no parent.
///
/// `discover` walks recursively, so searching the skill's own directory
/// would only ever re-find the skill itself; siblings live one level up, in
/// the standard `<root>/<skill-name>/SKILL.md` layout.
pub fn sibling_root(path: &Path) -> PathBuf {
    let skill_dir = skill_directory(path);
    skill_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(skill_dir)
}

fn is_excluded(entry: &walkdir::DirEntry) -> bool {
    // Never exclude the root itself, even if its name would otherwise match.
    if entry.depth() == 0 {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    if name.starts_with('.') {
        return true;
    }
    entry.file_type().is_dir() && EXCLUDED_DIR_NAMES.contains(&name.as_ref())
}
