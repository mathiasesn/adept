<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/mathiasesn/adept/main/assets/logo/adept-logo-white.svg">
    <img src="https://raw.githubusercontent.com/mathiasesn/adept/main/assets/logo/adept-logo.svg" alt="adept" width="320">
  </picture>
</p>

<p align="center">
  <a href="https://github.com/mathiasesn/adept"><img src="https://img.shields.io/badge/adept-checked-000000?logo=data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxNiIgaGVpZ2h0PSIxNiIgdmlld0JveD0iMCAwIDE2IDE2IiBzaGFwZS1yZW5kZXJpbmc9ImNyaXNwRWRnZXMiPjxwYXRoIGQ9Ik00IDNoOHYxaC04eiBNNCA0aDh2MWgtOHogTTEwIDVoMnYxaC0yeiBNMTAgNmgydjFoLTJ6IE00IDdoOHYxaC04eiBNNCA4aDh2MWgtOHogTTQgOWgydjFoLTJ6IE0xMCA5aDJ2MWgtMnogTTQgMTBoMnYxaC0yeiBNMTAgMTBoMnYxaC0yeiBNNCAxMWg4djFoLTh6IE00IDEyaDh2MWgtOHoiIGZpbGw9IiNmZmZmZmYiLz48L3N2Zz4=" alt="checked with adept"></a>
  <a href="https://crates.io/crates/adept"><img src="https://img.shields.io/crates/v/adept?logo=rust&logoColor=white&label=crates.io&color=E05D44" alt="crates.io"></a>
  <a href="https://github.com/mathiasesn/adept/blob/main/LICENSE"><img src="https://img.shields.io/github/license/mathiasesn/adept?label=license&color=44CC11" alt="license"></a>
  <img src="https://img.shields.io/badge/rust-1.85%2B-blue?logo=rust&logoColor=white&label=MSRV" alt="MSRV 1.85+">
  <a href="https://github.com/mathiasesn/adept/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/mathiasesn/adept/ci.yml?branch=main&logo=github&label=CI" alt="CI"></a>
</p>

An extremely fast linter and formatter for Agent Skills.

`adept` checks the folder-of-instructions "Agent Skill" pattern (a
`SKILL.md` file plus optional companion files) for the defects that make
skills fail to trigger, over-trigger, or bloat an agent's context: vague
descriptions, malformed frontmatter, token budget overruns, broken file
references, and conflicting/overlapping skills. It ships as a single Rust
binary with six surfaces:

- `adept check` — static, offline lint with ruff-style diagnostics.
- `adept fmt` — prettier-style formatting of `SKILL.md` (frontmatter +
  full Markdown reflow).
- `adept eval` — one evaluation command, four analyses: LLM-assisted
  triggering accuracy, token bloat, and cross-skill overlap detection,
  plus offline eval-dataset grading (pass rate, assertion success, skill
  lift) against a harness-supplied `results.jsonl`.
- `adept fix` — LLM-assisted autofix for the diagnostics that need
  rewriting rather than a mechanical transform.
- `adept create` — LLM-assisted skill generation from a written brief:
  generate → lint → repair, plus a synthetic eval dataset.
- `adept mcp` — an MCP server (stdio) so agents can lint/format/evaluate
  skills themselves.

`check`, `fmt`, and eval-dataset grading never touch the network; `eval`'s
other three analyses, `fix`, and `create` do.

## Install

```bash
cargo install adept
```

Or, from source (`adept` is the package name of `crates/adept_cli`):

```bash
cargo install --path crates/adept_cli
# or, to build and run in place without installing to PATH:
cargo build --release -p adept
./target/release/adept --help
```

## `adept check`

Lints one or more `SKILL.md` files or directories of skills.

