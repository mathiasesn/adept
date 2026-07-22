//! `--check`-mode support: detecting whether input is already formatted and
//! producing a unified diff of what would change.

/// The result of checking whether a document is already formatted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    /// `true` if the input was already in canonical formatted form (byte
    /// for byte).
    pub formatted: bool,
    /// A unified diff from the original input to the formatted output.
    /// Empty when `formatted` is `true`.
    pub diff: String,
}

/// Build a [`CheckResult`] comparing `original` against `formatted`.
pub fn check(original: &str, formatted: &str) -> CheckResult {
    if original == formatted {
        return CheckResult {
            formatted: true,
            diff: String::new(),
        };
    }
    let diff = similar::TextDiff::from_lines(original, formatted)
        .unified_diff()
        .header("original", "formatted")
        .to_string();
    CheckResult {
        formatted: false,
        diff,
    }
}
