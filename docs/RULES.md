# adept lint rules

This file documents every rule registered in `adept::Registry`. It must stay
in sync with the registry — `crates/adept/tests/docs_test.rs` asserts every
registered rule code appears here.

## SL00x — frontmatter / naming

### SL001 `missing-description` (Error)
Flags a `description` frontmatter field that is missing entirely (parse-time)
or present but empty/whitespace-only (post-parse).
**Fix:** write a description stating what the skill does and when to use it.

### SL002 `missing-name` (Error)
Flags a `name` frontmatter field that is missing entirely (parse-time) or
present but empty/whitespace-only (post-parse).
**Fix:** set `name` to match the skill's directory name.

### SL003 `malformed-frontmatter` (Error)
Flags unparseable YAML frontmatter, a missing opening/closing `---` fence, a
frontmatter block that isn't a YAML mapping, or a known field with the wrong
type. Synthesized directly from parse failures (`SkillSet::errors`), since a
skill with malformed frontmatter has no parsed `Skill` to run other rules
against.
**Fix:** fix the YAML frontmatter block so the file parses.

### SL004 `name-mismatch` (Warning)
Flags a frontmatter `name` that does not match the name of the directory
containing SKILL.md.
**Fix:** rename the `name` field or the directory so they match.

### SL005 `invalid-name-format` (Error)
Flags a `name` that is not kebab-case (contains whitespace, uppercase
letters, or characters other than lowercase ASCII letters, digits, and
hyphens).
**Fix:** use lowercase letters, digits, and hyphens only.

## SL1xx — structure

### SL101 `empty-body` (Error)
Flags a SKILL.md whose markdown body (everything after the frontmatter) is
empty or whitespace-only.
**Fix:** add instructions describing how to use the skill.

### SL102 `missing-h1` (Warning)
Flags a body with no top-level heading. Both ATX (`# Title`) and setext
(`Title` underlined with `===`) h1s satisfy it.
**Fix:** add a single `# Title` heading near the top of the body.

### SL103 `heading-skip` (Warning)
Flags a heading level that jumps by more than one, e.g. an `h1` followed
directly by an `h3` with no intervening `h2`.
**Fix:** use the next heading level down, or add the missing intervening
headings.

### SL104 `broken-file-reference` (Error)
Flags a markdown link/image destination (`[text](path)` / `![alt](path)`),
or a backtick-quoted span that is either an explicit relative path (`./x`,
`../x`) or contains a `/` and ends in a known file extension (e.g.
`` `scripts/run.py` ``), that does not exist on disk next to SKILL.md.

Deliberately conservative to avoid false positives on things that merely
look path-like: it skips anything with a URL scheme (`https://...`,
`mailto:...`), an in-page anchor (`#section`), a `~`-relative or absolute
(`/...`) path, a glob metacharacter (`*?[]{}`), a template placeholder
(`{lang}`, `<VAR>`, `$VAR`), a scoped package name (`@scope/name`), or a
bare filename with no directory component and no markdown-link context
(e.g. a prose mention of `package.json`) — real companion-file references
almost always come as a markdown link or an explicit relative path.

