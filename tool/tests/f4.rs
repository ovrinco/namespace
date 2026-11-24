use ovrin_ns::{document, manifest::Namespace};
use serde_json::json;
use std::{fs, path::Path, path::PathBuf, process::Command};
fn write(p: &Path, s: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}
fn root(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("f4-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    write(
        &root.join("ovrin.build.json"),
        json!({
            "roots":[{"role":"constitution","dir":"00_constitution"},
                {"role":"governance","dir":"01_governance"},{"role":"docs","dir":"docs"}],
            "records":{"entries":[]},
            "index":{"roles":{"constitution":[{"paths":["**"],"action":"embed"}],
                "governance":[{"paths":["**"],"action":"embed"}],
                "docs":[{"paths":["**"],"action":"embed"}]}}
        })
        .to_string()
        .as_bytes(),
    );
    write(
        &root.join("01_governance/registry/names.toml"),
        br#"[[name]]
form = "QROC"
wrong = ["Q-ROC"]
[[name]]
form = "STRATUM"
wrong = ["stratum"]
[[name.ordinary]]
wrong = "stratum"
use = "common_noun"
[[name]]
form = "Codex"
wrong = ["codex"]
[[name.ordinary]]
wrong = "codex"
use = "key"
[[name]]
form = "QREAM"
wrong = ["qream"]
[[retired]]
name = "nexvolv"
"#,
    );
    write(
        &root.join("01_governance/registry/slop.toml"),
        br#"[[pattern]]
id = "SL06"
kind = "regex"
pattern = "(?i)\\bplaceholder\\b"
applies_to = ["core"]
exempt = "forbidding"
"#,
    );
    write(
        &root.join("01_governance/registry/filenames.toml"),
        br#"casing = "^[a-z0-9]+(?:_[a-z0-9]+)*(?:\\.[a-z0-9]+)+$"
[[exception]]
name = "README.md"
[[exception]]
name = "CHANGELOG.md"
[chronicles]
folder = "canon_chronicles"
book = "^book_(?P<n>[ivxlc]+)_[a-z0-9]+(?:_[a-z0-9]+)*\\.md$"
"#,
    );
    root
}
fn entries(root: &Path, paths: &[PathBuf], class: &str) -> Vec<String> {
    let ns = Namespace::load(&root.join("ovrin.build.json")).unwrap();
    document::inspect(&ns, paths, class, None)
        .unwrap()
        .findings
        .into_iter()
        .map(|f| {
            format!(
                "{}:{}",
                f.file.file_name().unwrap().to_string_lossy(),
                f.entry
            )
        })
        .collect()
}
#[test]
fn ordinary_uses_forbidding_rules_and_glossary_definitions_are_not_hits() {
    let root = root("precision");
    let g = root.join("01_governance/handbook");
    write(
        &g.join("rules.md"),
        "Data lives in the stratum of models.\nstratum holds weights.\n\
No stub, filler or placeholder unless he says so.\nNever ship a \"placeholder\" label.\n\
The settings page is a placeholder.\nSources (version, codex and claude) import.\n\
Ask codex:01ab for the run. Then codex runs it.\nWe moved away from Q-ROC.\n"
            .as_bytes(),
    );
    write(
        &g.join("07_glossary.md"),
        b"- **NexVolv**: the earlier planning application.\n- **Plan**: never NexVolv again.\n",
    );
    let mut found = entries(&root, &[g], "core");
    found.sort();
    assert_eq!(
        found,
        [
            "07_glossary.md:retired:nexvolv",
            "rules.md:SL06",
            "rules.md:name:Codex",
            "rules.md:name:QROC",
            "rules.md:name:STRATUM",
        ]
    );
    fs::remove_dir_all(&root).unwrap();
}
#[test]
fn filenames_casing_chronicles_and_duplicates() {
    let root = root("filenames");
    let c = root.join("00_constitution");
    for f in [
        "README.md",
        "CHANGELOG.md",
        "canon_core/principles.md",
        "canon_core/Principles_New.md",
        "canon_chronicles/book_i_genesis.md",
        "canon_chronicles/book_ii_exodus.md",
        "canon_chronicles/book_iv_proverbs.md",
        "canon_chronicles/Book_V_Psalms.md",
        "canon_chronicles/origin_story.md",
    ] {
        write(&c.join(f), b"Text.\n");
    }
    write(&root.join("01_governance/laws/principles.md"), b"Text.\n");
    write(&root.join("01_governance/README.md"), b"Text.\n");
    write(
        &root.join("01_governance/skills/x/principles.md"),
        b"Text.\n",
    );
    let mut found = entries(&root, &[c.clone(), root.join("01_governance")], "core");
    found.sort();
    assert_eq!(
        found,
        [
            "Book_V_Psalms.md:filename:casing",
            "Book_V_Psalms.md:filename:chronicle",
            "Principles_New.md:filename:casing",
            "book_iv_proverbs.md:filename:chronicle",
            "principles.md:filename:duplicate",
            "principles.md:filename:duplicate",
        ]
    );
    fs::remove_dir_all(&root).unwrap();
}
#[test]
fn non_text_is_skipped_skill_fixtures_are_records_and_paths_resolve() {
    let root = root("cli");
    let d = root.join("docs/crate/tests/data");
    write(&d.join("stored.txt"), &[0x41, 0xd6, 0x00, 0xff]);
    write(&root.join("docs/live.md"), b"Q-ROC\n");
    let s = root.join("01_governance/skills/demo");
    write(&s.join("SKILL.md"), b"Q-ROC\n");
    write(&s.join("candidates/row.md"), b"Q-ROC\n");
    write(&s.join("proof/fixtures/case.md"), b"Q-ROC\n");
    let ns = Namespace::load(&root.join("ovrin.build.json")).unwrap();
    let r = document::inspect(&ns, &[root.join("docs"), s.clone()], "spine", None).unwrap();
    assert_eq!(r.skipped.len(), 1);
    assert_eq!(r.refused_records.len(), 2);
    assert_eq!(r.checked, 2);
    assert_eq!(r.findings.len(), 2);
    let bin = env!("CARGO_BIN_EXE_ovrin-ns");
    let rel = Command::new(bin)
        .current_dir(&root)
        .args(["--root", ".", "check", "docs", "--class", "spine"])
        .output()
        .unwrap();
    assert_eq!(rel.status.code(), Some(1), "{rel:?}");
    let missing = Command::new(bin)
        .current_dir(&root)
        .args(["--root", ".", "check", "nowhere", "--class", "spine"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    write(&root.join("docs/empty/data.json"), b"{}");
    let zero = Command::new(bin)
        .current_dir(&root)
        .args(["--root", ".", "check", "docs/empty", "--class", "spine"])
        .output()
        .unwrap();
    assert_eq!(zero.status.code(), Some(2));
    fs::remove_dir_all(&root).unwrap();
}
#[test]
fn html_comment_markers_and_cmake_files_are_not_prose() {
    let root = root("markers");
    write(
        &root.join("docs/a.md"),
        b"<!-- qream arch:begin -->\nText.\nqream runs.\n",
    );
    write(&root.join("docs/CMakeLists.txt"), b"# the qream binary\n");
    let found = entries(&root, &[root.join("docs")], "spine");
    assert_eq!(found, ["a.md:name:QREAM"]);
    fs::remove_dir_all(&root).unwrap();
}
