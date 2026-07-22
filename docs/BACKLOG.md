# Backlog

Open items as of `877721f` (MVP baseline `9bf467a` → `2e29dff`, plus the
markdown-parsing unification on top). Nothing here blocks the four shipped
surfaces (`check`, `fmt`, `score`, `mcp`); these are known gaps, deliberate
deferrals, and follow-ups surfaced by the two-axis review.

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

Re-confirmed at `130398d`: still exactly 7, and the same 7. Moving SL104 onto
the shared `pulldown-cmark` lexer neither fixed nor worsened them, which is
the expected result — they are judgement failures in the heuristic filters,
not lexing failures.

### Formatter limitations
Documented in `crates/adept_fmt`, each visible as an unexpected diff to users:
- Reference-style link *definitions* are inlined at each use rather than
  re-emitted as definitions.
- Setext headings are always converted to ATX (`HeadingStyle` has one variant).
  The linter now agrees this is the house style — `SL105 setext-heading` (Info)
  flags them — so this is no longer a silent surprise, but adding a second
  `HeadingStyle` variant would still mean deciding what `SL105` does under it.
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

Deliberately not done, since `check` runs ~18ms against a 1s target
(`lint_100_skills`: 18.4ms before the markdown unification, 18.1ms after):

- Skill discovery/parsing and the per-skill lint loop are sequential and
  embarrassingly parallel (`rayon` would cut wall time ~Ncores).
- **The `SL1xx` rules parse each skill body five times** — `markdown::headings`
  in `SL102`, `SL103` and `SL105`, plus `link_destinations` and
  `inline_code_spans` in `SL104` — where the pre-unification code made one line
  pass. This was measured, not assumed: it costs nothing detectable, because
  token counting dominates. Recorded so it is not rediscovered as a bug and
  "fixed" without a benchmark. If rule dispatch ever grows a per-skill context
  object, parsing once belongs there; do not bolt on a cache for its own sake.
- `SL402`/`SL403` are each O(n²) pairwise Jaccard. Fine at 100 skills; at
  1000 it's 500k pairs and will dominate. Hashing words to `u64` would remove
  the per-word `String` allocation.
- `Skill` retains both `source` and `body`, ~2× file bytes per skill.
  `source` plus a body offset would halve it.

## Test coverage

- **Prose reflow is partly covered.** The proptest still excludes tables,
  fences, HTML, and emphasis, and the fixture idempotency loop is still gated on
  `reflow_prose: false` — but the vendored corpus is now exercised at
  `reflow_prose: true` (`idempotency_holds_for_corpus_with_prose_reflow_enabled`),
  so real prose does reach the reflow path. 8 of 10 corpus skills pass; the two
  that don't are the leaning-toothpick bug below.
- **The criterion benchmark asserts nothing.** CI gates performance by parsing
  `--quick` output in a bash step (~23ms against a 500ms threshold). A native
  assertion in the bench would be less brittle than text parsing.
- ~~**No fixture exercises a real skills corpus.**~~ Done: 10 Apache-2.0 skills
  are vendored under `crates/adept/tests/fixtures/corpus/` at upstream
  `1f630fdf9259cec4a14913127dfd7c3b69ef72eb`, and `tests/corpus.rs` snapshots the
  linter's output over them (36 diagnostics). The manual clone-build-diff ritual
  is retired. What remains narrowed to the items below.
- **The corpus cannot cover the SL104 residuals.** `anthropics/skills` is not
  uniformly licensed: `docx`, `pdf`, `pptx` and `xlsx` are source-available, not
  open source, so they are not vendored — and they are exactly the skills whose
  zip-internal paths and template filenames produce the 7 known SL104 false
  positives. The corpus produces 3 SL104 findings, all
  `skill-creator` → `evals/evals.json`. **The 7 residuals above remain manually
  verified, not corpus-covered.** Three more skills (`canvas-design`,
  `theme-factory`, `web-artifacts-builder`) are excluded for carrying binary
  assets. Do not "helpfully" refresh the pin; see the corpus README.
- **Setext handling has no real-world coverage.** The corpus contains no setext
  headings at all, so `SL105` fires nowhere in it. Its only evidence is the unit
  and regression tests in `markdown/query.rs` and `tests/rules.rs`.
- **Reflow can emit marker-like line starts ("leaning toothpick").**
  `wrap_tokens`/`build_tokens` in `crates/adept_fmt/src/print.rs` choose break
  points purely by width, never checking whether the resulting line-initial
  token would re-parse as CommonMark block syntax — a `-`/`+`/`*` bullet, ATX
  `#`, blockquote `>`, or ordered-list `N.`. This breaks
  `format(format(x)) == format(x)` whenever prose has such a character
  mid-sentence where the width limit happens to fall. Confirmed in two corpus
  skills: `algorithmic-art` (a mid-sentence `-` wraps into a line read as a
  nested list item, flipping the list from tight to loose) and `claude-api` (a
  `+` inside a blockquote). Both sit on `KNOWN_NON_IDEMPOTENT` in
  `crates/adept_fmt/tests/format_tests.rs`, with `#[ignore]`d minimized repros
  ready to un-ignore. Likely fix: when a candidate break would put a
  block-marker token at line start, force it onto the previous line even
  over-width, or escape the leading character.
- **`SL303` flags bundled `LICENSE.txt` files.** 9 of the corpus's 36
  diagnostics are SL303 companion-file-bloat findings against the per-skill
  Apache-2.0 license text every upstream skill ships. Bundled license text is
  not bloat in any useful sense; the rule probably wants a license-filename
  exemption. The current snapshot enshrines these as expected output.
- **`adept::markdown::build::collect_inlines` never coalesces adjacent `Text`
  events.** A backslash escape splits one word into several `Inline::Text`
  nodes with nothing between them. `adept_fmt` now defends against this locally
  (see the reflow fix in `877721f`), but the surprise is still in the shared
  AST for any other consumer.

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
- **`SL105`'s fix suggestion reads oddly for hash-prefixed headings.** For
  `#hashtag\n========` it suggests ``write it as `# #hashtag` ``, which is
  correct CommonMark but looks like a typo. Cosmetic; the message would need to
  special-case a `#`-leading heading text.
- **The formatter's semantic oracle no longer pins its own parser options.**
  `crates/adept_fmt/tests/format_tests.rs` used to construct its own `Parser`
  as an independent differential check; it now calls `adept::markdown::parser`
  so the workspace holds exactly one construction site. The oracle still works
  (it compares events before vs after formatting), but an `Options` flag added
  in `adept::markdown` now silently changes the oracle too. Deliberate trade:
  the single-construction-site invariant is worth more than the duplicated
  option list.

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
