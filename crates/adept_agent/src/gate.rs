//! Shared accept/reject comparison machinery, used by both [`crate::fix`]
//! (which compares a candidate's diagnostics against the *previous* round's,
//! for an existing skill) and [`crate::create`] (which has no "before" — it
//! judges a candidate against a fixed severity gate, and separately tracks
//! the best candidate seen across rounds).
//!
//! Lifted out of `fix`'s own loop so the comparison is defined once; `fix`'s
//! behaviour is unchanged by this move (see `fix`'s own regression tests).

use adept::{Diagnostic, Severity};

/// Per-severity tally of a diagnostic set, used to compare two rounds
/// without materialising a combined `Vec<Diagnostic>` (e.g. `create`'s
/// repair loop, which would otherwise build one on every round purely to
/// compare counts).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    pub total: usize,
}

impl Counts {
    /// Tally a full diagnostic slice in one pass.
    #[must_use]
    pub fn of(diagnostics: &[Diagnostic]) -> Self {
        let mut counts = Self::default();
        for d in diagnostics {
            counts.add(d.severity);
        }
        counts
    }

    /// Fold one more diagnostic's severity into the tally.
    pub fn add(&mut self, severity: Severity) {
        match severity {
            Severity::Error => self.errors += 1,
            Severity::Warning => self.warnings += 1,
            Severity::Info => self.infos += 1,
        }
        self.total += 1;
    }
}

/// Whether `candidate`'s diagnostics are a strict improvement over
/// `current`'s, severity-aware: `(errors, warnings, infos)` compared
/// lexicographically (fewer errors wins outright regardless of warning/info
/// counts; ties on errors break on warnings, then infos), falling back to
/// total diagnostic count when all three components are equal. Equal on
/// every component is not an improvement.
///
/// This is `fix`'s round-over-round acceptance rule. `create` reuses it too
/// (via [`improves_on_counts`]), to decide whether a new round's candidate
/// is strictly better than the best one seen so far (its analogue of
/// "current" when there is no pre-existing "before").
#[must_use]
pub fn improves_on(current: &[Diagnostic], candidate: &[Diagnostic]) -> bool {
    improves_on_counts(Counts::of(current), Counts::of(candidate))
}

/// [`Counts`]-only form of [`improves_on`], for a caller that has already
/// tallied severities and wants to avoid re-walking (or building) the
/// underlying diagnostic slices.
#[must_use]
pub fn improves_on_counts(current: Counts, candidate: Counts) -> bool {
    if candidate.errors != current.errors {
        return candidate.errors < current.errors;
    }
    if candidate.warnings != current.warnings {
        return candidate.warnings < current.warnings;
    }
    if candidate.infos != current.infos {
        return candidate.infos < current.infos;
    }
    candidate.total < current.total
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

    /// Severity-aware semantics: fewer errors wins outright even at the
    /// cost of more lower-severity findings, and trading warnings for an
    /// error is rejected even though the raw count shrinks. Equal severity
    /// profiles fall back to comparing total count.
    #[test]
    fn improves_on_requires_strictly_fewer() {
        // One error resolved into two infos: still an improvement, even
        // though the candidate has more diagnostics in total.
        assert!(improves_on(
            &[diag(Severity::Error)],
            &[diag(Severity::Info), diag(Severity::Info)]
        ));
        // Two warnings traded for one error: rejected, despite the lower
        // raw count.
        assert!(!improves_on(
            &[diag(Severity::Warning), diag(Severity::Warning)],
            &[diag(Severity::Error)]
        ));
        // Equal severity profiles (zero of everything vs zero of
        // everything) fall back to total count and are not an improvement.
        assert!(!improves_on(&[], &[]));
        assert!(!improves_on(
            &[diag(Severity::Error)],
            &[diag(Severity::Error)]
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
