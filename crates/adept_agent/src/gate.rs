//! Shared accept/reject comparison machinery, used by both [`crate::fix`]
//! (which compares a candidate's diagnostics against the *previous* round's,
//! for an existing skill) and [`crate::create`] (which has no "before" — it
//! judges a candidate against a fixed severity gate, and separately tracks
//! the best candidate seen across rounds).
//!
//! Lifted out of `fix`'s own loop so the comparison is defined once; `fix`'s
//! behaviour is unchanged by this move (see `fix`'s own regression tests).

use adept::{Diagnostic, Severity};

/// Whether `candidate`'s diagnostics are a strict improvement over
/// `current`'s: strictly fewer diagnostics remain, by count.
///
/// This is `fix`'s original round-over-round acceptance rule. `create` reuses
/// it too, to decide whether a new round's candidate is strictly better than
/// the best one seen so far (its analogue of "current" when there is no
/// pre-existing "before").
#[must_use]
pub fn improves_on(current: &[Diagnostic], candidate: &[Diagnostic]) -> bool {
    improves_on_len(current.len(), candidate.len())
}

/// Length-only form of [`improves_on`], for a caller that only needs the
/// counts (e.g. `create`'s repair loop, which would otherwise build a full
/// combined `Vec<Diagnostic>` on every round purely to compare lengths).
#[must_use]
pub fn improves_on_len(current_len: usize, candidate_len: usize) -> bool {
    candidate_len < current_len
}

/// Whether `diagnostics` clears `create`'s repair-loop gate: zero `Error`
/// and zero `Warning` findings. `Info` findings never block — pinned by a
/// dedicated test so the threshold cannot silently tighten.
///
/// Severity here is whatever the effective [`adept::LintConfig`] already
/// resolved it to; this function does not know about rule codes or
/// configuration, only the sorted-out `Severity` on each diagnostic.
#[must_use]
pub fn passes_severity_gate(diagnostics: &[Diagnostic]) -> bool {
    diagnostics
        .iter()
        .all(|d| !matches!(d.severity, Severity::Error | Severity::Warning))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn diag(severity: Severity) -> Diagnostic {
        Diagnostic::new("SL999", "test", severity, PathBuf::from("SKILL.md"), 1, 1)
    }

    #[test]
    fn improves_on_requires_strictly_fewer() {
        assert!(improves_on(
            &[diag(Severity::Error), diag(Severity::Error)],
            &[diag(Severity::Error)]
        ));
        assert!(!improves_on(
            &[diag(Severity::Error)],
            &[diag(Severity::Error)]
        ));
        assert!(!improves_on(
            &[diag(Severity::Error)],
            &[diag(Severity::Error), diag(Severity::Error)]
        ));
    }

    #[test]
    fn severity_gate_blocks_error_and_warning_but_not_info() {
        assert!(passes_severity_gate(&[]));
        assert!(passes_severity_gate(&[diag(Severity::Info)]));
        assert!(!passes_severity_gate(&[diag(Severity::Warning)]));
        assert!(!passes_severity_gate(&[diag(Severity::Error)]));
    }
}
