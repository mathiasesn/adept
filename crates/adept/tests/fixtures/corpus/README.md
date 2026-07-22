# Vendored skills corpus

This directory is a fixed, in-repo snapshot of a subset of real skills from
[`anthropics/skills`](https://github.com/anthropics/skills), used as a broad-input
regression fixture (real markdown messiness, not synthetic text) for the corpus
lint and formatter-idempotency tests in this crate. See
`specs/vendored-skills-corpus-fixture.md` for why this exists.

## Provenance

- Upstream repository: https://github.com/anthropics/skills
- Pinned commit SHA: `1f630fdf9259cec4a14913127dfd7c3b69ef72eb`
- Upstream license: Apache License, Version 2.0 (per-skill `LICENSE.txt`, all
  identical Apache 2.0 boilerplate text at the pinned SHA, verified by hashing
  every `skills/*/LICENSE.txt` in the upstream tree)
- The upstream Apache 2.0 license text is committed here as `LICENSE.upstream`
  (copied verbatim from `skills/webapp-testing/LICENSE.txt` upstream; all
  Apache-2.0 skills at the pinned SHA carry the same text, modulo one skill —
  `frontend-design` — whose copy omits the closing "APPENDIX: How to apply the
  Apache License" boilerplate; the license grant text itself is identical).
- Upstream has no repo-root `LICENSE` or `NOTICE` file; licensing is applied
  per skill directory via `skills/<name>/LICENSE.txt`. No `NOTICE` file exists
  upstream for any of the vendored skills, so none is included here.

## License split (why these 10 and not others)

`anthropics/skills` is **not uniformly licensed**. At the pinned SHA, the
top-level `skills/` directory contains 17 skills. Four of them —
`skills/docx`, `skills/pdf`, `skills/pptx`, `skills/xlsx` — carry a
source-available, **not open source**, per-directory `LICENSE.txt`
("© 2025 Anthropic, PBC. All rights reserved. ... governed by your agreement
with Anthropic regarding use of Anthropic's services."). This was verified
directly by reading the upstream root `README.md` (which states this
explicitly) and by diffing every `skills/*/LICENSE.txt` file in the pinned
tree: the four source-available skills share one license text, and the
remaining thirteen skills share the Apache 2.0 license text (with the one
`frontend-design` formatting difference noted above).

**Those four skills — `docx`, `pdf`, `pptx`, `xlsx` — are excluded from this
corpus and MUST NOT be added.**

Of the remaining 13 Apache-2.0 skills, three were also excluded for containing
binary assets, which the spec for this corpus explicitly rules out
(text-only fixture, no binary assets):
- `canvas-design` — ships ~50 TrueType `.ttf` font files.
- `theme-factory` — ships a `.pdf` file (`theme-showcase.pdf`).
- `web-artifacts-builder` — ships a `.tar.gz` archive.

That leaves exactly the 10 skills vendored here — every remaining Apache-2.0,
text-only skill in the upstream repo at the pinned SHA was vendored; no
further topical selection was needed to reach the 10-15 target.

## Selection criteria

The spec asked for breadth of markdown structure and skill shape (with/without
companion files, short/long bodies) over topical variety. The 10 vendored
skills happen to be the full set of Apache-2.0, text-only skills upstream, and
they already span that range without curation:

| skill | companion files | approx. size |
|---|---|---|
| `doc-coauthoring` | none (`SKILL.md` only) | 20K |
| `brand-guidelines` | 1 | 20K |
| `frontend-design` | 1 | 28K |
| `algorithmic-art` | 3 | 72K |
| `internal-comms` | 5 | 40K |
| `webapp-testing` | 5 | 44K |
| `slack-gif-creator` | 6 | 64K |
| `mcp-builder` | 9 | 156K |
| `skill-creator` | 17 | 272K |
| `claude-api` | 65 | 1.1M |

## Vendored skills

```
algorithmic-art
brand-guidelines
claude-api
doc-coauthoring
frontend-design
internal-comms
mcp-builder
skill-creator
slack-gif-creator
webapp-testing
```

Each directory was copied **whole and verbatim** from upstream at the pinned
SHA — `SKILL.md` plus every companion file, byte-for-byte unmodified. No file
was trimmed, reformatted, or otherwise edited.

## Do not casually refresh this pin

This corpus is deliberately a **fixed** snapshot, not a tracking mirror of
upstream `main`. Do not bump the pinned SHA without re-doing the license-split
verification above: upstream may relicense a skill, move a skill between the
open-source and source-available sets, or add binary assets to a
currently-text-only skill. Refreshing the pin also invalidates every snapshot
committed against this corpus (`crates/adept/tests/snapshots/`) and will
produce a large, hard-to-review diff. If a refresh is genuinely needed, treat
it as its own reviewed change: re-verify the Apache-2.0 status of every
candidate skill at the new SHA, re-check for binary assets, and expect to
regenerate all corpus-derived snapshots.
