# Vendored skills corpus fixture

## Problem / Why
No test exercises a real skills corpus. Every fixture in
`crates/adept/tests/fixtures/rules/` is a single-purpose file authored to fire
exactly one rule, and the CI performance gate lints 100 *generated* skills from
`benches/lint_100_skills.rs` — synthetic text with no real-world messiness.

This has already cost real time. Proving the markdown-parsing unification
(`130398d`) changed no behaviour meant cloning `anthropics/skills` by hand,
building both revisions, and diffing `--format json` output across them
(result: all 66 diagnostics byte-identical). That is a repeatable snapshot test
being run manually. The next refactor of the lexer, the rule pipeline, or the
formatter will have to redo it by hand.

Secondary payoff: the corpus is the safety net that makes the *other* backlog
items cheaper — SL104 heuristic tuning, the `ParseErrorRule` refactor, and
reflow hardening all currently have no broad-input regression check.

## Goals
- A skills corpus lives in-repo and is linted by `cargo test`.
- The corpus output is snapshotted, so a behaviour change in any rule,
  the markdown lexer, or the diagnostic renderer shows up as a reviewable diff.
- The manual clone-build-diff ritual is retired.

## Non-goals
- Vendoring the *entire* `anthropics/skills` repository.
- Formatter *golden-file snapshots* over the corpus. Only the idempotency
  property is in scope (see below); snapshotting formatted output would roughly
  double snapshot volume and churn.
- Fixing whatever formatter bugs the corpus idempotency check uncovers. Finding
  them is in scope; a bug hunt is a separate task (see Risks).
- Replacing the existing per-rule fixtures. This test complements them; it is a
  broad-input regression net, not a rule-behaviour spec.
- Changing the performance benchmark to use the corpus.

## Constraints
- Rust 2021, workspace at `rust-version = "1.85"`. Test deps already available
  to `crates/adept`: `insta`, `tempfile` (dev-dependencies).
- CI (`.github/workflows/ci.yml`) runs `cargo test --workspace` with no network
  access assumed — the corpus must be committed, not fetched at test time.
- Existing snapshot convention: `insta::assert_snapshot!` with snapshots
  committed under `crates/adept/tests/snapshots/`.
- Vendored third-party content must carry its upstream license and provenance.
  The repo itself is MIT (`LICENSE`).
- Size budget: 10–15 complete skill directories. Skill companion files are
  small text (scripts, reference markdown), so this stays modest; no binary
  assets are to be vendored.
- Only Apache-2.0 upstream content may be vendored (see Proposed approach).

## Proposed approach

### Composition and licensing (decided)
Real skills from `anthropics/skills`, copied verbatim at a pinned SHA, companion
files intact. Fidelity is the point — the real messiness is what the heuristics
are tuned against, and synthetic content only ever contains the messiness we
thought to include.

`anthropics/skills` is *not* uniformly licensed. Most skills are Apache 2.0
(redistributable with attribution, a license copy, and a notice of
modifications). But `skills/docx`, `skills/pdf`, `skills/pptx`, and
`skills/xlsx` are **source-available, not open source** — the upstream README
says so explicitly. Those four are exactly the skills whose zip-internal paths
(`word/document.xml`) and template filenames (`slideN.xml`) produce the known
SL104 false positives.

**Vendor Apache-2.0 skills only.** The four source-available skills are
excluded. Consequences, recorded so they are not misread later:
- The corpus will *not* reproduce the 66-diagnostic figure from the
  markdown-unification check, and will contain few or none of the 7 known SL104
  false positives. `docs/BACKLOG.md` must say the SL104 residuals remain
  manually verified, not corpus-covered.
- Apache 2.0 obligations: include the upstream `LICENSE` text under the corpus
  directory, attribute the source, and state that the files are unmodified (or
  list modifications, if any file had to be trimmed).

### Steps
1. Vendor a *mini-corpus* of **10–15 Apache-2.0 skills** under
   `crates/adept/tests/fixtures/corpus/`, one directory per skill, each
   copied **whole** — `SKILL.md` plus every companion file. Companion files are
   not optional: SL104 (broken file reference) and SL303 (companion file bloat)
   read the directory tree, so a `SKILL.md`-only corpus would fabricate SL104
   findings and leave SL303 dead.
   Selection criteria for the 10–15: prefer breadth of markdown structure and
   skill shape (with/without companion files, short and long bodies) over
   picking by topic. Record the criteria actually used in the README.
2. Record provenance in `crates/adept/tests/fixtures/corpus/README.md`: upstream
   repo, the exact commit SHA vendored from, the upstream license, and the
   selection criteria.
