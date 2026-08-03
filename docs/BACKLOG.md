# Backlog

Known gaps, deliberate deferrals, and follow-ups as of `1e89f60`. Nothing
here blocks the six shipped surfaces (`check`, `fmt`, `eval`, `fix`,
`create`, `mcp`).

**Read this before reporting a bug** — items marked *Resolved* or *Accepted*
are settled, and are retained with their commit trail so they are not
rediscovered and re-litigated. Only the pre-publish checklist is gated on a
release rather than a push.

Every open item carries its GitHub issue (`#N`, in `mathiasesn/adept`). Items
without one are settled by design and have no issue on purpose.

## Correctness

### SL104 false positives — resolved
Down from 55 to zero live cases, via two spec/intent-anchored exemptions in
`is_intended_file_reference`, both regression-covered in `rules/structure.rs`
and documented in `docs/RULES.md`:

- **Creation intent** (`996eb3e`, `360f384`, `930803f`): a path is exempt if
  any occurrence shares a body line with a `CREATION_VERBS` word
  (create/write/save/generate/output/produce/draft); the exemption propagates
  to later read mentions of the same path. Modification verbs (`update`,
  `store`, `populate`) are excluded — they imply the file already exists.
  Intent is line-granular; binding a verb to a specific path would need column
  tracking through the markdown query layer. Accepted bound.
- **OOXML part names** (`is_archive_internal_path`): first segment in
  `OPC_ROOTS` (`word`/`ppt`/`xl`/`docProps`/`_rels`/`customXml`) with a
  `.xml`/`.rels` extension. Anchored to ECMA-376 / ISO/IEC 29500, not to the
  skills that motivated it, so it is verifiable without redistributing the
  source-available skills (`docx`, `pdf`, `pptx`, `xlsx`). `xl/helper.py`
  still fires.

Characterized against upstream at the vendored pin, the entire live FP set was
two references (`word/document.xml`, `ppt/slides/slideN.xml`); `pdf` and
`xlsx` produced zero. The corpus snapshot went 27 → 24.

### Formatter limitations — open (#4)
Each is visible to users as an unexpected diff; documented in `adept_fmt`:

