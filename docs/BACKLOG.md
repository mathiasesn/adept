# Backlog

Open items as of `1e89f60` (MVP baseline `9bf467a` → `2e29dff`, plus the
markdown-parsing unification, the vendored-corpus fixture, the reflow
leaning-toothpick fix, the shared sibling-root rule, the SL303
bundled-license exemption, the SL104 creation-intent exemption, and the
`adept fix` command on top). Nothing here blocks the five shipped surfaces
(`check`, `fmt`, `score`, `fix`, `mcp`); these are known gaps, deliberate
deferrals, and follow-ups surfaced by the two-axis review. `score` and `mcp`
now discover sibling skills through one shared `adept::sibling_root` (the
parent of the skill's own directory), resolving the divergence previously
tracked here.

Items struck through below are closed; they are retained with their commit
trail so a resolved concern is not rediscovered and re-litigated. The only
work gated on a release rather than a push is the pre-publish checklist at
the end.

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
`adept_agent::write_all_transactionally` writes every file in a batch to a
sibling temp file first and only renames into place once all temp writes
have succeeded, so a normal I/O error never partially applies a batch. But
each `rename` is atomic only *per file*, not across the whole batch — a
crash or power loss between the first and last rename in a multi-file fix
(SKILL.md plus one or more relocated companion files) can leave the batch
partially applied. Accepted known limitation: there is no cross-file
transaction log, and the exposure window is short (all fallible work
happens before any rename). Documented in `crates/adept_agent/src/writer.rs`.

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
limitation; see `crates/adept_agent/src/lib.rs` module docs.

### Parse errors bypass the rule pipeline
`SL001`/`SL002`/`SL003` are synthesized by a `match` on `AdeptError` in
`Linter::lint_set`, which re-inlines the enable/severity logic that
`LintConfig` already owns. A third rule flavor (`ParseErrorRule`) in the same
`Registry` would share dispatch. Until then, a custom `SkillParser` has no
seam to contribute its own error codes — the `match` is closed over
`AdeptError`.

## Performance

Deliberately not done, since `check` runs well under the 1s target
(`lint_100_skills`: 18.4ms before the markdown unification, 18.1ms after,
19.6ms at `1e89f60`). CI's ~23ms figure is the same benchmark on runner
hardware, not a regression — compare like for like before reading either
number as movement:

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

`adept fix` has its own, differently-shaped performance item — its cost is
network latency, not CPU:

- **Per-skill LLM calls run sequentially.** `commands/fix.rs` loops over the
  discovered skills and `block_on`s one `fix_skill` at a time, so fixing N
  skills costs N × (rounds × request latency) even though the calls are
  independent. Buffering them through a bounded `futures` concurrency limit is
  the single biggest wall-clock win available for multi-skill runs. Deliberately
  deferred out of the cleanup pass because it is a **behavior** change, not a
  simplification: it reorders which skill's error aborts the run (today the
  first in path order, always) and changes rate-limit exposure against the
  configured endpoint. Wants its own task, with a concurrency cap sourced from
  `[fix]` config rather than hardcoded.

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
- **`FixFileConfig` and `ScoreFileConfig` are structurally identical.** Both
  carry `model`, `base_url` and `tokenizer` (`fix` adds `max_rounds`), and both
  resolve through the same shared `resolve_llm_client` helper. A
  `#[serde(flatten)]`ed common `LlmFileConfig` would remove the duplication, at
  the cost of making the two sections harder to document independently. Not
  worth it at three fields; revisit if a third LLM-backed command appears or
  the shared field set grows. The independence of the `[fix]` and `[score]`
  *sections* is a spec requirement and must survive any such refactor — this is
  about the Rust structs, not the TOML surface.
