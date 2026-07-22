# Backlog

Open items as of the MVP (baseline `9bf467a` → `2e29dff`). Nothing here blocks
the four shipped surfaces (`check`, `fmt`, `score`, `mcp`); these are known
gaps, deliberate deferrals, and follow-ups surfaced by the two-axis review.

## Correctness gaps

### MCP `score_skill` cannot detect overlap
`crates/adept_cli/src/commands/mcp.rs` passes `[skill]` as the skillset, so
overlap detection over MCP is inert — a skill is only ever compared against
itself. Fixing this needs a directory argument (or sibling discovery) that the
MCP tool schema does not currently express.

### Residual SL104 false positives
7 findings remain on the anthropics/skills corpus, down from 55. The survivors
are genuinely ambiguous rather than noise: zip-internal paths (`word/document.xml`),
template filenames (`slideN.xml`), and plausible-but-uncreated companion files
(`evals/evals.json`). Resolving them needs semantic or archive-aware checking.

### Formatter limitations
Documented in `crates/adept_fmt`, each visible as an unexpected diff to users:
- Reference-style link *definitions* are inlined at each use rather than
  re-emitted as definitions.
- Setext headings are always converted to ATX (`HeadingStyle` has one variant).
- Tight-list preservation only holds when every item is a single bare-inline
  block; items mixing text with a nested list print as loose. CommonMark-valid,
  but it adds blank lines.
- Text escaping covers a conservative subset rather than every line-start
  ambiguity.

### Parse errors bypass the rule pipeline
`SL001`/`SL002`/`SL003` are synthesized by a `match` on `AdeptError` in
`Linter::lint_set`, which re-inlines the enable/severity logic that
`LintConfig` already owns. A third rule flavor (`ParseErrorRule`) in the same
`Registry` would share dispatch. Until then, a custom `SkillParser` has no
seam to contribute its own error codes — the `match` is closed over
`AdeptError`.

## Performance

Deliberately not done, since `check` runs ~21ms against a 1s target:

- Skill discovery/parsing and the per-skill lint loop are sequential and
  embarrassingly parallel (`rayon` would cut wall time ~Ncores).
- `SL402`/`SL403` are each O(n²) pairwise Jaccard. Fine at 100 skills; at
  1000 it's 500k pairs and will dominate. Hashing words to `u64` would remove
  the per-word `String` allocation.
- `Skill` retains both `source` and `body`, ~2× file bytes per skill.
  `source` plus a body offset would halve it.

## Test coverage

- **Prose reflow is the least-covered high-risk path.** The proptest excludes
  tables, fences, HTML, and emphasis, and the broad-corpus idempotency loop is
  gated on `reflow_prose: false` — so the component the spec flagged as
  highest-effort has the weakest guarantees.
- **The criterion benchmark asserts nothing.** CI gates performance by parsing
  `--quick` output in a bash step (~23ms against a 500ms threshold). A native
  assertion in the bench would be less brittle than text parsing.
- **No fixture exercises a real skills corpus.** The clean-ish acceptance
  criterion was verified manually; a vendored mini-corpus would make it a test.

## API and consistency

- **`SkillParser` only abstracts contents, not filenames.** `SKILL_FILE_NAME`
  is a `const` in `crates/adept/src/skillset.rs`, not part of the trait, so a
  parser for a format using `skill.yaml` or `AGENT.md` can never be handed a
  file. Adding `fn file_names(&self) -> &[&str]` (defaulted) would complete the
  pluggability seam the spec asked for.
- **Deliberate similarity divergence.** `adept_score`'s overlap shortlist uses
  name+description at 0.25; SL402 uses description-only at 0.6. Both now call
  `adept::text::jaccard`, and the divergence is documented — but `check` and
  `score` can still reach different conclusions about the same pair.
- **`--statistics` prints counts in addition to diagnostics**, not instead of.
  One line in `check.rs` if instead-of is wanted.

## Deferred by design

From the spec's non-goals and constraints — recorded so they aren't
rediscovered as bugs:

- Hosted registry / marketplace / index.
- Executing or sandbox-testing skill scripts.
- Full agent-harness end-to-end evals (only prompt→trigger judgments).
- Non-Anthropic skill formats. The parser trait exists; see the filename
  limitation above before building on it.

## Pre-publish checklist

- Decide the MCP overlap and reference-link behaviours above — both change
  observable output, so they are cheaper to settle before there are users.
- `score` has never run against a live endpoint. Testing is mock-only by
  design; one manual run against a real OpenAI-compatible endpoint would
  confirm the request shape before release.