- Reference-style link *definitions* are inlined at each use, not re-emitted.
  Tracked separately as a pre-publish decision (#5).
- Setext headings are always converted to ATX (`HeadingStyle` has one
  variant). `SL105 setext-heading` (Info) makes this the stated house style,
  so it is no longer silent — but a second variant would mean deciding what
  `SL105` does under it.
- Tight-list preservation holds only when every item is a single bare-inline
  block; items mixing text with a nested list print as loose. CommonMark-valid,
  but it adds blank lines.
- Text escaping covers a conservative subset, not every line-start ambiguity.

### `adept fix` writes are not cross-file atomic — accepted
`write_all_transactionally` writes all temp files before any rename, so no I/O
error partially applies a batch. But `rename` is atomic per file: a crash
between the first and last rename in a multi-file fix can leave it partly
applied. No cross-file transaction log; exposure window is short. Documented in
`crates/adept_agent/src/writer.rs`.

### `adept fix` never rewrites cross-skill findings — accepted
`fix_skill` attempts only `SkillRule` diagnostics tagged `FixKind::Llm`
(`SL206`, `SL301`, `SL302`). `SetRule` findings (`SL401`, `SL402`, `SL403`) are
reported by `check` and never fixed: reconciling one candidate across multiple
skills' files is a materially different and riskier problem. See
`crates/adept_agent/src/lib.rs` module docs.

## Performance

Deliberately not done — `check` is far under the 1s target
(`lint_100_skills`: 19.6ms at `1e89f60`; CI's ~23ms is the same benchmark on
runner hardware, not a regression). Compare like for like before reading
either number as movement.

- **Discovery/parsing and the per-skill lint loop run sequentially (#7).**
  They are embarrassingly parallel; `rayon` would cut wall time ~Ncores.
- **`SL1xx` parses each body five times (#10).** `markdown::headings` in
  `SL102`, `SL103`, `SL105`; `link_destinations` + `inline_code_spans` in
  `SL104`. Measured, not assumed: costs nothing detectable, because token
  counting dominates. Do not "fix" without a benchmark. If rule dispatch ever
  grows a per-skill context object, parsing once belongs there.
- **`SL402`/`SL403` are each O(n²) pairwise Jaccard (#8).** Fine at 100
  skills; at 1000 it's 500k pairs and will dominate. Hashing words to `u64`
  would remove the per-word `String` allocation.
- **`Skill` retains both `source` and `body`, ~2× file bytes per skill
  (#9).** `source` plus a body offset would halve it.

**`adept fix` per-skill LLM calls run sequentially (#6).** `commands/fix.rs`
`block_on`s one `fix_skill` at a time, so N skills cost N × rounds × latency
despite being independent. Bounded `futures` concurrency is the biggest
wall-clock win available. Deferred because it is a *behavior* change, not a
simplification: it reorders which skill's error aborts the run (today always
the first in path order) and changes rate-limit exposure. Wants its own task,
with the concurrency cap sourced from `[fix]` config.

## Test coverage

- **Prose reflow is partly covered (#11).** The proptest still excludes tables,
  fences, HTML, and emphasis, and the fixture idempotency loop is still gated
  on `reflow_prose: false` — but the vendored corpus runs at
  `reflow_prose: true`
  (`idempotency_holds_for_corpus_with_prose_reflow_enabled`), and all 10 corpus
  skills pass.
- **The criterion benchmark asserts nothing (#12).** CI gates perf by parsing
  `--quick` output in a bash step (~23ms vs a 500ms threshold). A native
  assertion would be less brittle.
- **Setext handling has no real-world coverage (#13).** The corpus contains no
  setext headings, so `SL105` fires nowhere in it. Evidence is unit/regression
  tests in `markdown/query.rs` and `tests/rules.rs` only.
- **Diagnostic rendering has no display-root option (#14).** `reporting.rs`
  renders `d.path.display()` verbatim (always absolute), so `tests/corpus.rs`
  mutates each `Diagnostic::path` to be corpus-relative before snapshotting.
  Relativizing is a renderer concern: a `base: Option<&Path>` parameter on
  `render_human_colored` and the JSON renderer would serve the corpus test and
  any CLI caller wanting reproducible output.
- **`markdown::build::collect_inlines` never coalesces adjacent `Text`
  events (#15).** A backslash escape splits one word into several `Inline::Text`
  nodes. `adept_fmt` defends locally (`877721f`, `c596f8b`), but the surprise
  remains in the shared AST for any other consumer.
- **`repo_root()` is triplicated across `crates/adept/tests` (#36).**
  `rules.rs:18`, `docs_test.rs:14` (inlined into `load_doc`) and
  `workspace_metadata.rs` each encode "this crate lives at
  `<root>/crates/adept`" separately, two via `../..` and one via
  `.parent().and_then(Path::parent)`. Each `tests/*.rs` is its own crate, so
  sharing needs a `tests/common/mod.rs` — the layout
  `crates/adept_cli/tests/common/mod.rs` already establishes. They fail at
  different times because they are separate test binaries.
- **Nothing pins the crates.io publish metadata on new members (#37).**
  `workspace_metadata.rs` asserts the `Cargo.toml` ↔ `release-plz.toml`
  bijection, so a new crate cannot fall out of the shared `version_group` —
  but no test requires it to carry `keywords`/`homepage`/`categories`. The
  asymmetry favours the wrong half: release-plz drift is loud, whereas wrong
  metadata publishes silently and is then immutable for that version. The
  concrete case is a new *binary* crate forgetting to override the
  library-case `categories` default.

**Corpus, resolved.** 10 Apache-2.0 skills are vendored under
`crates/adept/tests/fixtures/corpus/` at upstream
`1f630fdf9259cec4a14913127dfd7c3b69ef72eb`; `tests/corpus.rs` snapshots the
linter over them (24 diagnostics). The manual clone-build-diff ritual is
retired. `anthropics/skills` is not uniformly licensed — `docx`, `pdf`,
`pptx`, `xlsx` are source-available and cannot be vendored (their SL104
residuals are handled by `is_archive_internal_path` instead);
`canvas-design`, `theme-factory`, `web-artifacts-builder` are excluded for
binary assets. Do not "helpfully" refresh the pin; see the corpus README.

**Resolved, do not reopen:**

- ~~Reflow emits marker-like line starts ("leaning toothpick").~~ Fixed across
  `21cd220`, `50563e3`, `76b46cb`, `99cc8a9`. `wrap_tokens`
  (`adept_fmt/src/print.rs`) enforces one invariant: no wrapped line may begin
  with a `marker_like` token — bullets, ATX `#`, `>`, ordered `N.`/`N)`,
  thematic breaks, setext underlines, tilde fences. Such a token is forced onto
  the previous line even over-width, or escaped when unavoidable (only after a
  hard break). `KNOWN_NON_IDEMPOTENT` is now empty.
- ~~`SL303` flags bundled `LICENSE.txt`.~~ Fixed in `f1f82c6`, `993758f`.
  `adept::companion::is_license_file` recognizes them; the skip is scoped to
  SL303, leaving the shared discovery walk and the token-bloat view untouched.
  Corpus snapshot 36 → 27.

## API and consistency

- **`SkillParser` abstracts contents, not filenames (#16).** `SKILL_FILE_NAME`
  is a `const` in `skillset.rs`, not part of the trait, so a parser for
  `skill.yaml` or `AGENT.md` can never be handed a file. A defaulted
  `fn file_names(&self) -> &[&str]` would complete the pluggability seam.
- **Deliberate similarity divergence.** `adept_agent::eval`'s overlap shortlist
  uses name+description at 0.25; SL402 uses description-only at 0.6. Both call
  `adept::text::jaccard`. The thresholds differ because the jobs do: 0.25
  casts a wide net for a shortlist an LLM pass then judges, while 0.6 must
  flag near-duplicates on its own with no LLM to filter it. `check` and
  `eval` can therefore reach different conclusions about the same pair —
  accepted, not a defect to reconcile.
- **`--statistics` prints counts in addition to diagnostics (#17).** Not
  instead of. One line in `check.rs` if instead-of is wanted.
- **`FixFileConfig` / `EvalFileConfig` / `CreateFileConfig` are near-identical,
  and this item's trigger has fired (#18).** All three carry `model`,
  `base_url`, `tokenizer`, `capture_dir` (`fix`/`create` add `max_rounds`;
  `create` adds `eval_cases`) and resolve through `resolve_llm_client`. The
  condition was "a third LLM-backed command" — `adept create` is it. The
  refactor (a `#[serde(flatten)]`ed common `LlmFileConfig`) was kept out of
  scope of both `create` and the `score`→`eval` unification because it touches
  all three. The independence of the `[fix]`/`[eval]`/`[create]` *TOML sections*
  is a spec requirement and must survive any such refactor.
- **No blanket `deny_unknown_fields` on config structs (#19).** The
  stale-`[score]` case is handled precisely (`contains_legacy_score_section`, a
  hard error naming `[eval]`; ARCHI §7). Making every typo a hard error is a
  separate, broader decision — a mistyped `[lint]` key would go from silently
  ignored to usage error.
- **`FixKind::Deterministic` has no implementors, deliberately.** It exists so
  the tag's shape need not change when mechanical autofixes land (spec
  non-goal: those belong on a future `check --fix`). Do not delete as dead code.
- **`adept_agent` is the one crate depending on a sibling.** Layering is
  otherwise one-way, but the spec mandates reusing `adept_fmt`'s
  canonicalization. Recorded in ARCHI as a top-of-stack crate nothing else may
  depend on. The exception is narrower than it was: `adept_score`'s transport
  moved *into* `adept_agent::llm` rather than remaining a second sibling dep.
- **`SL105`'s fix suggestion reads oddly for hash-prefixed headings (#20).** For
  `#hashtag\n========` it suggests ``write it as `# #hashtag` `` — correct
  CommonMark, looks like a typo. Cosmetic.
- **The formatter has two escaping seams with an unstated ownership split
  (#21).** `escape_text` unconditionally escapes a fixed inline set
  (`` \`*_[] ``) at tokenize time; `escape_line_start`/`marker_like` handle
  line-start markers at wrap time. The split is character-arbitrary:
  `marker_like`'s `*`/`_` arms are dead (covered by `escape_text`) while its
  `~` arm is live. A deeper factoring would give line-start escaping the full
  positional set and leave `escape_text` the context-free escapes. Moves
  observable output, so deferred.
- **`marker_like` hand-rolls CommonMark block-start detection in the
  printer (#22)** (`#`-count ≤6, digit-count ≤9, tilde-run ≥3,
  setext/thematic rules), despite the crate already parsing via
  `adept::markdown`. Its invariant — re-parsing the emitted line yields the
  same block structure — is in principle checkable against the real parser.
  Deferred: the oracle is a larger rearchitecture with a per-word parse cost
  on the format path.
- **`workspace_metadata.rs` lives in `adept-core`, not `adept_cli`.** Unlike
  `docs_test.rs`, which is here because it imports `adept::Registry`, this test
  imports nothing from `adept`: its subject is workspace-level config
  (`release-plz.toml` and the member list), which would put it closer to
  `adept_cli`, the layer that already owns config-file parsing. It stays here
  because a virtual workspace root cannot host tests and `crates/adept/tests`
  is the established home for doc/config drift checks. `toml` is a
  dev-dependency only, so nothing reaches the published artifact.
- **The formatter's semantic oracle no longer pins its own parser options.**
  `adept_fmt/tests/format_tests.rs` now calls `adept::markdown::parser`, so an
  `Options` flag added there silently changes the oracle too. Accepted as the
  price of the single-construction-site invariant: an oracle with its own
  `Options` would drift from the parser it is supposed to check.
- **The Windows branch of `python/adept/__main__.py` has no CI coverage —
  accepted.** The `python-packaging` CI job (`.github/workflows/ci.yml`) is
  ubuntu-only, so the `subprocess.run` branch never runs. Accepted deliberately
  when the Linux-only job was chosen, not rediscovered as a defect.
  `docs/ARCHI.md` §6 has the mechanics of why that branch exists at all.

## External prior art: `huggingface/upskill`

Recorded so this survey is not repeated. upskill (Python, on fast-agent)
generates and *empirically evaluates* skills: a teacher model writes a
`SKILL.md`, a test generator produces cases, and a student model runs with
and without the skill to measure whether it helped. Pipeline: `generate` →
`generate_tests` → `eval` → `runs`; prompts are external markdown agent
cards; graders live in `verifiers.py` / `validators/`.

The projects are **complementary**: adept checks *conformance* and never
runs a skill; upskill measures *outcomes* and never checks frontmatter or
token budgets. Items 1–2 below closed the one overlap and are shipped; they
supersede two of the project's original non-goals — read them alongside
"Deferred by design" below.

1. ~~**Deterministic verifiers as an `adept fix` accept gate.**~~ **Shipped**,
   but relocated: the four verifiers (`contains`, `file_exists`,
   `file_contains`, `command`) landed as `adept::evals`'s assertion vocabulary
   and offline grader (`grade`), reached via `adept eval --results` /
   `--select evals` and the MCP `eval_skill` tool — not as `fix`'s gate. adept
   still executes nothing: `command` is graded only from a harness-supplied
   exit code, so this item's sandboxing/timeout concerns never applied. See
   `docs/EVALS.md`.
2. ~~**Baseline / lift.**~~ **Shipped** as skill lift (percentage points,
   `pass_rate - baseline_pass_rate`) from `arm: "skill"`/`"baseline"` results,
   surfaced in `adept eval`'s `evals` analysis. **Not** shipped as
   lift-*per-token* (#26) — lift and token usage are separate fields;
   combining them is still open. `triggering` still answers "would this
   skill trigger" (a prompting proxy); `evals` answers "did it help"
   (measured).
3. **Run persistence and history (#23).** upskill writes `runs/<timestamp>/...`
   plus a batch summary and `results.csv`, queryable via `upskill runs`. `adept
   eval` is fire-and-forget (`--capture-dir` records individual LLM calls, not
   runs), so there is no way to tell whether a `fix` improved anything over
   time. `PROMPT_VERSION` exists precisely because scores drift between prompt
   revisions — runs keyed by it would make the versioning useful. Lowest-risk
   item here: new I/O in `adept_cli`, nothing in the library stack.
4. **Split the generator and judge models (#24).** upskill separates generation,
   test generation and evaluation into roles with distinct model flags and a
   documented fallback chain. `adept fix` uses one model to both rewrite and
   screen; a `[fix] model` / `judge_model` split would let a strong model
   propose while a cheap one screens. Cheap: config surface plus plumbing
   through `resolve_llm_client`. (upskill's *external* prompt files are not
   worth copying — a static binary benefits from compiled-in prompts.)
5. **Multi-model comparison (`--runs N`, repeated `-m`) (#25).** Single-sample
   LLM eval is noise; `--judge-samples` is the same instinct, but there is no
   way to compare two models on one skill. `&dyn LlmClient` is already the right
   seam, so this is mostly CLI surface. Only after 1–3.

~~**Not worth taking: skill generation.**~~ Superseded by `adept create`
below. The *constraints* named here still bind: adept's leverage is that
`check`/`fmt` are offline, deterministic and fast (~19.6ms/100 skills), and
generation is none of those — so it lands as a separate, opt-in,
network-using surface leaving those two untouched. Conversely, `adept check
--format json` remains the fast free gate belonging inside any generator's
refine loop, including adept's own. The HF Jobs remote executor stays out of
scope: adept has no orchestration story and should not grow one.

## Shipped surface: `adept create`

Generates a new skill from a description of the task it should cover — a
sibling of `eval` and `fix`, not of `check` and `fmt`.

**Shipped:** `adept_agent::create` (generate → screen → repair →
generate-evals; computing only, never writes) plus the CLI
(`--from-file`/stdin/interactive precedence, `--out`, `--name`, `--write`,
`--overwrite`, `--format json`, `[create]` config) and preview-only MCP
`create_skill` / `generate_evals` tools. See ARCHI §4/§8/§9/§12 and
`docs/EVALS.md`.

**The gate drives the repair loop; it does not veto the output.** An exhausted
`max_rounds` still emits the best candidate seen, reports every remaining
diagnostic prominently, and exits `1` — the user is never left empty-handed
after paying for N rounds. (This amends the earlier "refuses to emit" framing.)

**`create` never runs what it generates.** It generates and validates eval
datasets including the shell `command` assertion, but there is no runner and no
subprocess, pinned by `crates/adept_cli/tests/no_subprocess.rs` (scans
workspace crate sources, comments and string literals stripped, for
`Command::new`/`process::Command`). The item-1 supersession above is a
different, still-unimplemented piece of work.

### Authoring rules behind the generation prompt

Source: `mattpocock/skills`, `skills/productivity/writing-great-skills/SKILL.md`
(read 2026-07-31). Thesis: a skill wrangles determinism out of a stochastic
system, and **predictability** — the agent taking the same *process* every
run, not producing the same output — is the root virtue. Levers:

- **Invocation is a two-way choice with a cost on each side.** Model-invoked
  keeps a trigger-bearing `description` and pays *context load* every turn;
  user-invoked sets `disable-model-invocation: true`, pays zero context load,
  and spends *cognitive load* — the human becomes the index. Model-invocation
  is warranted only when the agent or another skill must reach it unaided.
- **The description carries triggers, not identity.** Front-load the leading
  word; one trigger per **branch**; synonyms renaming one branch are
  duplication; cut what the body already says.
- **Information hierarchy**, three rungs: in-skill *step* (ordered, ending on a
  checkable and often exhaustive **completion criterion**), in-skill
  *reference* (on demand; a flat peer-set is legitimate), *external reference*
  behind a **context pointer** whose wording — not its target — decides how
  reliably the agent reaches it. Branching is the disclosure test: inline what
  every branch needs, disclose what only some reach.
- **Split only when the cut earns it**: by invocation (a distinct leading word
  deserving its own trigger) or by sequence (hiding post-completion steps that
  tempt the agent to rush the current one).
- **Pruning**: single source of truth, relevance, and a sentence-by-sentence
  **no-op** test — delete the sentence rather than trim it.
- **Leading words**: compact concepts already in pretraining (*tight*, *red*,
  *fog of war*) that collapse a restated triad into one token.
- **Failure modes**: premature completion, duplication, sediment, sprawl,
  no-op, and **negation** (steering by prohibition backfires — prompt the
  positive).

**What is and isn't lintable.** The real source is almost entirely *editorial
judgment*, which a deterministic linter cannot check. (An earlier draft
substituted the local `write-a-skill` skill — a mechanical checklist — and
claimed every requirement was already an adept rule; that was wrong.)

- **Already covered:** sprawl → `SL3xx`; broken context pointers → `SL104`;
  cross-skill duplication → `SL402`/`SL403`.
- **New and genuinely lintable:** *invocation-mode coherence* (#27) is the
  strongest candidate — `disable-model-invocation: true` alongside
  model-facing trigger phrasing ("Use when…") is a flat contradiction,
  mechanically detectable, and readable today with no parser change (unknown
  frontmatter keys land in `Skill::extra`, `parser.rs:156`). Negation is a
  plausible Info-level heuristic. Intra-skill duplication is weaker but
  real — adept only compares *across* skills today (#28).
- **Not lintable, do not fake:** no-ops, leading-word strength, ladder
  placement, completion-criterion sharpness. These belong to the LLM
  surfaces — and that is the actual argument for `create`: the most
  valuable half of authoring guidance is judgment adept can only apply
  while *writing*. A clean `adept check` proves the mechanical tier only.

## Deferred by design

The project's original non-goals, recorded here — this list is now their only
home — so they aren't rediscovered as bugs:

- Hosted registry / marketplace / index.
- ~~Executing or sandbox-testing skill scripts.~~ **Superseded and shipped**
  (upskill item 1). Retired as a *product* boundary, not a safety one — "adept
  spawns no subprocess, ever" (ARCHI §16) replaced it and remains binding.
- ~~Full agent-harness end-to-end evals.~~ **Superseded and shipped** (item 2)
  as an analysis within `adept eval`, not the standalone surface originally
  anticipated. `triggering` keeps its narrower prompt→trigger meaning.
- Non-Anthropic skill formats. The parser trait exists; see the filename
  limitation above before building on it.

Both were original project non-goals, struck once shipped; `docs/EVALS.md` and
ARCHI §10/§16 describe the shipped shape.
`check`/`fmt` remain offline and side-effect-free throughout.

**Also deferred, from the same unification work:**

- **Dataset cases are referenced by 1-indexed line number, which is brittle
  (#29).** `CaseResult::case` names a case by its position in
  `evals/evals.jsonl`. A harness that reorders, filters, or regenerates a subset
  can make a `case` number silently point at the wrong case, undetectably.
  Content-addressed ids would fix it but need a dataset `schema_version` bump.
  Revisit if a harness author reports this biting.
- Run-history storage (#23, upskill item 3) was explicitly out of scope there.

## Tracing & capture follow-ups

Recorded 2026-07-31.

- **Resolved** (`96dc487`): rule snapshots baked the absolute checkout path
  into diagnostic paths, failing every rule-snapshot test in a git worktree.
  `strip_repo_root` in `crates/adept/tests/rules.rs` makes them repo-relative.
- **Captured calls do not record which logical step issued them (#30).** Knowing
  whether a call was prompt generation, a judge sample, or fix round N is what
  makes a capture directory readable without shell history. A
  `CapturedCall::step` field shipped and was removed: `OpenAiCompatClient` sees
  only a request body and could never populate it, so it was structurally
  always `None`. The intended shape is a **`tracing::Span` field set by
  `adept_agent::eval::triggering` / `adept_agent` and read at capture time** —
  the context already lives there and a span carries it down without widening a
  signature. Explicitly *not* a `set_step()` mutator (it would make a `Sync`
  client statefully order-dependent). Until then, infer the step from call
  ordering and prompt content.
- **The MCP `eval_skill` tool never captures**, deliberately (`cli-tracing.md`
  §12): capture is CLI-only, so the tool schema stays unchanged and an MCP
  client cannot make the server write to arbitrary paths.
  `crates/adept_cli/tests/tracing.rs` pins the schema half.
- **Resolved (#31): `LlmError::Status` carried an unscrubbed response body.**
  The capture layer's scrub covered every body reaching a tracing event or
  artifact, but `Status { body }` held the response verbatim — a backend
  echoing the API key in an error body leaked it to stderr via `Display`.
  Fixed by hoisting the scrub to where `send_once` reads the body, rather than
  adding a fourth per-egress scrub: that body fans out to a log event, a
  capture artifact, the parser and this error, and scrubbing per-consumer is
  what let this one ship unscrubbed. Past the read no unscrubbed backend text
  is in scope, so a future fifth consumer is covered by default. The other
  body-bearing variant, `MalformedResponse`, carries only `serde_json`'s
  message, which does not embed the input.
- **Resolved: a credential-bearing `base_url` leaked userinfo via three
  egresses.** `LlmError::Request(e.to_string())` embeds reqwest's URL
  serialization (including `user:password@`) via `Display`, reaching stderr
  and a `tracing` event; `RunMetadata.base_url` wrote the same value verbatim
  into capture artifacts on disk. Per-egress sanitization was considered and
  rejected for the same reason as the fix for `LlmError::Status` above (#31):
  it is the exact shape that let that one ship unscrubbed, and it cannot cover
  URL logging performed by `reqwest` or a
  transitive dependency, which is not adept's code to scrub. Fixed by
  rejecting credentials at `LlmConfig::resolve` instead — a `base_url` whose
  parsed `username()`/`password()` is non-empty now returns
  `ConfigError::CredentialsInBaseUrl` before it can reach any egress, making
  `resolved.base_url` credential-free by construction rather than sanitized at
  each of the three sites. `LlmError` also gained `#[non_exhaustive]` so it can
  no longer be constructed or exhaustively matched from outside `adept-agent`.
  Breaking on both counts: a previously-working `base_url` now exits 2, and
  external code matching on `LlmError` needs a wildcard arm.

## Pre-publish checklist

- **Decide the reference-link behaviour above (#5).** It changes observable
  output, so it is cheaper to settle before there are users.
- **`eval`'s LLM analyses and `fix` have never run against a live endpoint
  (#32).** Testing is mock-only by design; one manual run of each against a
  real OpenAI-compatible endpoint would confirm the request shape. `fix`
  matters more: it *writes*, and its accept/reject gate has only seen
  hand-shaped `MockLlmClient` responses — a real model's JSON (fenced,
  truncated, or ignoring the companion-edit contract) is the untested input
  class. Run it in the default preview mode first, not `--write`.
- **Delete the `dry_run` line in `release.yml`'s `release` job, don't flip it
  to `false` (#35).** `release-plz/action@v0.5.131` `action.yml:115-119` guards
  the flag on string emptiness, not truthiness — `if [[ -n "${{ inputs.dry_run
  }}" ]]` — and the input has no `default:` (`action.yml:40-47`), so
  `with: dry_run: false` renders as the non-empty string `"false"` and
  `--dry-run` stays in effect. The failure shape is the bad one: the "go live"
  commit merges, the job goes green, nothing publishes, and there's no signal
  until someone notices crates.io is empty. This is the last gate before the
  one-way door. Nothing named `adept*` exists on crates.io and
  `[workspace.package].version` is already `0.1.0`, so a live `release` job
  publishes all four on the very first push to `main`. Merge with
  `dry_run: true` first, read the job log to confirm it names exactly the four
  crates in dependency order, then delete the line in its own commit.
  `crates/adept/tests/workspace_metadata.rs` fails the build if the workflow
  ever sets `dry_run: false`, so the wrong edit cannot reach `main` silently.
- ~~**Both release secrets must exist before the first merge to `main`
  (#35).**~~ Done: `CARGO_REGISTRY_TOKEN` and `RELEASE_PLZ_TOKEN` are both set
  on the repo. The PAT was blocking, not a nicety — `release.yml`'s
  `release-pr` job has no fallback and errors without it; the `release` job
  succeeds independently on the plain `GITHUB_TOKEN` since the two jobs run in
  parallel, so a missing PAT would only fail the PR job, silently, next to a
  green publish.
- ~~**Confirm all four names are free on crates.io (#35).**~~ Done: `adept`,
  `adept-core`, `adept-fmt`, and `adept-agent` all return `does not exist` from
  the crates.io API. Category slugs are still validated server-side, so a wrong
  one is rejected at the worst moment.
- ~~**Confirm `persist-credentials: false` suits the release-plz action's
  git-write auth (#35).**~~ Done: release-plz never uses ambient git
  credentials. The action passes `--git-token "${GITHUB_TOKEN}"`
  (`action.yml:167,194`), and the `release-plz/git-config` action it runs
  first sets only `user.name`/`user.email`, no credential helper. Upstream's
  own quickstart workflow uses exactly `fetch-depth: 0` +
  `persist-credentials: false`.
