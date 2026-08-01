# Eval-dataset schema and grading (`evals/evals.jsonl` + `results.jsonl`)

A published contract, machine-checked against `crates/adept/src/evals.rs` by
`crates/adept/tests/docs_test.rs` — the build fails if this document and the
code disagree about the set of assertion kinds.

**adept grades an eval dataset; it never runs one.** It defines the schema
below and is the reference *grader*: `adept eval <skill> --results
results.jsonl` (or the MCP `eval_skill` tool's `results` argument) reads a
harness-produced sidecar and reports pass rate, assertion success, and skill
lift. *Running* a case — invoking the skill with each `prompt` and capturing
what happened — stays a separate harness's job. `adept::evals::grade` is a
substring match, two filesystem reads, and a lookup into a harness-supplied
exit-code map: no subprocess, no model call, no agent. This document exists so
two independent harnesses can hand adept an equivalent `results.jsonl` and get
an identical report.

## File layout

An eval dataset is a `.jsonl` file, conventionally `evals/evals.jsonl` inside
a skill directory (where `adept create` writes it). It is **JSONL, not a single
JSON document**: one object per line, no enclosing array, no wrapper. That lets
a dataset be streamed, appended to, and diffed one case at a time, and travel
with the skill directory it was generated for.

Because there is no envelope for document-level metadata, **every line repeats
its own `schema_version`** — intentionally redundant, so a truncated,
concatenated, or hand-appended file stays self-describing.

Blank lines are permitted and skipped; every non-blank line must parse as a
case object.

## The case object

```json
{"schema_version": 1, "prompt": "Summarize the attached report.", "assertions": [{"kind": "contains", "value": "summary"}, {"kind": "file_exists", "path": "out/summary.md"}, {"kind": "file_contains", "path": "out/summary.md", "value": "conclusion"}, {"kind": "command", "command": "test -s out/summary.md"}]}
```

| Field | Type | Meaning |
|---|---|---|
| `schema_version` | integer | Schema version this line was written against. Current: `1` (`adept::evals::SCHEMA_VERSION`). |
| `prompt` | string | The prompt the skill under test should handle. |
| `assertions` | array | Deterministic checks run against the response. May be empty. |

`schema_version` is **not** the same axis as adept's prompt versioning
(`adept_agent::eval::prompts::PROMPT_VERSION`; see ARCHI §10). Prompt wording
drifts as generation is tuned; that changes a dataset's *content*, not its
*shape*, and must never look like a breaking change to a harness.
`schema_version` changes only when the case object's shape does — a field
renamed, an assertion kind's fields changed, a case-level field added or
removed. Rare and loud, the same discipline adept applies to rule codes. A
harness should refuse an unrecognized `schema_version` rather than guess.

## Assertion kinds

Exactly four, taken from
[huggingface/upskill](https://github.com/huggingface/upskill)'s graders. Every
assertion object carries a `kind` field (Rust: `#[serde(tag = "kind",
rename_all = "snake_case")]` on `adept::evals::Assertion`). An unrecognized
`kind` is a validation error, never silently ignored — that is what lets the
vocabulary grow a fifth kind without an old reader misreading a new file.

### `contains`

```json
{"kind": "contains", "value": "summary"}
```

Passes if the harness-produced output (the response text, as the harness
defines "output") contains `value` as a plain substring. No normalization, no
regex, no case-folding.

### `file_exists`

```json
{"kind": "file_exists", "path": "out/summary.md"}
```

Passes if a file at `path` exists after the run. `path` is relative to the
working directory the harness used for that case; adept does not define that
directory, so a dataset's `path` values are only meaningful in the context of
the harness that produced them.

### `file_contains`

```json
{"kind": "file_contains", "path": "out/summary.md", "value": "conclusion"}
```

Passes if the file exists *and* contains `value` as a plain substring. A
missing file fails the assertion; it does not error separately.

### `command`

```json
{"kind": "command", "command": "test -s out/summary.md"}
```

Passes iff running `command` as a shell command **exits `0`**. adept never runs
it, so the contract has to be stated precisely for two harnesses to grade
identically:

- **Only the exit code decides.** stdout/stderr are not inspected as part of
  grading; a command that prints a diagnostic but exits `0` still passes.
- **Working directory** is the same one that case's outputs landed in — the
  directory `file_exists`/`file_contains` paths resolve against. A dataset
  needing a different one encodes a `cd` into `command` itself; adept-produced
  datasets keep `command` self-contained for this reason.
- **Shell**: run through `sh -c` or equivalent — a command string, not an argv
  array, so pipes, globs, and builtins like `test` all work.

adept validates that `command` is a non-empty string.

## Validation

`adept::evals::validate` checks, in order:

1. every non-blank line parses as a case object (an unknown assertion `kind`
   surfaces here, naming the offending line);
2. every `schema_version` is one this build understands;
3. the dataset is non-empty.

It does not check whether assertions are *satisfiable* — that requires running
them, which is the part adept does not do.

## Grading (`adept eval --results results.jsonl`)

Once a harness has run every case — with the skill, and optionally again
without it as a baseline — it hands adept a `results.jsonl` sidecar.
`adept::evals::grade(cases, results) -> EvalBenchmarkReport` is the function
underneath both the CLI and the MCP tool, and is pure and offline. Add
`--select evals` to skip the three LLM analyses and their `ADEPT_MODEL`
requirement entirely.

### The `results.jsonl` sidecar

A **separate, separately-versioned format** from the dataset: not an
`EvalCase`, and it carries no `schema_version`, since a harness produces it
fresh each run rather than authoring and maintaining it. Adding a field to it
does not touch `adept::evals::SCHEMA_VERSION`, which governs the dataset shape
only. Also JSONL: one `adept::evals::CaseResult` per line, blank lines skipped.

| Field | Required | Meaning |
| --- | --- | --- |
| `case` | yes | Which dataset case this is for — the 1-indexed line number in `evals/evals.jsonl`. |
| `arm` | no | `"skill"` (default) or `"baseline"`. The baseline arm is what makes skill lift computable; a file that never mentions arms is graded as all-`skill`. |
| `response` | yes | The agent's response text, graded by `contains`. |
| `cwd` | no | Working directory the case ran in. `file_exists`/`file_contains` resolve against it; absent means those are `skipped`. |
| `command_exit_codes` | no | Map of command string to observed exit code. A `command` assertion with no entry is `skipped`. |
| `tokens` | no | `{"in": N, "out": N}`; aggregated across results when present. |

### Division of labour

It follows from "adept spawns no subprocess, ever": only `command` needs one,
so only `command` is the harness's to grade.

| Assertion | Graded by | Why |
| --- | --- | --- |
| `contains` | **adept** | substring match on the supplied `response` |
| `file_exists` | **adept** | filesystem read, relative to the supplied `cwd` |
| `file_contains` | **adept** | filesystem read plus substring match |
| `command` | **harness** | needs a subprocess; the harness reports the exit code via `command_exit_codes` |

`file_exists`/`file_contains` resolve strictly inside `cwd` — a `path` escaping
it (`../`, an absolute path) is a graded failure, not a panic, reusing the same
companion-path sandboxing `adept_agent` uses elsewhere.

### Metrics

`EvalBenchmarkReport` (in the spirit of upskill's `upskill eval`) reports:

- **Pass rate** — fraction of `skill`-arm results that passed.
- **Assertion success rate** — assertions met over assertions *checked*.
  Skipped assertions leave both numerator and denominator, and are reported
  separately by reason.
- **Skill lift**, in percentage points — `pass_rate - baseline_pass_rate`,
  present only when at least one `baseline`-arm result was graded. *Omitted*
  rather than zero when there is no baseline: a skill evaluated alone has no
  counterfactual.
- **Token usage** — summed input/output across results reporting `tokens`.
- **Out-of-range and unmatched cases** — a result naming a `case` outside the
  dataset, or a dataset case with no `skill`-arm result, is reported
  explicitly (`out_of_range_results` / `unmatched_cases`), never silently
  ignored.

### Skip semantics

Every assertion outcome is `pass` / `fail` / `skipped`. **A skipped assertion
is never a pass** and is excluded from the success denominator, so a run that
silently graded nothing cannot look perfect. A case passes only if every
non-skipped assertion passed **and** at least one assertion was actually
checked — a case whose assertions were all skipped (no `cwd` supplied for
`file_exists`-only assertions) is not reported as passing. `skipped_reasons`
counts skips by reason, so a harness that forgot `cwd` or `command_exit_codes`
surfaces as a visible anomaly instead of an inflated pass rate.