3. Add `crates/adept/tests/corpus.rs` that runs `SkillSet::discover` over the
   corpus, lints it with the default `LintConfig`, and snapshots the rendered
   diagnostics. Reuse `render_human_colored(&diagnostics, false)` as
   `tests/rules.rs` does, so the snapshot is stable and colour-free.
4. Make the snapshot deterministic. Ordering is already handled:
   `Linter::lint_set` ends with `sort_diagnostics` (`crates/adept/src/rules/mod.rs:412`),
   so diagnostic order does not depend on `walkdir` traversal order. Paths are
   *not* handled: `reporting.rs:26` renders `d.path.display()`, which is the
   absolute path the set was discovered from, so the test must rewrite each
   `Diagnostic::path` to be corpus-relative before rendering.

5. Add a formatter idempotency check over the corpus: for every corpus
   `SKILL.md`, assert `format(format(x)) == format(x)`. This is the cheap,
   high-value half of the "prose reflow is least-covered" backlog item — the
   existing broad idempotency loop (`format_tests.rs:226`) is gated on
   `reflow_prose: false`, so real prose has never been exercised through reflow.
   Cross-crate placement: the corpus lives under `crates/adept/tests/fixtures/`
   but `adept_fmt` must read it, so the fmt-side test resolves it via
   `CARGO_MANIFEST_DIR/../adept/tests/fixtures/corpus`. Ugly but explicit;
   the alternative (duplicating the corpus) is worse.
   Config: **`FmtConfig::default()`, i.e. `reflow_prose: true`** — that is what
   users get from `adept fmt`, so it is the property worth guarding.
   If corpus prose fails idempotency, that is a genuine finding, not a reason to
   weaken the test:
   - minimize the failing input to the smallest reproducing snippet;
   - fix it if the fix is small and contained;
   - otherwise record it in `docs/BACKLOG.md` with the minimized input, and land
     the test with that specific skill on an explicit known-failing list so the
     remaining corpus still guards the property.
   The known-failing list must be a named `const` with a comment pointing at the
   backlog entry — not a silent skip.

## Acceptance criteria
- `cargo test --workspace` passes and includes a corpus lint test that fails if
  any diagnostic on the corpus changes.
- The corpus test's snapshot is byte-identical across two machines and two
  runs (no absolute paths, no ordering nondeterminism).
- The corpus holds 10–15 Apache-2.0 skill directories, each complete
  (`SKILL.md` + companion files), and none of `docx`/`pdf`/`pptx`/`xlsx`.
- `crates/adept/tests/fixtures/corpus/README.md` states the upstream source,
  the pinned commit SHA, the Apache-2.0 license, the selection criteria, that
  the files are unmodified, and that the pin must not be casually refreshed.
- The upstream Apache-2.0 `LICENSE` text is committed under the corpus dir.
- A formatter idempotency test runs `format(format(x)) == format(x)` over every
  corpus `SKILL.md` with `FmtConfig::default()` (`reflow_prose: true`), and
  passes — either outright, or with any failures minimized, recorded in
  `docs/BACKLOG.md`, and listed in an explicitly-named known-failing const.
- Deliberately reproducing the markdown-unification scenario (checking out the
  pre-unification revision) would surface as a snapshot diff rather than
  requiring a manual clone — verified by reasoning, not necessarily executed.
- `docs/BACKLOG.md` updated: the "No fixture exercises a real skills corpus"
  item is resolved or narrowed to what remains.

## Open questions
*(none — resolved during planning)*

## Risks
- **Snapshot churn.** A broad snapshot over many skills makes every intentional
  rule change produce a large diff. Mitigated by `cargo insta review`, but the
  diff still needs a human. Splitting into per-skill snapshots trades one big
  diff for many small files.
- **License / provenance.** Vendoring third-party skill content requires the
  upstream license to permit redistribution and requires attribution.
- **Staleness.** A pinned vendored corpus drifts from upstream. Acceptable —
  the point is a *fixed* input, not a current one — but the README must say so
  or someone will "helpfully" refresh it and blow up the snapshot.
- **Corpus known-bad findings.** The corpus will produce real diagnostics
  against real skills, and the snapshot enshrines them as expected output. The
  test must document that a *shrinking* diff is a win, not a regression.
- **Reflow idempotency may fail on first contact.** This is the likeliest
  source of unplanned work: real prose has never been through `reflow_prose:
  true` in a broad test. The known-failing-list escape hatch bounds it, but
  expect this task to surface at least one formatter finding.
- **Licence drift.** If upstream relicenses or moves skills between the
  open-source and source-available sets, the pinned SHA in the README is the
  record of what was true at vendoring time. Do not refresh the pin without
  re-checking which skills are Apache-2.0.
