//! Shared test-only fixture builders for `commands::create` and
//! `commands::mcp`'s unit tests, both of which drive
//! `adept_agent::create_skill`/`generate_evals` through a
//! `MockLlmClient` and therefore need the same well-formed
//! `generate`/`eval` JSON payloads and clean skill body/description text.
//! Kept in one place so a schema change to either payload only needs
//! updating once.

/// A well-formed `create_skill` "generate" response body.
pub fn valid_generate_json(name: &str, description: &str, body: &str) -> String {
    serde_json::json!({
        "name": name,
        "description": description,
        "disable_model_invocation": false,
        "body": body,
        "companion_files": [],
    })
    .to_string()
}

/// A well-formed eval-dataset response with `n` trivially passing cases.
pub fn valid_eval_json(n: usize) -> String {
    let cases: Vec<_> = (0..n)
        .map(|i| {
            serde_json::json!({
                "prompt": format!("prompt {i}"),
                "assertions": [{"kind": "contains", "value": "ok"}],
            })
        })
        .collect();
    serde_json::json!({ "cases": cases }).to_string()
}

/// A clean `SKILL.md` body that passes lint with no diagnostics.
pub fn clean_body() -> &'static str {
    "# Demo Skill\n\n## Overview\n\nDoes the one thing this skill is for.\n\n## Steps\n\n1. Read the input.\n2. Produce the output.\n"
}

/// A clean frontmatter `description` that passes lint with no diagnostics.
pub fn clean_description() -> &'static str {
    "Extracts structured data from PDF forms. Use when the user needs form fields pulled out programmatically. Do not use for scanned image-only PDFs."
}
