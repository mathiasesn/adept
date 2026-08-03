# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/mathiasesn/adept/compare/adept-agent-v0.2.0...adept-agent-v0.3.0) - 2026-08-03

### Fixed

- [**breaking**] stop credentials leaking through LlmError and base_url ([#47](https://github.com/mathiasesn/adept/pull/47))

## [0.2.0](https://github.com/mathiasesn/adept/compare/adept-agent-v0.1.1...adept-agent-v0.2.0) - 2026-08-03

### Added

- [**breaking**] distribute the adept CLI as a Python package ([#45](https://github.com/mathiasesn/adept/pull/45))

## [0.1.1](https://github.com/mathiasesn/adept/compare/adept-agent-v0.1.0...adept-agent-v0.1.1) - 2026-08-02

### Other

- release v0.1.0 ([#39](https://github.com/mathiasesn/adept/pull/39))

## [0.1.0](https://github.com/mathiasesn/adept/releases/tag/adept-agent-v0.1.0) - 2026-08-02

### Added

- [**breaking**] rename packages for crates.io and automate releases ([#35](https://github.com/mathiasesn/adept/pull/35))

### Fixed

- make the fix/create accept gate severity-aware

### Other

- add status badges to README ([#41](https://github.com/mathiasesn/adept/pull/41))
- adopt the adept logo ([#40](https://github.com/mathiasesn/adept/pull/40))
- tighten README, remove docs/MVP.md, drop dangling spec citations
- address cleanup review findings
- deduplicate eval orchestration, generalize MCP timeout helper
- Fix review findings in the eval surface
- Update docs for the eval unification
- Replace adept score with a unified adept eval command
- Dissolve adept_score into adept_agent as llm/ and eval/
- Add an offline eval-dataset grader to adept::evals
- Simplify the create surface after review
- Correct documentation drift found by review
- Fix review findings in the create surface
- Document the create surface and the evals contract
- Add preview-only create_skill and generate_evals MCP tools
- Add the adept create CLI surface
- Add adept_agent::create, the generate/screen/repair pipeline
- Rename adept_fix to adept_agent and split out the fix module
- Add tracing regression tests and document the capture layer
- Simplify adept fix: push fix regions onto the rule trait
- Address review: conservation guard soundness and docs
- dedup shared logic, cache tokenizer tables, cut boilerplate
- Fix review findings across all four crates
- Add adept CLI with check, fmt, score, and MCP subcommands
