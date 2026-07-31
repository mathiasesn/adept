//! All prompt templates used by `adept_agent::eval`, gathered in one module so
//! they can be audited and versioned. Bump [`PROMPT_VERSION`] whenever any
//! template's wording changes in a way that could shift scores, so old and
//! new [`crate::eval::report::EvalReport`]s can be told apart.

/// A version string stamped into every [`crate::eval::report::EvalReport`],
/// identifying which revision of these templates produced it.
pub const PROMPT_VERSION: &str = "adept_score-prompts-v1";

/// System prompt for generating candidate triggering-evaluation prompts.
///
/// Given a skill's name and description (and a target count `N`), asks the
/// model to produce `N` candidate user prompts: half that should trigger
/// the skill (`"positive"`) and half that should not (`"negative"`).
/// Expects a JSON response shaped like:
/// `{"prompts": [{"text": "...", "label": "positive"}, ...]}`.
pub const GENERATE_TRIGGER_PROMPTS_SYSTEM: &str = r#"You are building an evaluation set for an AI agent's tool-selection system. You will be given the name and description of one "skill" (a tool the agent can invoke) and a target count N.

Generate exactly N candidate user prompts:
- The first half should be prompts a well-calibrated agent SHOULD trigger this skill for (label "positive").
- The second half should be plausible prompts a well-calibrated agent should NOT trigger this skill for, e.g. superficially similar but out of scope, or clearly unrelated (label "negative").

Respond with strict JSON only, no commentary, in exactly this shape:
{"prompts": [{"text": "<user prompt text>", "label": "positive"}, ...]}

Each prompt should be a short, realistic message a user might type to the agent."#;

/// User-message template for [`GENERATE_TRIGGER_PROMPTS_SYSTEM`].
///
/// Format with `skill_name`, `skill_description`, and `count` (`N`).
pub const GENERATE_TRIGGER_PROMPTS_USER_TEMPLATE: &str =
    "Skill name: {skill_name}\nSkill description: {skill_description}\nN: {count}";

/// System prompt for the triggering judge.
///
/// Given ONLY a skill's name and description (never its body — the judge
/// must decide purely from what an agent's tool-selection layer would see)
/// and a single candidate user prompt, asks whether the skill would be
/// invoked. Always called at temperature 0. Expects a JSON response shaped
/// like: `{"would_trigger": true, "reasoning": "..."}`.
pub const JUDGE_TRIGGER_SYSTEM: &str = r#"You are simulating an AI agent's tool-selection step. You will be given the name and description of exactly one available skill (tool), and a user message. You do NOT have access to the skill's internal instructions — only its name and description, exactly as the real tool-selection layer would see.

Decide: would a well-calibrated agent invoke this skill in response to this user message?

Respond with strict JSON only, no commentary, in exactly this shape:
{"would_trigger": true, "reasoning": "<one sentence>"}"#;

/// User-message template for [`JUDGE_TRIGGER_SYSTEM`].
///
/// Format with `skill_name`, `skill_description`, and `user_prompt`.
pub const JUDGE_TRIGGER_USER_TEMPLATE: &str =
    "Skill name: {skill_name}\nSkill description: {skill_description}\n\nUser message: {user_prompt}";

/// System prompt for token-bloat trimming suggestions.
///
/// Given a skill's description, body, and companion-file token counts,
/// asks for concrete, actionable suggestions to reduce token usage without
/// losing functionality. Expects a JSON response shaped like:
/// `{"suggestions": ["...", ...]}`.
pub const TOKEN_BLOAT_SUGGESTIONS_SYSTEM: &str = r#"You are a technical editor reviewing an AI agent "skill" (a folder of instructions: a description plus a markdown body, and optionally companion files) for token bloat.

You will be given the skill's description, its body, and a token count breakdown. Suggest concrete, specific ways to reduce token usage while preserving all functional behavior (e.g.: "collapse the three near-duplicate examples in section X into one", "move the reference table to a companion file loaded on demand", "cut the preamble restating the description").

Respond with strict JSON only, no commentary, in exactly this shape:
{"suggestions": ["<concrete suggestion>", ...]}

Only include suggestions that are actually justified by the content shown; if the skill is already lean, return an empty list."#;

/// User-message template for [`TOKEN_BLOAT_SUGGESTIONS_SYSTEM`].
///
/// Format with `skill_name`, `description`, `body`, `description_tokens`,
/// `body_tokens`, and `companion_tokens_summary`.
pub const TOKEN_BLOAT_SUGGESTIONS_USER_TEMPLATE: &str = "Skill name: {skill_name}\n\nDescription ({description_tokens} tokens):\n{description}\n\nBody ({body_tokens} tokens):\n{body}\n\nCompanion files:\n{companion_tokens_summary}";

/// System prompt for overlap/conflict adjudication between two skills.
///
/// Given ONLY the name and description of each of two skills (shortlisted
/// offline by cheap pairwise similarity), asks whether they conflict or
/// overlap, and how to disambiguate them if so. Expects a JSON response
/// shaped like:
/// `{"overlaps": true, "conflicts": false, "explanation": "...", "disambiguation": "..."}`.
pub const OVERLAP_ADJUDICATION_SYSTEM: &str = r#"You are reviewing a set of AI agent skills (tools) for overlap and conflicts. You will be given the name and description of exactly two skills, as an agent's tool-selection layer would see them (not their internal bodies).

Decide:
- "overlaps": would a reasonable user request plausibly trigger BOTH skills, making it ambiguous which one an agent should pick?
- "conflicts": beyond mere overlap, do the two skills' descriptions actively contradict or duplicate each other's purpose such that one is likely redundant?
- If either is true, suggest a concrete way to disambiguate them (e.g. narrowing one description, adding an explicit "do not use for..." clause).

Respond with strict JSON only, no commentary, in exactly this shape:
{"overlaps": true, "conflicts": false, "explanation": "<one or two sentences>", "disambiguation": "<concrete suggestion, or empty string if not applicable>"}"#;

/// User-message template for [`OVERLAP_ADJUDICATION_SYSTEM`].
///
/// Format with `skill_a_name`, `skill_a_description`, `skill_b_name`, and
/// `skill_b_description`.
pub const OVERLAP_ADJUDICATION_USER_TEMPLATE: &str = "Skill A name: {skill_a_name}\nSkill A description: {skill_a_description}\n\nSkill B name: {skill_b_name}\nSkill B description: {skill_b_description}";

/// Render a template containing `{placeholder}` tokens by substituting each
/// `(name, value)` pair in `substitutions`.
///
/// A tiny helper rather than a templating dependency, since every template
/// above is a fixed, fully-enumerated set of placeholders.
pub fn render(template: &str, substitutions: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in substitutions {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_substitutes_all_placeholders() {
        let out = render(
            GENERATE_TRIGGER_PROMPTS_USER_TEMPLATE,
            &[
                ("skill_name", "pdf-filler"),
                ("skill_description", "Fills PDF forms"),
                ("count", "6"),
            ],
        );
        assert_eq!(
            out,
            "Skill name: pdf-filler\nSkill description: Fills PDF forms\nN: 6"
        );
    }
}
