//! Unified diff rendering across every file a fix candidate touches.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Render a single unified diff spanning every file in `updated` that
/// differs from its counterpart in `originals` (missing from `originals`
/// counts as an empty original, so newly created companion files render as
/// pure additions).
///
/// Files are rendered in `updated`'s (`BTreeMap`, so path-sorted) order;
/// files identical between `originals` and `updated` are skipped entirely.
#[must_use]
pub fn render_multi_file_diff(
    originals: &BTreeMap<PathBuf, String>,
    updated: &BTreeMap<PathBuf, String>,
) -> String {
    let mut out = String::new();
    for (path, new_content) in updated {
        let old_content = originals.get(path).map(String::as_str).unwrap_or("");
        if old_content == new_content {
            continue;
        }
        let label = path.display().to_string();
        let file_diff = similar::TextDiff::from_lines(old_content, new_content.as_str())
            .unified_diff()
            .header(&label, &label)
            .to_string();
        out.push_str(&file_diff);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_unchanged_files() {
        let originals = BTreeMap::from([(PathBuf::from("a"), "same\n".to_string())]);
        let updated = BTreeMap::from([(PathBuf::from("a"), "same\n".to_string())]);
        assert_eq!(render_multi_file_diff(&originals, &updated), "");
    }

    #[test]
    fn renders_new_file_as_addition() {
        let originals = BTreeMap::new();
        let updated = BTreeMap::from([(PathBuf::from("new.md"), "hello\n".to_string())]);
        let diff = render_multi_file_diff(&originals, &updated);
        assert!(diff.contains("new.md"));
        assert!(diff.contains("+hello"));
    }

    #[test]
    fn renders_changed_file() {
        let originals = BTreeMap::from([(PathBuf::from("a"), "old\n".to_string())]);
        let updated = BTreeMap::from([(PathBuf::from("a"), "new\n".to_string())]);
        let diff = render_multi_file_diff(&originals, &updated);
        assert!(diff.contains("-old"));
        assert!(diff.contains("+new"));
    }
}
