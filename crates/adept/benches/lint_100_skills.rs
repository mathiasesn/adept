//! Benchmarks linting 100 generated skills. The doc's target
//! (`docs/adept-mvp.md`) is under 1 second.

use std::fs;

use criterion::{criterion_group, criterion_main, Criterion};

use adept::{LintConfig, Linter, SkillSet};

fn generate_skills(root: &std::path::Path, count: usize) {
    for i in 0..count {
        let dir = root.join(format!("skill-{i:03}"));
        fs::create_dir_all(&dir).expect("should create skill dir");
        let content = format!(
            "---\nname: skill-{i:03}\ndescription: Extracts widget number {i} from a document. Use when the user asks to extract widget {i} data, but do not use for unrelated widgets.\n---\n# Skill {i:03}\n\nThis skill extracts widget {i} data from the input document and returns it as structured JSON.\n\n## Usage\n\nRun the bundled script against the input file.\n"
        );
        fs::write(dir.join("SKILL.md"), content).expect("should write SKILL.md");
    }
}

fn bench_lint_100_skills(c: &mut Criterion) {
    let tmp = tempfile::tempdir().expect("should create tempdir");
    generate_skills(tmp.path(), 100);

    let set = SkillSet::discover(tmp.path()).expect("should discover skills");
    let linter = Linter::new(LintConfig::default()).expect("default tokenizer should load");

    c.bench_function("lint_100_skills", |b| {
        b.iter(|| linter.lint_set(std::hint::black_box(&set)));
    });
}

criterion_group!(benches, bench_lint_100_skills);
criterion_main!(benches);
