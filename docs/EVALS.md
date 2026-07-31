# Eval-dataset schema and grading (`evals/evals.jsonl` + `results.jsonl`)

This document is a published contract, machine-checked against
`crates/adept/src/evals.rs` the same way `docs/RULES.md` is machine-checked
against the rule registry (`crates/adept/tests/docs_test.rs`): a test fails
the build if this document and the code disagree about the set of assertion
kinds.

**adept never executes an eval dataset — it grades one.** It defines the
dataset schema (below), and it is the reference *grader*: `adept eval
<skill> --results results.jsonl` (and the MCP `eval_skill` tool's `results`
argument) reads a harness-produced results sidecar and reports pass rate,
assertion success, and skill lift. What adept still never does is *run* a
case — invoking a skill with each `prompt`, capturing what happened, and
handing adept the result — that stays a separate harness's job. The
distinction that matters: **adept grades results it is given; it does not
produce them.** `adept::evals::grade` is a substring match, two filesystem
reads, and a lookup into a harness-supplied exit-code map — nothing that
requires a subprocess, a model call, or driving an agent. This document
exists so two independent harnesses can hand adept an equivalent
`results.jsonl` and get an identical report, and so a harness author knows
exactly which assertion kinds it must resolve itself before calling adept.

## File layout

An eval dataset is a `.jsonl` file, conventionally `evals/evals.jsonl` inside
a skill directory (this is where `adept create` writes it). It is **JSONL,
not a single JSON document**: one JSON object per line, no enclosing array
and no wrapper object. This is deliberate — it lets a dataset be streamed,
appended to, and diffed one case at a time, and it lets a dataset travel with
the skill directory it was generated for.

Because there is no envelope to carry document-level metadata, **every line
repeats its own `schema_version`**. This is intentionally redundant: it is
what keeps a truncated, concatenated, or hand-appended file self-describing,
since there is no header line a reader could otherwise consult.

Blank lines are permitted and skipped; every non-blank line must parse as one
case object.

## The case object

```json
{"schema_version": 1, "prompt": "Summarize the attached report.", "assertions": [{"kind": "contains", "value": "summary"}, {"kind": "file_exists", "path": "out/summary.md"}, {"kind": "file_contains", "path": "out/summary.md", "value": "conclusion"}, {"kind": "command", "command": "test -s out/summary.md"}]}
```

| Field | Type | Meaning |
|---|---|---|
| `schema_version` | integer | The schema version this line was written against. Adept's current version is `1` (`adept::evals::SCHEMA_VERSION`). |
| `prompt` | string | The prompt the skill under test should handle. |
| `assertions` | array of assertion objects | The deterministic checks a harness runs against the response. May be empty for an individual case. |

## `schema_version` is independent of prompt versioning

`schema_version` is **not** the same axis as `adept_agent::eval::prompts::PROMPT_VERSION`
(unmoved by the `adept_score` → `adept_agent` crate merge, still the literal
string `"adept_score-prompts-v1"` — deliberately, so old and new reports
compare identically) or any other `adept_agent` prompt version. Prompt
wording drifts routinely as
generation is tuned — that drift changes what a dataset's *content* looks
like, not its *shape*, and must never look like a breaking change to a
harness reading the file. `schema_version` changes only when the shape of a
case object changes: a field renamed, an assertion kind's fields changed,
a case-level field added or removed. Such changes should be rare and
loud, the same discipline adept applies to lint rule codes (never reused).

A harness should refuse (or explicitly downgrade-handle) a `schema_version`
it does not recognize rather than guess at the shape.

## Assertion kinds

The assertion vocabulary is exactly four kinds, taken from
[huggingface/upskill](https://github.com/huggingface/upskill)'s graders.
Every assertion object carries a `kind` field selecting which of the four it
is (Rust: `#[serde(tag = "kind", rename_all = "snake_case")]` on
`adept::evals::Assertion`). An unrecognized `kind` is a validation error, not
something adept or a well-behaved harness should silently ignore — this is
what lets the vocabulary grow a fifth kind later without an old reader
misinterpreting a new file.

### `contains`

```json
{"kind": "contains", "value": "summary"}
```

Passes if the harness-produced output (the skill's response text, as the
harness defines "output" for its own runner) contains `value` as a plain
substring. No normalization, no regex, no case-folding — a literal substring
check.

### `file_exists`

```json
{"kind": "file_exists", "path": "out/summary.md"}
```

Passes if a file at `path` exists after the run. `path` is relative to
whatever working directory or output directory the harness uses for the
run (see `command`, below, for the same question). adept does not define
that directory — it is a harness concern — so a dataset's `path` values are
only meaningful in the context of the harness that produced them.

### `file_contains`

```json
{"kind": "file_contains", "path": "out/summary.md", "value": "conclusion"}
```

Passes if a file at `path` exists *and* its contents contain `value` as a
plain substring (same substring semantics as `contains`). A missing file
fails this assertion; it does not error separately from `file_exists`.

### `command`

```json
{"kind": "command", "command": "test -s out/summary.md"}
```

Passes if running `command` as a shell command **exits with status `0`**;
any non-zero exit status is a failure. This is the exit-code-only contract,
stated precisely because adept itself never runs this command — the
following must hold for two harnesses to grade the same dataset identically:

- **Only the exit code decides pass/fail.** stdout and stderr are not
  inspected by the assertion itself (a harness may still capture and log
  them for debugging, but that is not part of grading).
- **Working directory**: the same directory the harness used as the output
  location for that case's run (the same directory `file_exists` /
  `file_contains` paths are relative to). A dataset author who needs a
  different working directory encodes a `cd` into `command` itself.
  adept-produced datasets keep `command` self-contained for this reason —
  never assume an ambient directory beyond "wherever this case's outputs
  landed."