- **`FixKind::Deterministic` is a documented placeholder with no
  implementors.** It exists so the tag's shape does not have to change when
  mechanical autofixes land (spec non-goal: "those belong on a future
  `check --fix`"). Retained deliberately; do not delete it as dead code.
- **`adept_agent` is the one crate that depends on its siblings.** ARCHI's
  layering rule is otherwise one-way (everything depends on `adept`, siblings
  never on each other), but the spec explicitly mandates reusing
  `adept_score`'s `LlmClient` stack and `adept_fmt`'s canonicalization rather
  than duplicating HTTP/auth or printer code. Recorded in `docs/ARCHI.md` as a
  top-of-stack composing crate that nothing else may depend on. The alternative
  — moving the `LlmClient` stack down into `adept` — would restore the
  original invariant at the cost of putting `reqwest` in the core crate.
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

## External prior art: `huggingface/upskill`

Recorded so this survey is not repeated. `huggingface/upskill` (Python,
`uv`/`uvx`, built on fast-agent) generates and *empirically evaluates* agent
skills: a teacher model writes a `SKILL.md`, a test generator produces
synthetic cases, and a student model is run with and without the skill to
measure whether the skill actually helped. Its pipeline is `generate` →
`generate_tests` → `eval` → `runs`, its prompts live as three standalone
"agent cards" (`skill_gen.md`, `test_gen.md`, `evaluator.md`), and its
graders live in `verifiers.py` / `validators/`.

The two projects are **complementary, not competing**: adept checks
*conformance* (does this skill file follow the rules that correlate with
working well) and never runs a skill; upskill measures *outcomes* and never
checks frontmatter validity or token budgets. The single overlap is
`adept score`, and it is the overlap where adept is weakest — `score`'s
triggering accuracy is an LLM judge over synthetic prompts, a proxy for
behaviour, where upskill measures behaviour directly.

Ranked. Items 1 and 2 are **accepted at full scope**, which supersedes two of
the spec's original non-goals; read the "Deferred by design" section below
alongside them, and `docs/MVP.md` where those non-goals originate.

1. **Deterministic verifiers as an `adept fix` accept gate.** upskill's
   `verifiers.py` is ~150 lines of dumb, reliable grading — `contains`,
   `file_exists`, `file_contains`, shell `command` (exit code, 60s timeout)
   — plus a pluggable validator registry (a `register_validator("name")`
   decorator), each returning `{passed, assertions_passed, assertions_total}`.
   Today every adept LLM surface is judged by another LLM call; `fix`'s
   accept/reject gate relies on re-linting alone. Deterministic assertions
   would give it a gate that cannot itself hallucinate.
   **Adopted at full scope, including the shell `command` verifier**, which
   supersedes the "executing or sandbox-testing skill scripts" non-goal (see
   below, and `docs/MVP.md`). Executing skill-supplied commands is a real new
   threat surface for a tool that until now only reads files, so the design
   owes three things before any code lands: execution must be opt-in via
   flag/config rather than on by default; it must be unreachable from `check`
   and `fmt`, which stay offline and side-effect-free; and upskill's limits
   (60s timeout, ~1200-char output truncation) are the floor, not the design.
   Sandboxing is the genuinely open question — upskill leans on its executor
   (including a container image for the HF Jobs path) for isolation, and
   adept, as a single binary with no orchestration layer, has no equivalent
   to lean on.
2. **Baseline / lift, reported as lift-per-token.** upskill's headline metric
   is *skill lift*: pass rate with the skill minus pass rate without it,
   computed automatically in single-model mode. `adept score` reports an
   absolute F1 with no counterfactual, so a 0.95 does not tell you whether
   the skill earns the context budget it spends. Pairing lift with the
   existing token-bloat analysis gives lift-per-token, which is the number
   that actually decides whether a skill should exist — and which neither
   tool computes today. **Adopted at full scope**, superseding the "full
   agent-harness end-to-end evals" non-goal: a real baseline comparison means
   running a task with and without the skill and grading the outcome, which
   is an end-to-end eval by definition. Two consequences to design around.
   First, this is a *new surface*, not an extension of `score` — `score`
   answers "would this skill trigger", lift answers "did this skill help",
   and collapsing them into one command would make the existing F1 output
   mean two different things depending on flags. Second, it depends on
   item 1: lift is only as trustworthy as the grader that decides whether a
   run passed, so the verifiers land first.
3. **Run persistence and history.** upskill writes
   `runs/<timestamp>/run_N/{run_metadata,run_result}.json` plus a
   `batch_summary.json` and an aggregate `results.csv`, queryable via
   `upskill runs`. `adept score` is fire-and-forget, so there is no way to
   tell whether a `fix` improved anything. `PROMPT_VERSION` already exists in
   `adept_score::prompts` precisely because scores drift between prompt
   revisions — persisted runs keyed by that version would make the versioning
   useful rather than just a warning label. Lowest-risk item here: new I/O in
   `adept_cli`, nothing in the library stack, no non-goal touched.
4. **Split the generator and judge models.** upskill separates skill
   generation, test generation and evaluation into distinct roles with
   distinct model flags and a documented fallback chain (CLI
   `--test-gen-model` → config `test_gen_model` → `skill_generation_model`).
   `adept fix` uses one model to both rewrite and screen. A
   `[fix] model` / `[fix] judge_model` split would let a strong model propose
   while a cheap one screens. Cheap: config surface plus plumbing through the
   existing `resolve_llm_client` helper.
   Note upskill keeps its prompts as external markdown files where adept
   compiles them in — that part is *not* worth copying, since a single static
   binary benefits from compiled-in prompts. Only the role split transfers.
5. **Multi-model comparison (`--runs N`, repeated `-m`).** upskill defaults to
   these because single-sample LLM eval is noise. adept's `--judge-samples` is
   the same instinct, but there is no way to compare two models on one skill.
   `&dyn LlmClient` is already the right seam, so this is mostly CLI surface.
   Nice-to-have; only after 1–3.

~~**Explicitly not worth taking:** skill *generation* (`upskill generate`).~~
**Superseded** by the `adept create` item below, which adopts generation as
a first-class surface. The reasoning recorded here is retained because the
*constraints* it names still bind the design: adept's leverage is that
`check` and `fmt` are offline, deterministic and fast (~19.6ms/100 skills),
and generation is none of those — so it must land as a separate, opt-in,
network-using surface that leaves those two untouched, exactly as `score`
and `fix` already do. The integration framing also still holds in reverse:
`adept check --format json` remains the fast, free gate that belongs inside
any generator's refine loop, including adept's own. The HF Jobs remote
executor stays out of scope: adept has no orchestration story and should
not grow one.

## Planned surface: `adept create`

A sixth surface, a sibling of `score` and `fix` rather than of `check` and
`fmt`: generate a new skill from a description of the task it should cover.
Where `fix` repairs an existing `SKILL.md` and `score` judges one, `create`
produces one that did not exist. This reverses the "skill generation is not
worth taking" conclusion recorded above; that paragraph is annotated
accordingly.

**Source of the authoring rules.** `mattpocock/skills`,
`skills/productivity/writing-great-skills/SKILL.md` (read 2026-07-31; it is
itself user-invoked, `disable-model-invocation: true`, and discloses its
definitions to a sibling `GLOSSARY.md`). Its thesis: a skill exists to
wrangle determinism out of a stochastic system, and **predictability** —
the agent taking the same *process* every run, not producing the same
output — is the root virtue every other lever serves. The levers:

- **Invocation is a two-way choice with a cost on each side.** A
  model-invoked skill keeps a trigger-bearing `description` and pays
  *context load* (the description sits in the window every turn); a
  user-invoked skill sets `disable-model-invocation: true`, pays zero
  context load, and spends *cognitive load* instead — the human becomes the
  index. Model-invocation is warranted only when the agent or another skill
  must reach the skill unaided.
- **The description carries triggers, not identity.** Front-load the
  leading word; one trigger per **branch**; synonyms renaming a single
  branch are duplication; cut anything the body already says.
- **Information hierarchy**, a three-rung ladder: in-skill *step* (ordered
  action, ending on a **completion criterion** that must be checkable and
  often exhaustive), in-skill *reference* (consulted on demand; a flat
  peer-set is legitimate, not a smell), and *external reference* pushed
  behind a **context pointer** whose wording — not its target — decides how
  reliably the agent reaches it. Branching is the disclosure test: inline
  what every branch needs, disclose what only some reach.
- **Split only when the cut earns it**, by invocation (a distinct leading
  word deserving its own trigger) or by sequence (hiding post-completion
  steps that tempt the agent to rush the current one).
- **Pruning**: single source of truth, relevance, and a sentence-by-sentence
  **no-op** test — delete the whole sentence rather than trim it.
- **Leading words**: compact concepts already in pretraining (*tight*,
  *red*, *fog of war*) that collapse a restated triad into one token and
  anchor behaviour by recruiting priors the model already holds.
- **Failure modes** to diagnose against: premature completion, duplication,
  sediment, sprawl, no-op, and **negation** (steering by prohibition
  backfires — prompt the positive).

**Correcting the earlier draft.** This item previously stood in for the
above with the local `write-a-skill` skill, and claimed every requirement
was already an adept rule or could be. Against the real source that claim
is **wrong**, and the difference matters enough to state plainly. The
substituted guide was a mechanical checklist (≤100 lines, ≤1024-char
description, "Use when…" phrasing, when to bundle scripts) — none of which
appears in the real skill, which is instead almost entirely *editorial
judgment*: is this sentence a no-op, is this the right rung of the ladder,
is this word doing pretraining work. Judgment is precisely what a
deterministic linter cannot check, so the honest split is:

- **Already covered by existing rules.** Sprawl is `SL3xx` (token budget);
  broken context pointers are `SL104`; cross-skill duplication is
  `SL402`/`SL403`.
- **New, and genuinely lintable.** An *invocation-mode coherence* rule is
  the strongest candidate: `disable-model-invocation: true` alongside a
  description written with model-facing trigger phrasing ("Use when…") is a
  flat contradiction the skill names explicitly, and it is mechanically
  detectable. `disable-model-invocation` already parses — unknown
  frontmatter keys land in `Skill::extra` (`parser.rs:156`), so the field is
  readable today without a parser change. Negation is a plausible Info-level
  heuristic ("don't"/"never" in imperative position). Intra-skill
  duplication is a weaker but real candidate; adept only compares *across*
  skills today.
- **Not lintable, and should not be faked.** No-ops, leading-word strength,
  ladder placement, completion-criterion sharpness. These belong to the LLM
  surfaces — and this is the actual argument for `create`: the most valuable
  half of skill-authoring guidance is judgment adept can only apply while
  *writing*, not while checking.

**Why this fits adept specifically.** adept is the only tool in this space
that can lint its own output before emitting it. `create` should therefore
not be a prompt that returns markdown; it should be a generate → `check` →
repair loop that refuses to emit a skill its own linter rejects, reusing
the accept/reject machinery `adept_agent` already has. Note the honest bound
from the split above: a clean `adept check` proves the mechanical tier
only, and the guide's own emphasis lies mostly outside it. `create` is
worth building because the generation prompt can carry the judgment the
linter cannot — not because the linter validates it.

**Placement.** `adept_agent` is already the designated top-of-stack crate that
may compose `adept_score` and `adept_fmt`; `create` needs the same three
things (LLM transport, linting, canonicalization) plus a writer. Two
options, and this is the open design question: a new `adept_create` crate
alongside `adept_agent` at the top of the stack, or a module within
`adept_agent` on the grounds that "LLM proposes, linter screens, writer
commits" is one pipeline instantiated twice. The second is less code and
keeps one accept-gate implementation; the first keeps a crate named after
repair from also owning creation. Either way nothing in the library stack
may depend on it, and `adept_cli` is the only consumer.

**Constraints inherited, not renegotiated.** `create` calls the network, so
it lives under the same rules as `score` and `fix`: `check` and `fmt` stay
offline and side-effect-free; no test performs network I/O
(`MockLlmClient`); exit codes keep their `0`/`1`/`2` meanings; writes are
atomic. It also *writes new files into a directory the user names*, which
is a wider blast radius than `fix`'s in-place rewrite of files it was
pointed at — so it needs a preview-by-default mode with an explicit
`--write`, mirroring `fix`, and it must refuse to overwrite an existing
skill directory without an explicit opt-in.

**Ordering.** Independent of the five upskill items above, but sequenced
behind them in value: `create` produces skills, and items 1–2 are what
would tell you whether the skills it produces are any good. Building
generation before the lift measurement means shipping a generator with no
way to evaluate it beyond its own linter.

## Deferred by design

From the spec's non-goals and constraints — recorded so they aren't
rediscovered as bugs:

- Hosted registry / marketplace / index.
- ~~Executing or sandbox-testing skill scripts.~~ **Superseded** by item 1 of
  the upskill survey above: adept will grow deterministic verifiers including
  a shell `command` verifier, to give `adept fix` an accept gate that is not
  itself an LLM. The non-goal is retired as a *product* boundary, not as a
  safety one — the constraints in item 1 (opt-in, never reachable from
  `check`/`fmt`, hard timeout and output caps, sandboxing story required)
  replace it and are binding.
- ~~Full agent-harness end-to-end evals (only prompt→trigger judgments).~~
  **Superseded** by item 2: measuring skill lift requires running a task with
  and without the skill and grading the result. `score` keeps its narrower
  prompt→trigger meaning; lift belongs to a new surface, and depends on the
  verifiers from item 1 landing first.
- Non-Anthropic skill formats. The parser trait exists; see the filename
  limitation above before building on it.

Both supersessions originate in `docs/MVP.md`'s non-goals list, annotated
there to match. They are decisions of record, not open questions — but note
neither has an implementation yet, so nothing in the shipped binary has
changed and `check`/`fmt` remain offline and side-effect-free.

## Pre-publish checklist

- Decide the reference-link behaviour above — it changes observable output,
  so it is cheaper to settle before there are users.
- `score` and `fix` have never run against a live endpoint. Testing is
  mock-only by design; one manual run of each against a real
  OpenAI-compatible endpoint would confirm the request shape before release.
  `fix` is the more important of the two to exercise: unlike `score` it
  *writes*, and its accept/reject gate has only ever seen `MockLlmClient`
  responses shaped by hand — a real model's JSON (fenced, truncated, or
  ignoring the companion-edit contract) is the untested input class. Run it
  with the default preview mode first, not `--write`.

