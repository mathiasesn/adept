# Fix review findings from the corpus-fixture change

## Problem / Why
The two-axis review of the vendored-skills-corpus work (`51093d4..HEAD`,
commits `6ab41ec`..`34f44d4`) found **no hard violations and no spec gaps**.
It surfaced three Standards judgement calls worth cleaning up before this
settles into the codebase. None is a defect; all are about matching repo
convention and reducing future-reader friction.

1. **`print.rs:308-315` — double-dispatch with an `unreachable!()` arm.**
   `build_tokens` tests `item` twice: an `if let Inline::Text(s)` guard, then a
   `match item` whose first arm is `Inline::Text(_) => unreachable!(...)`. The
   surrounding formatter code favours a single exhaustive `match`. The
   `unreachable!()` arm is dead weight a reader has to reason about.

2. **`format_tests.rs` — the repo's first `#[ignore]`d tests.** `git grep
   '#[ignore'` at the base commit returns nothing. The two `#[ignore]`d
   minimized repros (`wrapped_line_starting_with_dash_...`,
   `wrapped_line_starting_with_plus_...`) introduce committed red tests as a new
   pattern. The established convention for known-broken behaviour here is a
   `docs/BACKLOG.md` entry (which the change also made) plus the
   `KNOWN_NON_IDEMPOTENT` const. This is a precedent decision, not a bug.

3. **`corpus_dir()` duplicated across crates.** `crates/adept/tests/corpus.rs`
   and `crates/adept_fmt/tests/format_tests.rs` each define their own
   `corpus_dir()`. The reviewer judged this within bounds (test-path helpers,
   not library code; the fixture genuinely lives in `adept`). Listed for
   completeness; likely no action.

## Goals
- Collapse the `build_tokens` double-match into one exhaustive `match`, removing
  the `unreachable!()` arm while preserving the text-run coalescing behaviour
  that fixes escaped-word idempotency.
- Resolve the `#[ignore]`d-test precedent one way or the other, consistently.
- Keep the full workspace green and clean: `cargo test --workspace`, `clippy
  --all-targets -- -D warnings`, `cargo fmt --all -- --check`.

## Non-goals
- Fixing the leaning-toothpick reflow bug itself (the deferred BACKLOG item).
  This task is about the *review findings on the already-landed change*, not new
  formatter work.
- Re-vendoring, changing corpus composition, or touching the snapshot's expected
  diagnostics.
- Any behaviour change to `build_tokens` — finding #1 is a pure refactor; the
  before/after token output must be identical.

## Constraints
- Rust 2021, workspace `rust-version = "1.85"`.
- Finding #1 must not alter formatter output: the corpus idempotency test and
  the existing `format_tests.rs` snapshot/idempotency suite are the guard.
- Whatever is decided for finding #2, the property must still be *guarded*: the
  8 passing corpus skills stay under the live idempotency assertion, and the two
  failures stay recorded (BACKLOG + `KNOWN_NON_IDEMPOTENT`) so they are not lost.
- Convention source: `docs/ARCHI.md` §13/§14 (known-issues go to BACKLOG),
  §15 invariants, and the existing test idioms in `tests/rules.rs`.

## Proposed approach

### Finding #1 — collapse the match (decided: fix)
Rewrite the loop in `build_tokens` so `Inline::Text` is a normal match arm that
appends to `text_run`, with the non-text arms preceded by a `flush_text_run`.
One clean way: `match item { Inline::Text(s) => text_run.push_str(s), other =>
{ flush_text_run(..); /* handle other */ } }` — or hoist the flush so every
non-text arm is reached only after flushing. The `unreachable!()` arm
disappears. Verify token-identical output via the existing tests.

### Finding #2 — the `#[ignore]`d tests (decided: keep)
Keep the two `#[ignore]`d regression tests. Committed-red repros are accepted as
a repo pattern here: they live as runnable code, cross-linked to
`KNOWN_NON_IDEMPOTENT` and the BACKLOG entry, and can be un-ignored the moment
the leaning-toothpick reflow bug is fixed. No code change for this finding —
the existing tests already embody the decision; this task only ratifies it.
One small follow-through: make sure each `#[ignore]` attribute carries a reason
string (`#[ignore = "..."]`) pointing at the BACKLOG item, if it does not
already, so the pattern is self-documenting.

### Finding #3 — duplicated `corpus_dir()` (decided: leave as-is)
No shared test-helper crate exists, and the two helpers differ (one is
in-crate, one reaches cross-crate via `../adept/...`). Extracting a shared
helper would create a new cross-crate test dependency for marginal gain —
worse than the duplication per ARCHI §4/§5's "shared behaviour moves *down*
only when it's library behaviour." Leave both; no change.

## Acceptance criteria
- `build_tokens` contains a single `match` over `Inline` with no `unreachable!()`
  arm, and the text-run coalescing is preserved.
- `cargo test --workspace` passes (155+ passing), and the corpus + formatter
  idempotency tests still pass — proving finding #1 changed no output.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
  are clean.
- The two `#[ignore]`d reflow repros are kept, and each `#[ignore]` carries a
  reason string referencing the BACKLOG item.
- No change to `crates/adept/tests/fixtures/corpus/`, the corpus snapshot's
  expected diagnostics, or corpus composition.

## Open questions
*(none — resolved during planning)*

## Risks
- **Refactor silently changes output.** The `flush` ordering in `build_tokens`
  is subtle — a misplaced flush could drop or reorder tokens. Mitigated by the
  idempotency + snapshot suite, which would diff immediately.
- **The kept `#[ignore]`d tests are silent unless someone runs `--ignored`.** A
  future fix to the reflow bug could pass without anyone un-ignoring them. This
  is inherent to the pattern and accepted; the BACKLOG entry is the reminder.