- **Shell**: the command is run through a shell (`sh -c` or equivalent); it
  is a shell command string, not an argv array, so it may use pipes,
  globs, and shell builtins like `test`.
- **Nothing beyond exit code is captured as part of the grade.** A command
  that prints a helpful diagnostic to stderr but exits `0` still passes; one
  that exits non-zero fails regardless of what it printed.

adept validates that `command` is a non-empty string. It never invokes it —
no test in this repository spawns a subprocess to grade a `command`
assertion, and no adept binary does either.

## Validation

`adept::evals::validate` checks, in order:

1. every non-blank line parses as a case object (an unknown assertion `kind`
   surfaces here, as a parse error naming the offending line);
2. every case's `schema_version` is one this build of adept understands;
3. the dataset is non-empty (at least one case).

Validation does not check whether assertions are *satisfiable* against any
particular skill — that can only be known by running them, which is exactly
the part adept does not do.

## Grading (`adept eval --results results.jsonl`)

Once a harness has run every case in a dataset — with the skill available,
and optionally again without it as a baseline — it hands adept a
`results.jsonl` sidecar and adept grades it: `adept eval <skill> --results
results.jsonl` (offline; add `--select evals` to skip the three LLM
analyses and their `ADEPT_MODEL` requirement entirely), or the MCP
`eval_skill` tool's inline `results` argument. `adept::evals::grade(cases,
results) -> EvalBenchmarkReport` is the function underneath both, and it is
pure and offline: no network, no subprocess, no LLM.

### The `results.jsonl` sidecar

`results.jsonl` is a **separate, separately-versioned format** from the
eval dataset above — it is not an `EvalCase` and carries no
`schema_version` of its own, since a harness produces it fresh for every
run rather than authoring and maintaining it like a dataset. Adding a field
to it does not touch `adept::evals::SCHEMA_VERSION`, which governs the
dataset shape only. Like the dataset, it is JSONL: one JSON object
(`adept::evals::CaseResult`) per line, blank lines skipped.

| Field | Required | Meaning |
| --- | --- | --- |
| `case` | yes | Which dataset case this result is for — the 1-indexed line number in `evals/evals.jsonl`. |
| `arm` | no | `"skill"` (default) or `"baseline"`. The baseline arm is what makes skill lift computable; a results file that never mentions arms just works and is graded as all-`skill`. |
| `response` | yes | The agent's response text, graded by the `contains` assertion. |
| `cwd` | no | Working directory the case ran in. `file_exists`/`file_contains` paths resolve against it; absent means those assertions are graded `skipped`. |
| `command_exit_codes` | no | Map of command string to observed exit code, as reported by the harness (adept never runs a `command` assertion itself). A `command` assertion with no matching entry is `skipped`. |
| `tokens` | no | `{"in": N, "out": N}`; aggregated across results when present. |

### Division of labour, per assertion kind

The division follows directly from "adept spawns no subprocess, ever":
only `command` needs one, so only `command` is the harness's to grade.

| Assertion | Graded by | Why |
| --- | --- | --- |
| `contains` | **adept** | a substring match on the harness-supplied `response` text |
| `file_exists` | **adept** | a filesystem read, relative to the harness-supplied `cwd` |
| `file_contains` | **adept** | a filesystem read plus a substring match |
| `command` | **harness** | requires spawning a subprocess; the harness runs it and reports the exit code back via `command_exit_codes` |

`file_exists`/`file_contains` resolve strictly inside `cwd` — a `path`
escaping it (`../`, an absolute path) is a graded failure, not a panic,
reusing the same companion-path sandboxing `adept_agent` uses elsewhere
rather than a second implementation.

### Metrics

`EvalBenchmarkReport` (in the spirit of huggingface/upskill's `upskill
eval`) reports:

- **Pass rate** — fraction of `skill`-arm results that passed.
- **Assertion success rate** — assertions met divided by assertions
  *checked* (skipped assertions are excluded from both numerator and
  denominator, and reported separately by reason, e.g. `command` with no
  exit code, `file_*` with no `cwd`).
- **Skill lift**, in percentage points — `pass_rate - baseline_pass_rate`,
  present only when at least one `baseline`-arm result was graded. Lift is
  *omitted*, not reported as zero, when there is no baseline arm — a skill
  evaluated alone has no counterfactual to compare against.
- **Token usage** — summed input/output tokens across results that
  reported `tokens`.
- **Out-of-range and unmatched cases** — a result naming a `case` outside
  the dataset, or a dataset case with no `skill`-arm result at all, is
  reported explicitly (`out_of_range_results` / `unmatched_cases`) rather
  than silently ignored.

### Skip semantics

Every assertion outcome is `pass` / `fail` / `skipped`. **A skipped
assertion is never counted as a pass**, and is excluded from the
assertion-success denominator — a run that silently graded nothing
therefore cannot look like a perfect score. A case is reported `pass` only
if every non-skipped assertion passed **and** at least one assertion was
actually checked; a case whose assertions were all skipped (e.g. no `cwd`
supplied for `file_exists`-only assertions) is not reported as passing.
`skipped_reasons` on the report counts skips by reason, so a harness that
forgot to supply `cwd` or `command_exit_codes` shows up as a visible
anomaly instead of an inflated pass rate.