## Tracing & capture follow-ups

Recorded 2026-07-31, from `specs/cli-tracing.md`.

- **`crates/adept` insta snapshots bake an absolute path.** The rule snapshots
  embed `/home/mathias/code/adept/...` as the diagnostic path, so all 21
  rule-snapshot tests fail in any git worktree (and would fail on any other
  machine or in CI under a different checkout root). The fix is an insta
  path filter (`insta::with_settings!` + a `filters` entry rewriting the
  checkout prefix) or emitting fixture-relative paths in the snapshot
  harness. **Do not "fix" this by running `cargo insta accept` from a
  worktree** — that bakes the worktree path in and breaks the main checkout
  instead. Until then, treat those 21 failures as environmental when working
  outside the primary clone.
- **Captured calls do not record which logical step issued them.** Knowing
  whether a call was prompt generation, a judge sample, or fix round N is what
  makes a capture directory readable without the shell history. A
  `CapturedCall::step` field was shipped and then removed: `OpenAiCompatClient`
  sees only a request body and could never populate it, so the field was
  structurally always `None` and made the feature read as more complete than it
  was. The intended shape when this lands is a **`tracing::Span` field set by
  `adept_score::triggering` / `adept_agent` and read at capture time** — the step
  context already lives there, and a span carries it down without widening any
  signature. Explicitly *not* a `set_step()` mutator on the client (it would
  make a `Sync` client statefully order-dependent), and not a struct field the
  client is expected to fill. Until it lands, a reader must infer the step from
  the call ordering and the prompt content.
- **The MCP `score_skill` tool never captures.** `commands/mcp.rs` builds its
  own `OpenAiCompatClient` with no `with_capture` call. This is deliberate, per
  `specs/cli-tracing.md` §12: capture is a CLI-only surface, so the MCP tool
  schema stays unchanged and an MCP client cannot make the server write to
  arbitrary paths on disk. Recorded here as a decision so it does not read as
  an omission — `crates/adept_cli/tests/tracing.rs` pins the schema half of it.

### `LlmError::Status` carries an unscrubbed response body

The defensive scrub added with the capture layer covers every body on its way
into a tracing event or a capture artifact, but `LlmError::Status { body }`
still holds the response text verbatim. If a backend ever echoed the API key
back inside an error body, that key would reach stderr through the error's
`Display`. Scrubbing there means changing the contents of a returned error, so
it was left alone deliberately rather than folded into the capture work. The
exposure requires a misbehaving endpoint, but the fix is cheap if we decide the
error type should carry scrubbed text.
