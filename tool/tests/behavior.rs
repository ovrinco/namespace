use ovrin_ns::{
    document::{self, Catalogue},
    fix,
    manifest::Namespace,
    preflight,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn write(p: &Path, text: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}
fn fixture() -> (PathBuf, Namespace) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "fixture-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&out).unwrap();
    for (name, body) in [
        (
            "ovrin.build.json",
            include_str!("fixtures/ovrin.build.json"),
        ),
        (
            "01_governance/registry/names.toml",
            include_str!("fixtures/names.toml"),
        ),
        (
            "01_governance/registry/slop.toml",
            include_str!("fixtures/slop.toml"),
        ),
    ] {
        write(&out.join(name), body);
    }
    assert!(root.exists());
    fs::create_dir_all(out.join("02_tools/demo")).unwrap();
    fs::create_dir_all(out.join("06_memory")).unwrap();
    write(&out.join("consumer.sh"), "#!/bin/sh\necho clean\n");
    let ns = Namespace::load(&out.join("ovrin.build.json")).unwrap();
    (out, ns)
}
#[test]
fn names_slop_context_and_code_exemptions() {
    let (root, ns) = fixture();
    let p = root.join("page.md");
    write(
        &p,
        "Q-ROC is useful. TODO\n`Q-ROC TODO` and ``Q-ROC ` TODO``\n```text\nQ-ROC TODO\n```\n/home/Q-ROC/file https://example/Q-ROC\nPreviously it worked.\nQROC was previously useful.\n[Q-ROC](../Q-ROC.md)\n    Q-ROC TODO\n",
    );
    let (f, n, r) = document::check(&ns, &[p], "core").unwrap();
    assert_eq!(n, 1);
    assert!(r.is_empty());
    assert_eq!(
        f.iter()
            .filter(|f| f.entry == "name:QROC")
            .map(|f| f.line)
            .collect::<Vec<_>>(),
        vec![1, 2, 2, 4, 9, 10]
    );
    assert_eq!(f.iter().filter(|f| f.entry == "SL06").count(), 5);
    assert_eq!(
        f.iter()
            .filter(|f| f.entry == "SL02")
            .map(|f| f.line)
            .collect::<Vec<_>>(),
        vec![8]
    );
}
#[test]
fn names_fix_is_accepted_bounded_ledgered_and_preserves_code() {
    let (root, ns) = fixture();
    let p = root.join("page.md");
    write(&p, "Q-ROC Q-ROCish Q-ROCs `Q-ROC` q-roc\n");
    let plan = fix::plan(&ns, std::slice::from_ref(&p), true).unwrap();
    assert_eq!(plan.changes.len(), 1);
    let accepted = root.join("accept.json");
    write(&accepted, &serde_json::to_string(&plan).unwrap());
    assert_eq!(
        fix::apply(
            &ns,
            &plan,
            &accepted,
            &root.join("ledger.tsv"),
            &ns.root.with_file_name(format!(
                "{}-before",
                ns.root.file_name().unwrap().to_string_lossy()
            ))
        )
        .unwrap(),
        1
    );
    assert_eq!(
        fs::read_to_string(&p).unwrap(),
        "QROC Q-ROCish Q-ROCs `Q-ROC` QROC\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("ledger.tsv"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(
        fix::apply(
            &ns,
            &plan,
            &accepted,
            &root.join("ledger.tsv"),
            &ns.root.with_file_name(format!(
                "{}-before",
                ns.root.file_name().unwrap().to_string_lossy()
            ))
        )
        .is_err()
    );
}
#[test]
fn records_never_read_or_rewritten() {
    let (root, ns) = fixture();
    let p = root.join("06_memory/original.md");
    write(&p, "Q-ROC TODO");
    let (f, n, r) = document::check(&ns, std::slice::from_ref(&p), "core").unwrap();
    assert!(f.is_empty());
    assert_eq!(n, 0);
    assert_eq!(r, vec![p.clone()]);
    let plan = fix::plan(&ns, std::slice::from_ref(&p), true).unwrap();
    assert!(plan.changes.is_empty());
    assert_eq!(plan.skipped[0].reason, "record: refused");
    assert_eq!(fs::read_to_string(p).unwrap(), "Q-ROC TODO");
}
#[test]
fn path_fix_skips_data_stores_and_binary() {
    let (root, ns) = fixture();
    write(&root.join("run.sh"), "02_old/demo X02_old 02_oldish\n");
    write(&root.join("data.json"), "[\"02_old\"]");
    write(&root.join("store.db"), "02_old");
    fs::write(root.join("binary"), b"02_old\0").unwrap();
    let targets = [
        root.join("run.sh"),
        root.join("data.json"),
        root.join("store.db"),
        root.join("binary"),
    ];
    let plan = fix::plan(&ns, &targets, false).unwrap();
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.skipped.len(), 3);
    let a = root.join("accepted.json");
    write(&a, &serde_json::to_string(&plan).unwrap());
    fix::apply(
        &ns,
        &plan,
        &a,
        &root.join("ledger.tsv"),
        &ns.root.with_file_name(format!(
            "{}-before",
            ns.root.file_name().unwrap().to_string_lossy()
        )),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(&targets[0]).unwrap(),
        "02_tools/demo X02_old 02_oldish\n"
    );
}
#[test]
fn preflight_finds_mismatch_retired_seed_and_broken_link() {
    let (root, _) = fixture();
    let m = root.join("ovrin.build.json");
    let base = preflight::run(&m, "staged").unwrap();
    assert!(!base.iter().any(|c| c.status == "FAIL"), "{base:?}");
    fs::create_dir(root.join("02_tools/extra")).unwrap();
    write(&root.join("consumer.sh"), "1\n2\n3\n4\n5\n6\n02_old/demo\n");
    let c = preflight::run(&m, "pre").unwrap();
    assert!(c[0].findings.iter().any(|s| s.contains("02_tools/extra")));
    assert!(
        c[1].findings
            .iter()
            .any(|s| s.contains("consumer.sh:7: 02_old"))
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("missing", root.join("broken")).unwrap();
        let mut v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&m).unwrap()).unwrap();
        v["consumers"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!("broken"));
        write(&m, &v.to_string());
        let c = preflight::run(&m, "pre").unwrap();
        assert!(c[2].findings.iter().any(|s| s.contains("broken")));
    }
}
#[test]
fn retired_names_are_case_insensitive_and_longest_replacement_wins() {
    let c=Catalogue::parse("[[name]]\nform='Toroidal Composition'\nwrong=['Paisano','Paisano Formatting']\n[[retired]]\nname='Paisano Formatting'\ncurrent='Toroidal Composition'", "pattern=[]").unwrap();
    assert!(
        !c.findings(Path::new("x.md"), "PAISANO FORMATTING", "core")
            .unwrap()
            .is_empty()
    );
}
#[test]
fn stale_or_tampered_accepted_plan_is_refused_before_write() {
    let (root, ns) = fixture();
    let p = root.join("page.txt");
    write(&p, "Q-ROC\n");
    let mut plan = fix::plan(&ns, std::slice::from_ref(&p), true).unwrap();
    let fresh = fix::plan(&ns, std::slice::from_ref(&p), true).unwrap();
    plan.changes[0].edits[0].replacement = "arbitrary".into();
    let a = root.join("accept.json");
    write(&a, &serde_json::to_string(&plan).unwrap());
    assert!(
        fix::apply(
            &ns,
            &fresh,
            &a,
            &root.join("ledger"),
            &ns.root.with_file_name(format!(
                "{}-before",
                ns.root.file_name().unwrap().to_string_lossy()
            ))
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(p).unwrap(), "Q-ROC\n");
}

#[test]
fn binary_path_offsets_are_exact_and_preflight_is_read_only() {
    let (root, _) = fixture();
    let manifest = root.join("ovrin.build.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
    v["consumers"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!("payload"));
    write(&manifest, &v.to_string());
    fs::write(
        root.join("payload"),
        b"\xff\0/home/user/02_old/bin\0unrelated 02_old\0",
    )
    .unwrap();
    let paths = ovrin_ns::files(std::slice::from_ref(&root)).unwrap();
    let snapshot = || {
        paths
            .iter()
            .map(|p| {
                (
                    p.clone(),
                    fs::metadata(p).unwrap().modified().unwrap(),
                    fix::hash(&fs::read(p).unwrap()),
                )
            })
            .collect::<Vec<_>>()
    };
    let before = snapshot();
    let checks = preflight::run(&manifest, "pre").unwrap();
    assert_eq!(snapshot(), before);
    assert_eq!(
        checks[1]
            .findings
            .iter()
            .filter(|s| s.contains("payload:byte13: 02_old"))
            .count(),
        1
    );
    assert_eq!(checks[1].findings.len(), 1);
}

#[test]
fn front_matter_and_filename_tokens_are_data() {
    let (_, ns) = fixture();
    let cat = Catalogue::load(&ns).unwrap();
    let text = "---\nschema: Q-ROC\n---\nQ-ROC.md is a file. `Q-ROC` is code.\nQ-ROC is prose.\n";
    let f = cat.findings(Path::new("page.md"), text, "core").unwrap();
    assert_eq!(f.len(), 2);
    assert_eq!(f[0].line, 4);
    assert_eq!(f[1].line, 5);
}

#[test]
fn unit_documentation_file_uri_resolves() {
    let (root, _) = fixture();
    write(
        &root.join("demo.service"),
        &format!(
            "[Unit]\nDocumentation=file:{}\n",
            root.join("consumer.sh").display()
        ),
    );
    let manifest = root.join("ovrin.build.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
    v["consumers"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!("demo.service"));
    write(&manifest, &v.to_string());
    let checks = preflight::run(&manifest, "pre").unwrap();
    assert_eq!(checks[2].status, "PASS");
    assert_eq!(checks[2].checked, 1);
}

#[test]
fn joined_tokens_are_never_findings_or_fixes_but_whole_variants_are() {
    let (root, ns) = fixture();
    let names = "[[name]]\nform='Lorengine'\nwrong=['lorengine']\n[[name]]\nform='Citium'\nwrong=['citium']\n[[name]]\nform='HYMDLR'\nwrong=['hymdlr','KAIROS']\n[[name]]\nform='Enzo'\nwrong=['enzo']\n[[name]]\nform='Codex'\nwrong=['codex']\n[[name]]\nform='QROC'\nwrong=['qroc','Q-ROC','q_roc']\n[[retired]]\nname='KAIROS'\ncurrent='HYMDLR'\n[[retired]]\nname='q_roc'\ncurrent='QROC'\n";
    write(&root.join("01_governance/registry/names.toml"), names);
    let cat = Catalogue::load(&ns).unwrap();
    for token in [
        "lorengine.dev",
        "citium.ovrinlabs.ca",
        "hymdlr.com",
        "enzo-cli",
        "codex-cli",
        "qroc_contract",
        "x.lorengine",
        "x-enzo",
        "x_qroc",
        "x/Q-ROC",
        "Q-ROC.x",
        "q_roc-v2",
    ] {
        let p = root.join("joined.txt");
        write(&p, token);
        assert!(
            cat.findings(&p, token, "core").unwrap().is_empty(),
            "{token}"
        );
        assert!(
            fix::plan(&ns, &[p], true).unwrap().changes.is_empty(),
            "{token}"
        );
    }
    for token in ["Q-ROC", "q_roc"] {
        let p = root.join("wrong.txt");
        write(&p, token);
        assert!(
            !cat.findings(&p, token, "spine").unwrap().is_empty(),
            "{token}"
        );
        assert_eq!(
            fix::plan(&ns, &[p], true).unwrap().changes[0].edits[0].replacement,
            "QROC"
        );
    }
}

#[test]
fn retired_identities_with_successors_are_mechanically_fixed() {
    let (root, ns) = fixture();
    write(
        &root.join("01_governance/registry/names.toml"),
        "[[name]]\nform='HYMDLR'\nwrong=['KAIROS']\n[[retired]]\nname='KAIROS'\ncurrent='HYMDLR'\n[[retired]]\nname='Phalanx'\ncurrent='compositor'\n[[retired]]\nname='Pneuvade'\ncurrent='execution'\n",
    );
    let cat = Catalogue::load(&ns).unwrap();
    for token in ["KAIROS", "kairos", "Phalanx", "Pneuvade"] {
        let p = root.join("history.md");
        write(&p, token);
        assert!(!cat.findings(&p, token, "core").unwrap().is_empty());
        assert!(cat.findings(&p, token, "spine").unwrap().is_empty());
        assert_eq!(fix::plan(&ns, &[p], true).unwrap().changes.len(), 1);
    }
}

#[test]
fn sl06_is_core_only_in_the_governance_catalogue() {
    let (root, ns) = fixture();
    // Exercise the shipped catalogue bytes as well as the parsing path.
    write(
        &root.join("01_governance/registry/slop.toml"),
        include_str!("fixtures/governance-slop.toml"),
    );
    let cat = Catalogue::load(&ns).unwrap();
    for token in ["TODO", "FIXME", "placeholder"] {
        assert_eq!(
            cat.findings(Path::new("core.md"), token, "core")
                .unwrap()
                .iter()
                .filter(|f| f.entry == "SL06")
                .count(),
            1
        );
        assert!(
            cat.findings(Path::new("spine.md"), token, "spine")
                .unwrap()
                .iter()
                .all(|f| f.entry != "SL06")
        );
    }
}

#[test]
fn chronicles_preserve_retired_lineage_under_core() {
    let cat = Catalogue::parse(
        "[[name]]\nform='HYMDLR'\nwrong=['KAIROS']\n[[retired]]\nname='KAIROS'\ncurrent='HYMDLR'",
        "pattern=[]",
    )
    .unwrap();
    assert!(
        cat.findings(
            Path::new("00_constitution/chronicles/lineage.md"),
            "KAIROS",
            "core"
        )
        .unwrap()
        .is_empty()
    );
    assert!(
        !cat.findings(
            Path::new("00_constitution/canon_core/lineage.md"),
            "KAIROS",
            "core"
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn list_nested_fences_do_not_invert_later_code_blocks() {
    let (root, ns) = fixture();
    let p = root.join("walkthrough.md");
    let text = "8. ```\n   Q-ROC\n   ```\nProse.\n\n```\nQ-ROC\n```\nQ-ROC\n";
    write(&p, text);
    let cat = Catalogue::load(&ns).unwrap();
    let f = cat.findings(&p, text, "core").unwrap();
    assert_eq!(f.len(), 3);
    assert_eq!(f.iter().map(|f| f.line).collect::<Vec<_>>(), vec![2, 7, 9]);
    assert_eq!(
        fix::plan(&ns, &[p], true).unwrap().changes[0].edits.len(),
        1
    );
}

#[test]
fn source_quotations_and_technical_search_literals_keep_bytes() {
    let (root, ns) = fixture();
    let p = root.join("technical.md");
    let text = "From session 123, lines 1–2:\n> \"Q-ROC was useful.\"\n\nFiles contain \"Q-ROC\" references.\nQ-ROC is useful.\n";
    write(&p, text);
    let cat = Catalogue::load(&ns).unwrap();
    let f = cat.findings(&p, text, "core").unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].line, 5);
    assert_eq!(
        fix::plan(&ns, &[p], true).unwrap().changes[0].edits.len(),
        1
    );
}

#[test]
fn source_linked_ledger_example_is_a_record_in_both_operations() {
    let (root, ns) = fixture();
    let p = root.join("ledger.md");
    let text =
        "# Ledger: Voice note of 29 September 2026\nSource: transcript\n| Source lines |\nQ-ROC\n";
    write(&p, text);
    let (f, n, r) = document::check(&ns, std::slice::from_ref(&p), "core").unwrap();
    assert!(f.is_empty());
    assert_eq!(n, 0);
    assert_eq!(r, vec![p.clone()]);
    for names in [true, false] {
        assert!(
            fix::plan(&ns, std::slice::from_ref(&p), names)
                .unwrap()
                .changes
                .is_empty()
        );
    }
    assert_eq!(fs::read_to_string(p).unwrap(), text);
}

#[test]
fn canon_chronicles_preserve_both_historical_names_and_spellings() {
    let cat=Catalogue::parse("[[name]]\nform='QROC'\nwrong=['Q-ROC']\n[[name]]\nform='HYMDLR'\nwrong=['KAIROS']\n[[retired]]\nname='KAIROS'\ncurrent='HYMDLR'", "pattern=[]").unwrap();
    let f = cat
        .findings(
            Path::new("00_constitution/canon_chronicles/book.md"),
            "KAIROS Q-ROC",
            "core",
        )
        .unwrap();
    assert!(f.is_empty());
}

#[test]
fn extracted_ledger_and_explicit_records_folder_are_protected() {
    let (root, ns) = fixture();
    let extracted = root.join("source_ledger.md");
    write(
        &extracted,
        "# Recovered from `source.xlsx`. Source chat: `source.md`; nothing edited.\n| Source lines |\nQ-ROC\n",
    );
    let record = root.join("docs/canon/records/adr.md");
    write(&record, "Q-ROC\n");
    for p in [extracted, record] {
        let (f, n, r) = document::check(&ns, std::slice::from_ref(&p), "core").unwrap();
        assert!(f.is_empty());
        assert_eq!(n, 0);
        assert_eq!(r.len(), 1);
        for names in [true, false] {
            assert!(
                fix::plan(&ns, std::slice::from_ref(&p), names)
                    .unwrap()
                    .changes
                    .is_empty()
            );
        }
    }
}

#[test]
fn case_replacement_maps_are_literal_data() {
    let (root, ns) = fixture();
    let p = root.join("map.md");
    write(
        &p,
        "Case variants: Q-ROC→QROC, q-roc->QROC.\nQ-ROC is prose.\n",
    );
    let (f, n, _) = document::check(&ns, std::slice::from_ref(&p), "core").unwrap();
    assert_eq!(n, 1);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].line, 2);
    assert_eq!(
        fix::plan(&ns, &[p], true).unwrap().changes[0].edits.len(),
        1
    );
}

#[test]
fn name_change_context_stays_in_its_markdown_sentence() {
    let cat=Catalogue::parse("[[name]]\nform='QROC'\nwrong=['Q-ROC']\n[[name]]\nform='execution'\nwrong=['Pneuvade']", "[[pattern]]\nid='SL05'\nkind='phrase'\npattern='superseded'\ncontext='name_change'\napplies_to=['core','spine']").unwrap();
    let text = "# QROC\n\nThe old architecture is superseded.\n\nQROC:\n- approval states: approved, superseded\n- QROC superseded the old identity.\n\nLocal execution may be superseded by cloud architectures.\n\nQROC was\nsuperseded by another name.\n";
    let f = cat.findings(Path::new("test.md"), text, "spine").unwrap();
    assert_eq!(f.iter().map(|f| f.line).collect::<Vec<_>>(), vec![7, 12]);
}

#[test]
fn unformatted_server_invocations_are_identifiers() {
    let (root, ns) = fixture();
    write(
        &root.join("01_governance/registry/names.toml"),
        "[[name]]\nform='Zerde'\nwrong=['zerde']",
    );
    let p = root.join("server.md");
    write(
        &p,
        "#### zerde serve / zerde api (Server Mode)\n#### zerde plugins (Plugin Management)\nzerde is the product.\n",
    );
    let (f, _, _) = document::check(&ns, std::slice::from_ref(&p), "spine").unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].line, 3);
    assert_eq!(
        fix::plan(&ns, &[p], true).unwrap().changes[0].edits.len(),
        1
    );
}