```console
$ adept check crates/adept_cli/tests/fixtures/defective-skill
crates/adept_cli/tests/fixtures/defective-skill/SKILL.md:3:1: SL201 description is only 1 tokens, below the minimum of 6
  fix: expand the description to state both what the skill does and when to use it
crates/adept_cli/tests/fixtures/defective-skill/SKILL.md:3:1: SL203 description does not state when the skill should be used
  fix: add trigger phrasing, e.g. "Use when the user asks to..."
crates/adept_cli/tests/fixtures/defective-skill/SKILL.md:3:1: SL206 description gives no guidance on when not to use the skill
  fix: consider adding "Do not use for..." guidance to reduce over-triggering
crates/adept_cli/tests/fixtures/defective-skill/SKILL.md:5:1: SL102 SKILL.md body has no top-level `#` heading
  fix: add a single `# Title` heading near the top of the body

Found 4 problems (0 errors, 3 warnings, 1 info)
```

Flags:

- `--format human|json` — output format (default `human`).
- `--select CODE,...` / `--ignore CODE,...` — enable only, or disable,
  specific rules by code (`SL201`) or kebab-case name
  (`description-too-short`); repeatable or comma-separated.
- `--statistics` — print per-rule diagnostic counts.
- `--exit-zero` — always exit `0`, even if diagnostics were found.
- `--tokenizer o200k-base|cl100k-base` — which `tiktoken-rs` BPE encoding
  to count tokens with (default `o200k-base`; overrides the config file's
  `[lint] tokenizer`).

**Exit codes**: `0` = no diagnostics found (or `--exit-zero`), `1` =
diagnostics found, `2` = a usage or I/O error (bad path, unreadable file,
bad config).

## `adept fmt`

Formats `SKILL.md` files in place: canonical frontmatter (key order,
minimal quoting) plus a full Markdown body reflow.

```console
$ adept fmt path/to/skill --check
--- original
+++ formatted
@@ -1,7 +1,8 @@
 ---
+name: pdf-extractor
 description: Extract text and tables from PDF files. Use this when the user asks to read, parse, or extract data from a PDF.
-name: pdf-extractor
 ---
+
 # PDF Extractor

-Use the bundled script to extract   content.
+Use the bundled script to extract content.
$ echo $?
1

$ adept fmt path/to/skill
1 file reformatted, 0 files unchanged
```

Flags:

- `--check` — don't write anything; exit `1` if any file would change and
  print a unified diff.
- `--diff` — print the unified diff without writing (exits `0`).
- `--line-width <n>` — target line width for prose reflow (default `100`).

Formatting is idempotent (`fmt(fmt(x)) == fmt(x)`) and writes are atomic
(temp file + rename), so a formatting error never clobbers the original
file.

## `adept eval`

Four analyses under one command, named `triggering`, `token-bloat`,
`overlap`, and `evals` — the first three LLM-assisted, the fourth offline.
`path` accepts either a `SKILL.md` file or a skill directory.

```console
$ adept eval path/to/skill/SKILL.md
Eval report for skill: pdf-extractor
(prompt set version: adept_score-prompts-v1)

== Triggering accuracy ==
precision: 1.00  recall: 0.90  f1: 0.95  (9/10 correct)
  [OK] (should-trigger, agreement 100%) predicted=true :: Fill out this W-9 PDF for me
  ...

== Token bloat ==
description: 24 tokens, body: 340 tokens, companions: 0 tokens, total: 364 tokens
  no trimming suggestions

== Overlap/conflict detection ==
  no shortlisted overlaps
```

The `triggering`, `token-bloat`, and `overlap` analyses talk to any
OpenAI-compatible `/chat/completions` endpoint (OpenAI itself, local
servers like Ollama/vLLM, or Anthropic via its OpenAI-compatibility
layer), configured via environment variables or flags:

| Env var           | Flag          | Purpose                                  |
| ------------------ | ------------- | ----------------------------------------- |
| `ADEPT_MODEL`       | `--model`     | Model identifier to request.              |
| `ADEPT_BASE_URL`    | `--base-url`  | Base URL, default `https://api.openai.com/v1`. |
| `ADEPT_API_KEY`     | *(none)*      | Bearer token, if the endpoint needs one.  |

