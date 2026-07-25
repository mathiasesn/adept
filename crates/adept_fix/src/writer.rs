//! Transactional multi-file write: either every file in a batch lands, or
//! none of them do.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Write every `(path, contents)` pair in `files`, transactionally:
///
/// 1. Write each to a sibling temp file (`.{filename}.adept-fix-tmp`, in the
///    same directory as its target), `write_all` then `sync_all`. This
///    mirrors `adept_cli`'s `write_atomically` (`crates/adept_cli/src/commands/fmt.rs`),
///    extended to a whole batch.
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
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let tmp_path = dir.join(format!(
            ".{}.adept-fix-tmp",
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "SKILL.md".to_string())
        ));

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

    fn tempdir(tag: &str) -> PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let dir = std::env::temp_dir().join(format!(
            "adept_fix_writer_test_{tag}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_all_files_on_success() {
        let dir = tempdir("success");
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
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("adept-fix-tmp"))
            .collect();
        assert!(leftovers.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn failure_leaves_originals_untouched_and_no_temp_files() {
        let dir = tempdir("failure");
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
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("adept-fix-tmp"))
            .collect();
        assert!(leftovers.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }
}
