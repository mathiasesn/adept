# Fix code review findings

## Problem / Why

A full-project review of `adept` @ `7c289ee` found one user-visible correctness bug, one
behaviour bug in the fix/create accept gate, two DRY violations (one of which defeats the
stated purpose of a module written specifically to prevent it), and four minor
doc/consistency issues. The codebase is otherwise clean (336 tests green, clippy and fmt
clean, zero debt markers), so these are worth closing out while the list is short.

## Goals

1. **`SL001`/`SL002`/`SL003` honour kebab-case names** in `disabled` and
   `severity_overrides`, matching `LintConfig::is_enabled` and the contract documented in
   `docs/ARCHI.md` §11. (`crates/adept/src/rules/mod.rs:451-464`)
2. **`improves_on` becomes severity-aware** so `adept fix` / `adept create` never accept a
   candidate that trades warnings for an error, nor reject one that resolves an error into
   several infos. (`crates/adept_agent/src/gate.rs:20-30`)
3. **`SL402`/`SL403` share one pairwise-similarity helper** instead of ~80 duplicated lines.
   (`crates/adept/src/rules/cross.rs`)
4. **`SL205` uses `adept::text::words`** instead of reimplementing it inline twice.
   (`crates/adept/src/rules/description.rs:167-181`)
5. **`SL205`'s `0.8` and `2` become named constants** consistent with sibling thresholds.
6. Minor: reconcile `pass_rate`'s doc vs. its denominator (`crates/adept/src/evals.rs:424`)
   by **correcting the wording to "results"** — the denominator is left alone, since
   changing it would move numbers users may already be tracking. Inline the pass-through
   `parse_jsonl_with_lines`. Give `SL204` the empty-description early return its siblings
   have. Document MCP `check_skill`'s use of `LintConfig::default()` in ARCHI §12 as
   **intended** behaviour — the CLI stays the only config-aware entry point — and pin it
   with a test so it cannot drift silently.

## Non-goals

- Any item already argued as a deliberate deferral in `BACKLOG.md`: the triplicated
  `*FileConfig` structs, `LlmError::Status`'s unscrubbed body, `adept fix` cross-file
  atomicity, O(n²) `SL4xx` cost, prose-reflow test coverage.
- Extending prose-reflow proptest coverage.
- Any new rule codes; no rule code is retired or reused.

## Constraints

- All four CI commands must stay green: `cargo build --workspace --all-targets`,
  `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`. Perf gate stays under 500ms.
- Dependency direction is fixed: nothing in the library stack may depend on `adept_agent`.
- Exit codes `0`/`1`/`2` are a public contract.
- No test may perform network I/O; no subprocess is ever spawned.
- `docs/RULES.md` is machine-checked by `crates/adept/tests/docs_test.rs`.
- Snapshot changes are behaviour changes and must be read, not blanket-accepted.

## Proposed approach

Land as a series of small commits on a branch off `main`, one per finding, so each
behaviour change is separately reviewable:

- **(1)** Full `ParseErrorRule` refactor (chosen over the minimal inline fix): make parse
  errors flow through the ordinary enablement/severity pipeline so `lint_set` stops
  special-casing `set.errors`, closing `BACKLOG.md:105` rather than just its symptom.

  Two facts constrain the shape. `parse_error_diagnostic` emits **`SL001`, `SL002` and
  `SL003`**, but `PARSE_ERROR_META` (`rules/mod.rs:158`) lists only `SL003`;
  `SL001`/`SL002` already exist as real `SkillRule`s (`frontmatter::MissingDescription`,
  `frontmatter::MissingName`) that fire on skills which *did* parse. So the same code is
  reachable from two paths with metadata in two places, and a naive "one rule per parse
  error" would duplicate `SL001`/`SL002` in the registry.

  Shape: add a third rule trait, `ParseErrorRule: Rule`, with
  `check(&self, path: &Path, err: &AdeptError) -> Vec<Diagnostic>`. `Registry` gains a
  `parse_error_rules` vec registering `frontmatter::MissingDescription`,
  `frontmatter::MissingName`, and a new `frontmatter::MalformedFrontmatter` unit struct for
  `SL003`. `lint_set`'s `set.errors` loop becomes structurally identical to the `set_rules`
  loop — `is_enabled` then `apply_overrides` — so enablement and severity resolve through
  exactly one code path for every rule in the system. `parse_error_diagnostic` and
  `PARSE_ERROR_META` are both deleted; their bodies move into the three `check` impls.
  `MissingDescription`/`MissingName` implement both `SkillRule` and `ParseErrorRule` and are
  registered in both vecs, so `Registry::meta_iter` must dedupe by code to keep `all_meta`
  and the `by_*` lookups single-valued.
