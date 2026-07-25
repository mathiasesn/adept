//! Atomic single-file write, and a transactional multi-file write built on
//! top of it: either every file in a batch lands, or none of them do.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The shared temp-file suffix used by both [`write_atomically`] and
/// [`write_all_transactionally`], so there is exactly one convention for
/// what an in-progress `adept` write looks like on disk.
const TMP_SUFFIX: &str = "adept-tmp";

/// Write `contents` to `path` atomically: write to a sibling temp file
/// (`.{filename}.adept-tmp`, in the same directory as `path`), `write_all`
/// then `sync_all`, then rename over `path`. Never leaves `path` clobbered
/// if the write fails partway through.
///
/// # Errors
/// Returns the first I/O error encountered creating, writing, syncing, or
/// renaming the temp file.
pub fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp_path = tmp_path_for(path);
    {
        let mut tmp_file = std::fs::File::create(&tmp_path)?;
        tmp_file.write_all(contents.as_bytes())?;
        tmp_file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)
}

/// The sibling temp path a write to `path` stages through.
fn tmp_path_for(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!(
        ".{}.{TMP_SUFFIX}",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "SKILL.md".to_string())
    ))
}

/// Write every `(path, contents)` pair in `files`, transactionally:
///
/// 1. Write each to its sibling temp file via the same staging step
///    [`write_atomically`] uses (create, `write_all`, `sync_all`), but
///    without yet renaming.
/// 2. If any temp write fails, unlink every temp file already created (best
///    effort) and return the error — no original file is touched.
/// 3. Only once every temp write has succeeded, rename each temp file into
///    place over its target.
///
/// # Atomicity caveat
/// Step 3 is atomic *per file* (each `rename` is atomic on the same
/// filesystem), but not atomic *across* files: a crash or power loss
/// between the first and last rename can leave the batch partially applied.
/// This is a known, accepted limitation (there is no cross-file transaction
/// log here) — the mitigation is that all fallible work (steps 1-2) happens
/// before any rename, so the only failure mode left is an external crash
/// during a short, all-renames window, not a normal I/O error partially
/// applying the batch.
///
/// # Errors
/// Returns the first I/O error encountered while creating/writing/syncing a
/// temp file. Errors during rename (step 3) are also propagated, though by
/// that point some files in the batch may already have been renamed.
pub fn write_all_transactionally(files: &BTreeMap<PathBuf, String>) -> std::io::Result<()> {
    let mut tmp_paths: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(files.len());

    for (path, contents) in files {
        let tmp_path = tmp_path_for(path);

        let write_result = (|| -> std::io::Result<()> {
            let mut tmp_file = std::fs::File::create(&tmp_path)?;
            tmp_file.write_all(contents.as_bytes())?;
            tmp_file.sync_all()
        })();

        if let Err(err) = write_result {
            for (_, tmp) in &tmp_paths {
                let _ = std::fs::remove_file(tmp);
            }
            let _ = std::fs::remove_file(&tmp_path);
            return Err(err);
        }

        tmp_paths.push((path.clone(), tmp_path));
    }

    for (path, tmp_path) in &tmp_paths {
        std::fs::rename(tmp_path, path)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_all_files_on_success() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let a = dir.join("SKILL.md");
        let b = dir.join("REFERENCE.md");
        let files = BTreeMap::from([
            (a.clone(), "new skill content".to_string()),
            (b.clone(), "new reference content".to_string()),
        ]);

        write_all_transactionally(&files).unwrap();

        assert_eq!(std::fs::read_to_string(&a).unwrap(), "new skill content");
        assert_eq!(
            std::fs::read_to_string(&b).unwrap(),
            "new reference content"
        );
        // No leftover temp files.
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("adept-tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn failure_leaves_originals_untouched_and_no_temp_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let a = dir.join("SKILL.md");
        std::fs::write(&a, "original content").unwrap();

        // A target whose parent directory does not exist: `File::create`
        // on its temp file fails, simulating a write failure partway
        // through the batch.
        let bad_path = dir.join("missing_subdir").join("REFERENCE.md");

        let files = BTreeMap::from([
            (a.clone(), "new skill content".to_string()),
            (bad_path.clone(), "new reference content".to_string()),
        ]);

        let result = write_all_transactionally(&files);
        assert!(result.is_err());

        // Original SKILL.md is untouched.
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "original content");
        // No leftover temp files anywhere in the directory tree.
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("adept-tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }
}
