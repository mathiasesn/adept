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
Flags a body with no top-level (`#`) heading.
**Fix:** add a single `# Title` heading near the top of the body.

### SL103 `heading-skip` (Warning)
Flags a heading level that jumps by more than one, e.g. an `h1` followed
directly by an `h3` with no intervening `h2`.
**Fix:** use the next heading level down, or add the missing intervening
headings.

### SL104 `broken-file-reference` (Error)
Flags a relative path or markdown link mentioned in the body (via
`[text](path)` or backtick-quoted paths) that does not exist on disk next to
SKILL.md.
**Fix:** fix the path, or add the missing file next to SKILL.md.

## SL2xx — description / triggering heuristics

### SL201 `description-too-short` (Warning)
Flags a description below `description_min_tokens` (default **6**).
Rationale: below this, a description can't state both what the skill does
and when to use it.
**Fix:** expand the description.

### SL202 `description-too-long` (Warning)
Flags a description over `description_max_tokens` (default **75**), as a
style/quality concern. See also SL301, the hard-budget equivalent.
Rationale: 75 `o200k_base` tokens is generously above two long sentences.
**Fix:** trim the description to one or two sentences.

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
Flags a description over `description_max_tokens` (default **75**), as a
hard budget breach (`Error`) independent of the softer SL202 style rule, so
CI can gate on it.
**Fix:** shorten the description below the configured token budget.

### SL302 `body-tokens-over-budget` (Error)
Flags a SKILL.md body over `body_max_tokens` (default **1500**).
Rationale: the body is loaded into context in full once a skill triggers;
1500 `o200k_base` tokens is generous for a focused skill.
**Fix:** move detailed reference material into companion files loaded on
demand.

### SL303 `companion-file-bloat` (Warning)
Flags any companion file (a file other than SKILL.md in the skill's
directory) over `companion_file_max_tokens` (default **2000**).
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