- **(2)** Replace the length comparison with a lexicographic `(errors, warnings, infos)`
  tuple comparison, falling back to total diagnostic count when all three are equal.
  Removing an error is progress even at the cost of several infos; introducing an error is
  never progress. `improves_on_len` exists only so `create`'s repair loop can avoid
  materialising a combined `Vec<Diagnostic>` per round — a severity-aware rule needs
  severity data, so it is replaced by a small `Counts { errors, warnings, infos, total }`
  struct that the loop accumulates instead. `passes_severity_gate` is left alone.
- **(3)** Extract `pairwise_similarity(...)` in `cross.rs` parameterized by set-builder,
  threshold, and message/suggestion; both rules shrink to ~15 lines and the mirrored
  double-push lives in one place.
- **(4)/(5)** Point `SL205` at `word_bag` / `words`, hoist the two literals to `const`s with
  a rationale comment matching the style of sibling thresholds.
- **(6)** Doc and cosmetic edits; no behaviour change expected.

Close out `BACKLOG.md:105` ("Parse errors bypass the rule pipeline") in the same commit as
fix (1), since the refactor resolves it rather than deferring it.

## Acceptance criteria

- `adept check <dir> --ignore malformed-frontmatter` suppresses `SL003`, as does
  `--ignore missing-name` for `SL001` and `--ignore missing-description` for `SL002`;
  equivalent `[lint] disabled` and `severity_overrides` entries in `adept.toml` behave
  identically. Regression test covers both the code and the name form for all three.
- `Registry::all_meta` still yields exactly one entry per code with `SL001`/`SL002`
  registered in two vecs; `docs_test.rs` and the `RULES.md` cross-check still pass.
  `parse_error_diagnostic` and `PARSE_ERROR_META` no longer exist.
- A candidate resolving one `Error` into two `Info`s is accepted by `improves_on`; one
  trading two `Warning`s for one `Error` is rejected; equal severity profiles fall back to
  total count. All three cases unit-tested, and the existing
  `improves_on_requires_strictly_fewer` test is updated rather than deleted.
- `grep` finds exactly one pairwise-similarity loop in `cross.rs`; `SL402`/`SL403`
  snapshots are unchanged.
- `description.rs` contains no inline reimplementation of the `text::words` split;
  `SL205` snapshots are unchanged.
- All four CI commands pass; test count is ≥ 336 and every pre-existing test still passes.
- `docs/RULES.md` and `docs/ARCHI.md` updated where behaviour or contracts moved.

## Open questions

_None — all resolved._

## Risks

- **Snapshot churn.** Fixes (3)–(5) are meant to be behaviour-preserving; any snapshot diff
  is a signal something actually changed and must be investigated, not accepted.
- **Fix (2) changes what `adept fix` and `adept create` accept.** Existing tests may encode
  the old count-only semantics and need updating — that's a real behaviour change to state
  in the commit, not a test to silently relax.
- **Fix (1) may surface latent expectations** that `SL001`–`SL003` are unsuppressible, and
  it is now the largest change in the set: a new trait, a new registry vec, dual
  registration for two rules, and meta dedup. If `meta_iter` dedup proves awkward, the
  fallback is to register `SL001`/`SL002` only in `parse_error_rules` for metadata purposes
  and derive `all_meta` from a single union — not to reintroduce a second lookup path.
- **`docs/ARCHI.md` §4 and §11 both describe the current parse-error special case** and must
  be updated in the same commit, or the docs become the stale source of truth.
- Extracting the `cross.rs` helper touches the `Rule` trait boundary; keep the extraction
  a free function so `Rule: Send + Sync` is unaffected.