It also exempts paths the skill *instructs the reader to create*: if any
reference to a path sits on a body line phrased as a creation instruction
(a create/write/save/generate/output/produce/draft word — "Save test cases
to `evals/evals.json`"), that path is treated as skill-authored and every
reference to it is skipped, including later read/update mentions ("if
`evals/evals.json` already exists…"). Modification verbs like *update* are
deliberately excluded, since they imply the file should already exist.
Intent is detected per line, so a genuine broken reference sharing a line
with an unrelated creation instruction is not currently flagged.

It also exempts OOXML archive-internal part names — a path whose first
segment is a reserved Open Packaging Conventions root (`word/`, `ppt/`,
`xl/`, `docProps/`, `_rels/`, `customXml/`) *and* which ends in a part
extension (`.xml`/`.rels`), e.g. `word/document.xml` or
`ppt/slides/slideN.xml`. Skills that manipulate Office documents describe
editing these parts, which are constants fixed by ECMA-376 / ISO/IEC 29500,
not files bundled next to SKILL.md. The extension gate keeps a genuinely
broken reference to a non-part file under such a directory (`xl/helper.py`)
firing.
**Fix:** fix the path, or add the missing file next to SKILL.md.

### SL105 `setext-heading` (Info)
Flags a heading written in setext form — a `Title` line underlined with
`===` (h1) or `---` (h2) — rather than ATX (`# Title`). Reported as a style
issue because `adept fmt` rewrites setext headings to ATX, so leaving one in
place means the linter and the formatter disagree about style. Informational
only: the formatter resolves it automatically, so it should not fail CI.
**Fix:** run `adept fmt`, or write the heading as `# Title` / `## Title`.

## SL2xx — description / triggering heuristics

### SL201 `description-too-short` (Warning)
Flags a description below `description_min_tokens` (default **6**).
Rationale: below this, a description can't state both what the skill does
and when to use it.
**Fix:** expand the description.

### SL202 (retired)
Originally `description-too-long`, flagging the same condition
(`description_max_tokens` exceeded) as `SL301` below, at `Warning` instead
of `Error`, with no other distinct meaning — every over-long description
fired both codes for one defect. Resolved by retiring `SL202` in favor of
`SL301`: token-budget breaches are an `SL3xx` concern per the rule taxonomy,
so `SL301` is the sole rule for this condition now. The code `SL202` is not
reused, so old configs referencing it fail closed rather than silently
picking up a new meaning.

### SL203 `missing-trigger-phrase` (Warning)
Flags a description with no recognizable trigger phrasing (e.g. "use when",
"when the user", "triggers on").
**Fix:** add trigger phrasing, e.g. "Use when the user asks to...".

### SL204 `first-person-description` (Warning)
Flags a description written in first person ("I will...", "I can...")
instead of third person.
**Fix:** rewrite in third person, e.g. "Extracts..." instead of "I
extract...".

### SL205 `description-restates-name` (Warning)
Flags a description that is little more than the skill name reworded (≥80%
word overlap with the name).
**Fix:** describe what the skill does and when to use it, not just its name.

### SL206 `no-negative-guidance` (Info)
Flags a description with no guidance on when *not* to use the skill (e.g.
"do not use for..."). Informational only, since not every skill needs
negative guidance.
**Fix:** consider adding "Do not use for..." guidance to reduce
over-triggering.

## SL3xx — token budget

### SL301 `description-tokens-over-budget` (Error)
Flags a description over `description_max_tokens` (default **75**). The
sole rule for this condition; see `SL202` above for why.
**Fix:** shorten the description below the configured token budget.

### SL302 `body-tokens-over-budget` (Error)
Flags a SKILL.md body over `body_max_tokens` (default **1500**).
Rationale: the body is loaded into context in full once a skill triggers;
1500 `o200k_base` tokens is generous for a focused skill.
**Fix:** move detailed reference material into companion files loaded on
demand.

### SL303 `companion-file-bloat` (Warning)
Flags any companion file (a file other than SKILL.md in the skill's
directory) over `companion_file_max_tokens` (default **2000**). Bundled
license files are exempt — `LICENSE`, `LICENCE`, `COPYING`, `COPYRIGHT`
(any extension) and `LICENSE-*`/`LICENCE-*` variants — since their
boilerplate legal text is not skill content and routinely exceeds any
reasonable budget. The exemption is scoped to this rule; `adept_agent::eval`'s
token-bloat view still counts license files.

Files under a top-level `evals/` directory *within the skill's own
directory* (e.g. `<skill>/evals/evals.jsonl`, the synthetic eval dataset
`adept create` writes alongside a generated skill) are also exempt, matched
by directory name only — no filename pattern, no content sniffing. "Top-level"
is relative to the skill directory, not to the filesystem root or any other
ancestor: a file nested more than one level down (`<skill>/sub/evals/x`) is
not exempt, and a skill that merely happens to live somewhere under a
directory named `evals` on disk is not exempt either. Unlike the license
exemption, this one is applied to both
this rule and `adept_agent::eval`'s token-bloat view, since a generated dataset is
not skill content either. In practice this exemption is currently dormant:
companion-file discovery is non-recursive, so a file nested under `evals/`
is never discovered as a companion file in the first place and could never
have produced a finding here regardless. It is kept as defence-in-depth for
if discovery ever becomes recursive. See `docs/EVALS.md` for the dataset
schema itself.
**Fix:** split the companion file or trim it down.

## SL4xx — cross-skill

### SL401 `duplicate-skill-name` (Error)
Flags two or more skills in a `SkillSet` sharing the same frontmatter `name`.
**Fix:** give each skill a unique `name`.

### SL402 `similar-description` (Warning)
Flags two skills whose descriptions have a word-level Jaccard similarity at
or above `similar_description_threshold` (default **0.6**).
**Fix:** differentiate the descriptions so agents can tell the skills apart.

### SL403 `overlapping-trigger-phrasing` (Warning)
Flags two skills whose descriptions share a bigram-shingle Jaccard
similarity at or above `trigger_overlap_threshold` (default **0.5**),
suggesting they'll compete to trigger on the same requests.
**Fix:** narrow the trigger conditions so the skills don't compete for the
same requests.
