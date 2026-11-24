use ovrin_ns::{document, index::Policy, manifest::Namespace};
use serde_json::json;
use std::{fs, path::Path, process::Command};
fn write(p: &Path, s: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}
#[test]
fn manifest_boundaries_precedence_runtime_reload_and_record_exit() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("living-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let manifest = root.join("ovrin.build.json");
    let mut value = json!({
        "roots":[{"role":"governance","dir":"01_governance"},{"role":"docs","dir":"docs"}],
        "records":{"entries":[]},
        "index":{"roles":{
            "governance":[{"paths":["**"],"action":"embed"}],
            "docs":[{"paths":["**/*.md"],"action":"embed"},
                {"paths":["listed","listed/**"],"action":"list_only"},
                {"paths":["listed/**"],"action":"embed"}]},
            "exclusions":[{"paths":["**/build/**","**/vendor/**","**/upstream","**/upstream/**","**/.agent","**/.agent/**"],"action":"never"},
                {"paths":["except/*.md"],"except_paths":["except/keep.md"],"action":"never"}]}
    });
    write(&manifest, &value.to_string());
    write(
        &root.join("01_governance/registry/names.toml"),
        "[[name]]\nform='QROC'\nwrong=['Q-ROC']\n",
    );
    write(&root.join("01_governance/registry/slop.toml"), "pattern=[]");
    for dir in ["build", "vendor", "upstream", ".agent", "listed", "except"] {
        write(&root.join(format!("docs/{dir}/bad.md")), "Q-ROC");
    }
    write(&root.join("docs/except/keep.md"), "Q-ROC");
    write(&root.join("docs/live.md"), "Q-ROC");
    let ns = Namespace::load(&manifest).unwrap();
    let policy = Policy::load(&ns).unwrap().unwrap();
    for dir in ["build", "vendor", "upstream", ".agent", "listed"] {
        assert!(
            !policy.admits(&root.join(format!("docs/{dir}")), true),
            "{dir}"
        );
    }
    let report = document::inspect(&ns, &[root.join("docs")], "spine", None).unwrap();
    assert_eq!(report.checked, 2);
    assert_eq!(report.findings.len(), 2);
    value["index"]["roles"]["docs"][0]["action"] = json!("list_only");
    write(&manifest, &value.to_string());
    let ns = Namespace::load(&manifest).unwrap();
    assert_eq!(
        document::inspect(&ns, &[root.join("docs")], "spine", None)
            .unwrap()
            .checked,
        0
    );
    write(&root.join("01_governance/records/proof.md"), "Q-ROC");
    let output = Command::new(env!("CARGO_BIN_EXE_ovrin-ns"))
        .args([
            "--root",
            root.to_str().unwrap(),
            "check",
            root.join("01_governance/records/proof.md")
                .to_str()
                .unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("refused record:")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ovrin-ns"))
        .args(["--root", root.to_str().unwrap(), "check"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn record_hold_is_outside_living_text_and_successors_are_explicit() {
    use ovrin_ns::fix;
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("living-hold-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    write(&root.join("ovrin.build.json"), &json!({
        "roots":[{"role":"governance","dir":"01_governance"},{"role":"memory","dir":"06_memory"}],
        "records":{"entries":[{"role":"memory","paths":["runs/**"]}]},
        "index":{"roles":{"governance":[{"paths":["**"],"action":"embed"}],
            "memory":[{"paths":["runs","runs/**"],"action":"list_only"}]},"exclusions":[]}
    }).to_string());
    write(
        &root.join("01_governance/registry/names.toml"),
        "[[retired]]\nname='RevRun'\ncurrent='Soliton'\n[[retired]]\nname='chimera'\n",
    );
    write(&root.join("01_governance/registry/slop.toml"), "pattern=[]");
    let page = root.join("01_governance/page.md");
    write(&page, "RevRun chimera");
    let ns = Namespace::load(&root.join("ovrin.build.json")).unwrap();
    let plan = fix::plan(&ns, std::slice::from_ref(&page), true).unwrap();
    assert_eq!(plan.changes[0].edits.len(), 1);
    assert_eq!(plan.changes[0].edits[0].replacement, "Soliton");
    let work = root.join("06_memory/runs/proof");
    fs::create_dir_all(&work).unwrap();
    let accepted = work.join("accepted.json");
    write(&accepted, &serde_json::to_string(&plan).unwrap());
    assert_eq!(
        fix::apply(
            &ns,
            &plan,
            &accepted,
            &work.join("fix.tsv"),
            &work.join("before")
        )
        .unwrap(),
        1
    );
    assert_eq!(fs::read_to_string(page).unwrap(), "Soliton chimera");
    assert!(
        fs::read_to_string(work.join("fix.tsv"))
            .unwrap()
            .contains(".before")
    );
}
