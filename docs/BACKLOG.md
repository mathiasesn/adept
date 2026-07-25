# Backlog

Open items as of `930803f` (MVP baseline `9bf467a` → `2e29dff`, plus the
markdown-parsing unification, the vendored-corpus fixture, the reflow
leaning-toothpick fix, the shared sibling-root rule, the SL303
bundled-license exemption, and the SL104 creation-intent exemption on top).
Nothing
here blocks the four shipped surfaces (`check`, `fmt`, `score`, `mcp`); these
are known gaps, deliberate deferrals, and follow-ups surfaced by the two-axis
review. `score` and `mcp` now discover sibling skills through one shared
`adept::sibling_root` (the parent of the skill's own directory), resolving the
divergence previously tracked here.

## Correctness gaps

### Residual SL104 false positives
Down from 55 to a residual of archive/template cases. The **plausible-but-
uncreated companion** class is now handled across `996eb3e` (exemption),
`360f384` (excluded modification verbs after review, documented in
`docs/RULES.md`) and `930803f` (reuse the shared `text::words` tokenizer):
`BrokenFileReference` exempts a path that any occurrence describes creating
(a `CREATION_VERBS` word — create/write/save/generate/output/produce/draft —
on the same body line, "Save test cases to `evals/evals.json`"), and
propagates that exemption to every later read/update mention of the same
path. Modification verbs (`update`, `store`, `populate`) are deliberately
excluded, since they imply the file already exists. Intent is line-granular:
a broken reference sharing a line with an unrelated creation instruction is
not flagged — an accepted narrow bound, since binding the verb to a specific
path would need column tracking through the markdown query layer. This
removed the 3 corpus `skill-creator` → `evals/evals.json` findings (corpus
snapshot 27 → 24) and is regression-covered in `rules/structure.rs`.

The **archive-internal** cases are now handled. They live only in the
non-vendored source-available skills (`docx`, `pdf`, `pptx`, `xlsx`), so the
concern was that any heuristic could be tested only against synthetic
fixtures, not the real inputs — the wrong trade for an Error-severity rule.
Resolved by anchoring the exemption to a *standard* rather than to the skills:
`is_intended_file_reference` now exempts OOXML Open Packaging Conventions part
names (`is_archive_internal_path`, first path segment in `OPC_ROOTS` —
`word`/`ppt`/`xl`/`docProps`/`_rels`/`customXml`), which are constants fixed by
ECMA-376 / ISO/IEC 29500, independently verifiable against the spec. The
exemption is gated on the part extension (`.xml`/`.rels`) so a broken
reference to a non-part file under an OOXML root (`xl/helper.py`) still fires.

The residual was also **smaller than recorded here**. Characterized against
upstream at the vendored pin (mirroring the rule: fenced/indented code
excluded, existence checked against the real bundled tree), the *entire* live
FP set was two references — `word/document.xml` (docx) and
`ppt/slides/slideN.xml` (pptx). `pdf` and `xlsx` produce **zero** SL104
findings: `xlsx`'s `xl/...xml` mentions are all in code blocks or bare prose
(never extracted), and `pdf`'s `FORMS.md`/`REFERENCE.md` are bare prose words,
not links or code spans (and are bundled anyway, as `forms.md`/`reference.md`).
The lone template case (`ppt/slides/slideN.xml`) carries a `ppt/` root, so the
root check covers it; no separate numbered-template pattern is needed. Both
fixes are regression-covered in `rules/structure.rs` and documented in
`docs/RULES.md`.

History: at `130398d` there were exactly 7, and moving SL104 onto the shared
`pulldown-cmark` lexer neither fixed nor worsened them — they were judgement
failures in the heuristic filters, not lexing failures.

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

### `adept fix` writes are not cross-file atomic
`adept_fix::write_all_transactionally` writes every file in a batch to a
sibling temp file first and only renames into place once all temp writes
have succeeded, so a normal I/O error never partially applies a batch. But
each `rename` is atomic only *per file*, not across the whole batch — a
crash or power loss between the first and last rename in a multi-file fix
(SKILL.md plus one or more relocated companion files) can leave the batch
partially applied. Accepted known limitation: there is no cross-file
transaction log, and the exposure window is short (all fallible work
happens before any rename). Documented in `crates/adept_fix/src/writer.rs`.

### `adept fix` never rewrites cross-skill (`SetRule`) findings
Only single-skill (`SkillRule`) diagnostics tagged `FixKind::Llm` (`SL206`,
`SL301`, `SL302`) are ever attempted by `fix_skill` — cross-skill findings
from `SetRule`s (`SL401` `duplicate-skill-name`, `SL402`
`similar-description`, `SL403` `overlapping-trigger-phrasing`) are reported
by `adept check` as usual but `adept fix` never attempts to resolve them.
This is deliberate, not an oversight: a cross-skill rewrite would need to
reconcile changes across multiple skills' files in one candidate, which is
a materially different (and riskier) problem than the single-skill
description/body rewrites `fix_skill` does today. Accepted known
limitation; see `crates/adept_fix/src/lib.rs` module docs.

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
  so real prose does reach the reflow path. All 10 corpus skills now pass at
  `reflow_prose: true` (the leaning-toothpick bug below is fixed).
- **The criterion benchmark asserts nothing.** CI gates performance by parsing
  `--quick` output in a bash step (~23ms against a 500ms threshold). A native
  assertion in the bench would be less brittle than text parsing.
- ~~**No fixture exercises a real skills corpus.**~~ Done: 10 Apache-2.0 skills
  are vendored under `crates/adept/tests/fixtures/corpus/` at upstream
  `1f630fdf9259cec4a14913127dfd7c3b69ef72eb`, and `tests/corpus.rs` snapshots the
  linter's output over them (24 diagnostics, after the SL303 license
  exemption and the SL104 creation-intent exemption below). The manual
  clone-build-diff ritual is retired. What remains narrowed to the items below.