Also: `--num-prompts`, `--seed`, `--judge-samples` (triggering),
`--format human|json`, `--tokenizer o200k-base|cl100k-base` (default
`o200k-base`; overrides the config file's `[eval] tokenizer`) for the
token-bloat analysis, and `--capture-dir <DIR>` (see [Logging and
capture](#logging-and-capture)).

The fourth analysis, **`evals`**, needs no model: pass `--results
<results.jsonl>` (a harness-produced sidecar — see
[`docs/EVALS.md`](docs/EVALS.md) for its exact fields) and it grades the
skill's dataset (`evals/evals.jsonl` by default, or `--evals <path>` to
override), reporting pass rate, assertion success, and skill lift. adept
grades a dataset but never executes it — running the cases is the
harness's job.

```console
$ adept eval path/to/skill --evals evals.jsonl --results results.jsonl --select evals
Eval report for skill: pdf-extractor

== Eval-dataset grading ==
pass rate: 100% (2 cases)
assertions: 3/3 met (0 skipped)
```

`--select`/`--ignore` (comma-separated or repeated) restrict which of the
four analyses run, by the names above. Without them, `adept eval` runs
whatever it can: `evals` when
`--results` is supplied, the three LLM analyses when a model is
configured. `--select evals` with no `--results`, or `--select triggering`
with no model, is a usage error naming what's missing rather than a silent
skip — and `--select evals` never constructs an LLM client or touches the
network, even with no `ADEPT_MODEL` set.

If an LLM analysis is selected (explicitly, or by the default "whatever's
available" rule) but no model can be resolved, `adept eval` exits `2` with
an actionable message instead of making a network call:

```console
$ adept eval path/to/skill/SKILL.md
adept: error: nothing to evaluate: no model configured (--model/ADEPT_MODEL/[eval] model) and no --results supplied
```

## `adept fix`

LLM-assisted autofix for the lint diagnostics that need rewriting rather
than a mechanical transform (`SL206 no-negative-guidance`, `SL301
description-tokens-over-budget`, `SL302 body-tokens-over-budget`).
**Preview by default** — it never touches disk unless you pass `--write`:

```console
$ adept fix path/to/skill/SKILL.md
adept fix: pdf-filler
1 round used
  resolved  SL302 SKILL.md body is 1842 tokens, over the budget of 1500
accepted

--- SKILL.md
+++ SKILL.md
...
```

| Flag | Purpose |
| ------------- | ----------------------------------------------------------- |
| `--write`     | Apply pending changes to disk (atomic, all-or-nothing per skill). |
| `--check`     | Exit `1` if any skill has pending changes; prints the diff, like `fmt --check`. |
| `--diff`      | Print only the unified diff, not the full report.            |
| `--select` / `--ignore` | Restrict which rule codes/names are attempted, same as `check`. |
| `--max-rounds <n>` | Bound the fix/re-lint retry loop (default `2`).          |
| `--model <M>` / `--base-url <U>` | LLM overrides, resolved against `[fix]`, not `[eval]`. |
| `--capture-dir <DIR>` | Save the raw request/response of every LLM call (see below). |

Uses the same `ADEPT_MODEL` / `ADEPT_BASE_URL` / `ADEPT_API_KEY`
environment variables and `--model`/`--base-url` flags as `adept eval`,
but resolved against the independent `[fix]` config section (see
Configuration below) — `adept fix` can point at a different model than
`adept eval`. If no model can be resolved, it exits `2` with the same
kind of actionable message `adept eval` gives.

A fix candidate for `SL302` is rejected (even if it clears the diagnostic)
unless it *relocates* content into companion files rather than deleting
it — the token-conservation guard in `adept_agent::relocate`.

## `adept create`

LLM-assisted skill generation from a written brief: one call generates a
candidate skill, it's screened by inserting it into the linter alongside any
sibling skills, and a bounded repair loop feeds diagnostics back until the
candidate clears zero `Error`/`Warning` findings (`Info` findings don't
block). A second call generates a synthetic eval dataset
(`evals/evals.jsonl`) for the accepted skill — see
[`docs/EVALS.md`](docs/EVALS.md) for the dataset schema. **Preview by
default**, like `fix` — nothing is written unless you pass `--write`:

```console
$ adept create --from-file brief.md --out skills/pdf-filler --write
adept create: pdf-filler
2 rounds used
0 diagnostics remaining
3 eval case(s) generated
  - Fill a W-9 form with the provided name and SSN [file_exists, file_contains]
  - Reject a PDF that is not a recognized form [contains]
  - Fill a form missing an optional field [file_exists]
<diff of the generated SKILL.md and evals/evals.jsonl>
wrote 2 files to skills/pdf-filler
```

| Flag | Purpose |
| ------------- | ----------------------------------------------------------- |
| `--from-file <path>` | Read the task brief from a file (else non-TTY stdin, else an interactive prompt). |
| `--out <dir>` | Destination directory for the new skill (default: current directory). |
| `--name` | Override the skill name the model derives from the brief. |
| `--write` / `-w` | Write the generated skill and eval dataset to disk. |
| `--overwrite` | Allow writing into a directory that already has a `SKILL.md`. |
| `--max-rounds <n>` | Bound the generate/repair loop (default `2`). |
| `--model <M>` / `--base-url <U>` | LLM overrides, resolved against `[create]`, independent of `[eval]`/`[fix]`. |
| `--capture-dir <DIR>` | Save the raw request/response of every LLM call (see below). |
| `--format json` | Machine-readable output: generated files, remaining diagnostics, dataset. |

A run that exhausts `--max-rounds` still writes/prints the best candidate
seen, reports every remaining diagnostic, and exits `1` — it never leaves
you empty-handed, but the exit code says the file is not guaranteed clean.

## `adept mcp`

Runs `adept` as an MCP server over stdio, exposing the static/offline
capabilities (`check_skill`, `format_skill`) plus `eval_skill` as tools
for other agents to call. Nothing but JSON-RPC responses is ever written
to stdout; logging goes to stderr.

```console
$ echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | adept mcp
{"jsonrpc":"2.0","id":1,"result":{"tools":[
  {"name":"check_skill", "description":"Lint a SKILL.md file...", "inputSchema":{...}},
  {"name":"format_skill", "description":"Format a SKILL.md file's content...", "inputSchema":{...}},
  {"name":"eval_skill", "description":"Evaluate a skill: triggering accuracy, token bloat, and overlap...", "inputSchema":{...}}
]}}
```

`eval_skill` runs the same four analyses as `adept eval` and is **always
advertised**, even with no `ADEPT_MODEL` set — grading a skill's eval
dataset (via an inline `results` argument, not a file path) needs no model
and no network call. Its `triggering`/`token-bloat`/`overlap` analyses do
need a resolvable model; calling those without one returns a structured
tool error (`isError: true`) rather than hanging or panicking, and
requests are bounded by an internal timeout. Passing `results` alongside
raw `content` (no `path`) grades `contains` only and reports
`file_exists`/`file_contains` as skipped, naming the missing directory —
not as passes and not as an error.

`format_skill`'s `line_width` argument is validated to the range
`20..=500`; out-of-range or zero values are rejected with a structured
tool error instead of silently truncating or producing degenerate
one-word-per-line output.

Two more tools, `create_skill` and `generate_evals`, mirror `adept
create`'s generation and eval-dataset pipeline. They are network-backed
and **conditionally advertised** — only when an LLM backend can actually
be resolved (`ADEPT_MODEL` etc. set, or `model`/`base_url` arguments
passed):

```console
$ echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | ADEPT_MODEL=gpt-4o-mini adept mcp
{"jsonrpc":"2.0","id":1,"result":{"tools":[
  {"name":"check_skill", ...},
  {"name":"format_skill", ...},
  {"name":"eval_skill", ...},
  {"name":"create_skill", ...},
  {"name":"generate_evals", ...}
]}}
```

**Both are preview-only**: they return the generated skill and dataset as
data and never write to disk — writing stays a CLI-only capability (`adept
create --write`). `eval_skill` is read-only for the same reason.

Point any MCP-compatible client at `adept mcp` as a stdio server.

## Configuration

`adept` reads an `adept.toml` file, discovered by walking up from the
target path (or use `--config <path>` to force a specific file):

```toml
[lint]
disabled = ["SL206"]
description_min_tokens = 6
description_max_tokens = 75
body_max_tokens = 1500
tokenizer = "o200k_base"  # or "cl100k_base"

[fmt]
line-width = 100

[eval]
model = "gpt-4o-mini"
base_url = "https://api.openai.com/v1"
tokenizer = "o200k_base"  # or "cl100k_base"
capture_dir = ".adept-capture"   # off by default; gitignore it

[fix]
model = "gpt-4o"
base_url = "https://api.openai.com/v1"
tokenizer = "o200k_base"  # or "cl100k_base"
max_rounds = 2             # falls back to adept_agent::DEFAULT_MAX_ROUNDS
capture_dir = ".adept-capture"   # independent of [eval] capture_dir

[create]
model = "gpt-4o"
base_url = "https://api.openai.com/v1"
tokenizer = "o200k_base"  # or "cl100k_base"
max_rounds = 2             # falls back to adept_agent::DEFAULT_MAX_ROUNDS
eval_cases = 10             # falls back to adept_agent::create::DEFAULT_EVAL_CASES; no CLI flag
capture_dir = ".adept-capture"   # independent of [eval]/[fix] capture_dir
```

`[fix]` and `[create]` are each fully independent of `[eval]` (and of each
other) — set any of them if you want `fix`, `create`, and `eval` to use
different models.

Precedence: CLI flag > config file value > built-in default.

A config file with a leftover `[score]` section (the pre-rename name) is a
hard error naming the fix, rather than being silently parsed and ignored:

```console
$ adept eval path/to/skill --config old-adept.toml
adept: error: old-adept.toml: `[score]` is no longer read; rename it to `[eval]`
```

A relative `capture_dir` in `adept.toml` resolves against the directory
containing that `adept.toml`; a relative `--capture-dir` resolves against
your current directory.

## Logging and capture

`-v` turns on diagnostic logging, which always goes to **stderr** — stdout
stays reserved for results (and, under `adept mcp`, for JSON-RPC only):

```console
$ adept fix path/to/skill/SKILL.md --diff -vv 2> run.log
```

`-v` is info, `-vv` debug (full request and response bodies for every LLM
call), `-vvv` trace. It is a global flag, accepted by every subcommand.
`ADEPT_LOG` overrides it with
[`EnvFilter`](https://docs.rs/tracing-subscriber) directive syntax, e.g.
`ADEPT_LOG=adept_agent::llm::client=trace`. With no `-v` and no `ADEPT_LOG`,
nothing is logged at all.

For anything you need to keep, use `--capture-dir` on `eval`, `fix`, or `create`
instead of scraping the log. Each invocation writes a timestamped folder
holding the verbatim request and response of every LLM call, plus enough
metadata (model, base URL, prompt version, adept version, resolved
options, exit code) to identify the run later:

```console
$ adept fix path/to/skill/SKILL.md --diff --capture-dir ./cap
$ ls cap/2026_07_31_14_22_07/
run_metadata.json  call_0001/  call_0002/
```

Bodies are written on receipt and never truncated, so malformed and
non-2xx responses are captured too. Runs only ever append — a previous
capture is never overwritten. Your API key appears in neither layer.
Captures contain full prompts and model output, so point `capture_dir` at
a gitignored path.

## Rules

See [`docs/RULES.md`](docs/RULES.md) for the full table of rule codes
(`SL001`–`SL403`), what each one flags, and how to fix it.

## Development

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Architecture and design rationale live in
[`docs/ARCHI.md`](docs/ARCHI.md); known gaps and deliberate deferrals in
[`docs/BACKLOG.md`](docs/BACKLOG.md).
