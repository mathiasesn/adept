# adept

An extremely fast linter and formatter for Agent Skills.

`adept` checks the folder-of-instructions "Agent Skill" pattern (a
`SKILL.md` file plus optional companion files) for the defects that make
skills fail to trigger, over-trigger, or bloat an agent's context: vague
descriptions, malformed frontmatter, token budget overruns, broken file
references, and conflicting/overlapping skills. It ships as a single Rust
binary with four surfaces:

- `adept check` — static, offline lint with ruff-style diagnostics.
- `adept fmt` — prettier-style formatting of `SKILL.md` (frontmatter +
  full Markdown reflow).
- `adept score` — LLM-assisted scoring: triggering accuracy, token bloat,
  and cross-skill overlap detection.
- `adept mcp` — an MCP server (stdio) so agents can lint/format skills
  themselves.

## Install

```bash
cargo install --path crates/adept_cli
# or, from within this repo:
cargo build --release -p adept_cli
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

## `adept score`

LLM-assisted scoring of one skill: how reliably its description triggers
the right prompts, whether it's token-bloated, and whether it conflicts
or overlaps with sibling skills.

```console
$ adept score path/to/skill/SKILL.md
Score report for skill: pdf-extractor
(prompt set version: 1)

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

`adept score` talks to any OpenAI-compatible `/chat/completions` endpoint
(OpenAI itself, local servers like Ollama/vLLM, or Anthropic via its
OpenAI-compatibility layer), configured via environment variables or
flags:

| Env var           | Flag          | Purpose                                  |
| ------------------ | ------------- | ----------------------------------------- |
| `ADEPT_MODEL`       | `--model`     | Model identifier to request (required).   |
| `ADEPT_BASE_URL`    | `--base-url`  | Base URL, default `https://api.openai.com/v1`. |
| `ADEPT_API_KEY`     | *(none)*      | Bearer token, if the endpoint needs one.  |

Also: `--num-prompts`, `--seed`, `--judge-samples`, `--format human|json`,
and `--tokenizer o200k-base|cl100k-base` (default `o200k-base`; overrides
the config file's `[score] tokenizer`) for the token-bloat analysis.

If no model can be resolved, `adept score` exits `2` with an actionable
message instead of making a network call:

```console
$ adept score path/to/skill/SKILL.md
adept: error: could not resolve an LLM model to score with.
  set one of: --model <MODEL>, config file `[score] model = "..."`, or the ADEPT_MODEL environment variable
  optionally also set ADEPT_BASE_URL (defaults to the OpenAI API) and ADEPT_API_KEY
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
| `--model <M>` / `--base-url <U>` | LLM overrides, resolved against `[fix]`, not `[score]`. |

Uses the same `ADEPT_MODEL` / `ADEPT_BASE_URL` / `ADEPT_API_KEY`
environment variables and `--model`/`--base-url` flags as `adept score`,
but resolved against the independent `[fix]` config section (see
Configuration below) — `adept fix` can point at a different model than
`adept score`. If no model can be resolved, it exits `2` with the same
kind of actionable message `adept score` gives.

A fix candidate for `SL302` is rejected (even if it clears the diagnostic)
unless it *relocates* content into companion files rather than deleting
it — the token-conservation guard in `adept_fix::relocate`.

## `adept mcp`

Runs `adept` as an MCP server over stdio, exposing the static/offline
capabilities (`check_skill`, `format_skill`) as tools for other agents to
call. Nothing but JSON-RPC responses is ever written to stdout; logging
goes to stderr.

```console
$ echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | adept mcp
{"jsonrpc":"2.0","id":1,"result":{"tools":[
  {"name":"check_skill", "description":"Lint a SKILL.md file...", "inputSchema":{...}},
  {"name":"format_skill", "description":"Format a SKILL.md file's content...", "inputSchema":{...}}
]}}
```

A third tool, `score_skill`, runs LLM-assisted scoring. Since it's
network-backed, it's only advertised in `tools/list` when an LLM backend
can actually be resolved (`ADEPT_MODEL` etc. set, or `model`/`base_url`
arguments passed); calling it directly without a resolvable config
returns a structured tool error (`isError: true`) rather than hanging or
panicking, and requests are bounded by an internal timeout.

```console
$ ADEPT_MODEL=gpt-4o-mini echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | adept mcp
{"jsonrpc":"2.0","id":1,"result":{"tools":[
  {"name":"check_skill", ...},
  {"name":"format_skill", ...},
  {"name":"score_skill", "description":"Score a skill's triggering accuracy, token bloat, and overlap with sibling skills using an LLM...", "inputSchema":{...}}
]}}
```

`format_skill`'s `line_width` argument is validated to the range
`20..=500`; out-of-range or zero values are rejected with a structured
tool error instead of silently truncating or producing degenerate
one-word-per-line output.

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

[score]
model = "gpt-4o-mini"
base_url = "https://api.openai.com/v1"
tokenizer = "o200k_base"  # or "cl100k_base"

[fix]
model = "gpt-4o"
base_url = "https://api.openai.com/v1"
tokenizer = "o200k_base"  # or "cl100k_base"
max_rounds = 2             # falls back to adept_fix::DEFAULT_MAX_ROUNDS
```

`[fix]` is fully independent of `[score]` — set both if you want `fix` and
`score` to use different models.

Precedence: CLI flag > config file value > built-in default.

## Rules

See [`docs/RULES.md`](docs/RULES.md) for the full table of rule codes
(`SL001`–`SL403`), what each one flags, and how to fix it.

## Development

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