- **The corpus cannot vendor the source-available skills, but the SL104
  archive residuals no longer need it.** `anthropics/skills` is not uniformly
  licensed: `docx`, `pdf`, `pptx` and `xlsx` are source-available, not open
  source, so they are not vendored. They were the skills whose OOXML part-name
  references produced the archive-internal SL104 false positives — now exempted
  via `is_archive_internal_path` (see Correctness gaps), anchored to ECMA-376
  and regression-tested against the two real references characterized from
  upstream, so the exemption is verified without redistributing the skills.
  The corpus's own SL104 finding — `skill-creator` → `evals/evals.json` — is
  suppressed by the creation-intent exemption, so the corpus produces 0 SL104
  findings. Three more skills (`canvas-design`,
  `theme-factory`, `web-artifacts-builder`) are excluded for carrying binary
  assets. Do not "helpfully" refresh the pin; see the corpus README.
- **Setext handling has no real-world coverage.** The corpus contains no setext
  headings at all, so `SL105` fires nowhere in it. Its only evidence is the unit
  and regression tests in `markdown/query.rs` and `tests/rules.rs`.
- ~~**Reflow can emit marker-like line starts ("leaning toothpick").**~~ Fixed
  across `21cd220` (initial bullet/ATX/`>`/ordered guard), `50563e3`
  (generalized to thematic breaks and setext underlines), `76b46cb` (tilde
  code fences `~~~`, plus firmed-up non-vacuous tests) and `99cc8a9` (cleanup).
  `wrap_tokens` in `crates/adept_fmt/src/print.rs` now enforces a single
  invariant: no wrapped line may begin with a marker-like token (`marker_like`),
  covering `-`/`+`/`*` bullets, ATX `#`, blockquote `>`, ordered `N.`/`N)`,
  thematic breaks (`---`/`***`/`___`), setext underlines (`===`, lone `-`), and
  tilde code fences (`~~~`). A marker-like token is forced onto the previous
  line even over-width, or backslash-escaped when it lands at a line start
  unavoidably (only after a hard break, since a paragraph's genuine first token
  is never marker-like). `KNOWN_NON_IDEMPOTENT` is now empty; the
  `algorithmic-art` and `claude-api` corpus skills and the formerly-`#[ignore]`d
  repros pass, with added regression tests for the thematic-break/setext/tilde
  cases.
