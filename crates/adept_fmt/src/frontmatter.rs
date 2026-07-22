//! Canonicalizing and rendering SKILL.md YAML frontmatter.

use adept::Frontmatter;

/// Render a [`Frontmatter`] in canonical form: `name`, `description`,
/// `license` (if present), then any remaining keys in alphabetical order
/// (guaranteed by [`Frontmatter::extra`] being a `BTreeMap`), each on its
/// own line, with minimal-but-correct YAML quoting, and a closing `---`
/// delimiter line.
///
/// The returned string ends with the closing `---\n` line; it does not
/// include the blank line that must separate it from the Markdown body —
/// that is added by the caller (see [`crate::format_skill`]).
pub fn render_frontmatter(fm: &Frontmatter) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("name: {}\n", yaml_scalar(&fm.name)));
    out.push_str(&format!("description: {}\n", yaml_scalar(&fm.description)));
    if let Some(license) = &fm.license {
        out.push_str(&format!("license: {}\n", yaml_scalar(license)));
    }
    for (key, extra) in &fm.extra {
        out.push_str(&render_extra_field(key, &extra.value));
    }
    out.push_str("---\n");
    out
}

/// Render a single non-well-known frontmatter key/value pair. The value is
/// arbitrary YAML (any type), so this defers to `serde_yaml`'s own
/// formatting rather than the bespoke minimal-quoting logic used for the
/// three well-known string fields.
fn render_extra_field(key: &str, value: &serde_yaml::Value) -> String {
    let mut map = serde_yaml::Mapping::new();
    map.insert(serde_yaml::Value::String(key.to_string()), value.clone());
    serde_yaml::to_string(&serde_yaml::Value::Mapping(map))
        .unwrap_or_else(|_| format!("{key}: null\n"))
}

/// Render a plain scalar string as YAML, quoting with double quotes only
/// when required for the value to round-trip unambiguously.
fn yaml_scalar(s: &str) -> String {
    if needs_quoting(s) {
        yaml_double_quote(s)
    } else {
        s.to_string()
    }
}

fn needs_quoting(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if s.trim() != s {
        return true;
    }
    if s.contains('\n') || s.contains('\t') {
        return true;
    }
    let first = s.chars().next().expect("checked non-empty above");
    if "-?:,[]{}#&*!|>'\"%@`".contains(first) {
        return true;
    }
    if s.contains(": ") || s.ends_with(':') || s.contains(" #") {
        return true;
    }
    let lower = s.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off"
    ) {
        return true;
    }
    if s.parse::<f64>().is_ok() {
        return true;
    }
    false
}

fn yaml_double_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}