- ~~**`SL303` flags bundled `LICENSE.txt` files.**~~ Fixed across `f1f82c6`
  (exemption) and `993758f` (cleanup). SL303 now exempts bundled license
  files (`LICENSE`/`LICENCE`/`COPYING`/`COPYRIGHT` any extension, plus
  `LICENSE-*` variants) via `adept::companion::is_license_file`, which lives
  beside `discover_companion_files` because recognizing a license file is a
  companion-file naming concern; the *application* (the skip) is scoped to the
  SL303 rule, so the shared discovery walk and `adept_score`'s token-bloat
  view are untouched. The corpus snapshot dropped from 36 to 27 diagnostics
  (the 9 per-skill `LICENSE.txt` findings are gone), and `docs/RULES.md`
  records the exemption.
- **Diagnostic rendering has no display-root option.** `reporting.rs` renders
  `d.path.display()` verbatim — always the absolute discovery path — so
  `tests/corpus.rs` rewrites each `Diagnostic::path` to be corpus-relative in
  the test before snapshotting. Relativizing for stable output is a renderer
  concern: a `base: Option<&Path>` parameter on `render_human_colored` (and the
  JSON renderer) would serve the corpus test and any CLI caller wanting
  reproducible/relative output, instead of mutating the public `path` field in
  a test.
- **`adept::markdown::build::collect_inlines` never coalesces adjacent `Text`
  events.** A backslash escape splits one word into several `Inline::Text`
  nodes with nothing between them. `adept_fmt` now defends against this locally
  (`build_tokens` coalesces adjacent `Text` before splitting words — landed in
  `877721f`, collapsed to a single exhaustive `match` in `c596f8b`), but the
  surprise is still in the shared AST for any other consumer.

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
- **The formatter has two escaping seams with an unstated ownership split.**
  `escape_text` (in `build_tokens`) unconditionally backslash-escapes a fixed
  inline set (`` \`*_[] ``) at tokenize time; `escape_line_start`/`marker_like`
  handle line-start-only markers (`-`/`=`/`#`/`>`/`~`, ordered, `***`) at wrap
  time. The split is character-arbitrary: `marker_like`'s `*`/`_` arms are dead
  because `escape_text` already covers them, while its `~` arm is live because
  `escape_text` does not — same hazard class, three different justifications.
  A deeper factoring would let line-start escaping own the full positional set
  and leave `escape_text` only the truly context-free escapes, making the
  backstop arms genuinely reachable. Changing it moves observable output
  (where backslashes land), so it is deferred, not folded into cleanup. Related
  to the "text escaping covers a conservative subset" formatter limitation
  above.
- **`marker_like` hand-rolls CommonMark block-start detection.** It re-derives
  the parser's grammar by string matching (`#`-count ≤6, digit-count ≤9,
  tilde-run ≥3, setext/thematic rules) in the printer, even though the crate
  already parses via `adept::markdown`/`pulldown_cmark`. The invariant it
  guards — "re-parsing the emitted line yields the same block structure" — is
  in principle checkable by feeding the candidate line start through the real
  parser instead of a hand-maintained lookalike that can drift. Deferred:
  the oracle approach is a larger rearchitecture with a per-word parse cost on
  the format path, and the current predicate is deliberate and cheap.
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

- Decide the reference-link behaviour above — it changes observable output,
  so it is cheaper to settle before there are users.
- `score` has never run against a live endpoint. Testing is mock-only by
  design; one manual run against a real OpenAI-compatible endpoint would
  confirm the request shape before release.
