use ovrin_ns::{
    document::{self, Catalogue},
    fix,
    manifest::Namespace,
    migration, preflight,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn write(p: &Path, s: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}
fn fixture() -> (PathBuf, Namespace) {
    let p = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "r2-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&p).unwrap();
    write(
        &p.join("ovrin.build.json"),
        r#"{"roots":[{"role":"constitution","dir":"00_constitution"},{"role":"governance","dir":"01_governance"},{"role":"memory","dir":"06_memory"}],"retired_roots":{"04_memory":"06_memory"},"records":{"entries":[]},"consumers":[]}"#,
    );
    write(
        &p.join("01_governance/registry/names.toml"),
        include_str!("fixtures/names.toml"),
    );
    write(
        &p.join("01_governance/registry/slop.toml"),
        include_str!("fixtures/slop.toml"),
    );
    fs::create_dir(p.join("00_constitution")).unwrap();
    fs::create_dir(p.join("06_memory")).unwrap();
    let ns = Namespace::load(&p.join("ovrin.build.json")).unwrap();
    (p, ns)
}
#[test]
fn chronicles_allow_historical_spellings_and_retired_names_but_never_slop() {
    let (p, ns) = fixture();
    let cat = Catalogue::load(&ns).unwrap();
    let text = "Q-ROC RevRun\n`Q-ROC` `RevRun`\nTODO\n";
    let f = cat
        .findings(
            &p.join("00_constitution/canon_chronicles/book.md"),
            text,
            "core",
        )
        .unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].entry, "SL06");
    for outside in [
        "00_constitution/canon_core/page.md",
        "docs/canon_chronicles/page.md",
    ] {
        let f = cat.findings(&p.join(outside), "Q-ROC", "core").unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].entry, "name:QROC");
    }
}
#[test]
fn scoped_proof_allowance_has_exact_counts_without_a_false_stale_in_the_other_class() {
    let (p, ns) = fixture();
    let file = p.join("proof.md");
    write(&file, "RevRun\n");
    write(
        &p.join("01_governance/registry/allow/proof.toml"),
        "[[allow]]\nfile='proof.md'\nentry='retired:RevRun'\ncount=1\nreason='Fixed historical proof input'\nclasses=['core']\n",
    );
    for class in ["core", "spine"] {
        let r = document::inspect(&ns, std::slice::from_ref(&file), class, None).unwrap();
        assert!(r.findings.is_empty(), "{r:?}");
        assert_eq!(
            r.allowed.iter().map(|a| a.count).sum::<usize>(),
            usize::from(class == "core")
        );
    }
}
#[test]
fn fix_keeps_prior_bytes_only_in_explicit_external_hold_and_journals_it() {
    let (p, ns) = fixture();
    let file = p.join("page.md");
    write(&file, "Q-ROC\n");
    let plan = fix::plan(&ns, std::slice::from_ref(&file), true).unwrap();
    let accepted = p.join("accepted.json");
    write(&accepted, &serde_json::to_string(&plan).unwrap());
    let hold = p.with_file_name(format!("{}-hold", p.file_name().unwrap().to_string_lossy()));
    let ledger = p.join("ledger.tsv");
    assert_eq!(
        fix::apply(&ns, &plan, &accepted, &ledger, &hold).unwrap(),
        1
    );
    let backup = hold.join(format!(
        "page.md.ovrin-ns.{}.before",
        plan.changes[0].before
    ));
    assert_eq!(fs::read_to_string(&backup).unwrap(), "Q-ROC\n");
    assert_eq!(fs::read_to_string(&file).unwrap(), "QROC\n");
    assert!(
        fs::read_to_string(ledger)
            .unwrap()
            .contains(backup.to_str().unwrap())
    );
    assert!(!p.join(backup.file_name().unwrap()).exists());
}
#[test]
fn internal_or_symlinked_internal_backup_is_refused_before_outputs() {
    let (p, ns) = fixture();
    let file = p.join("page.md");
    write(&file, "Q-ROC\n");
    let plan = fix::plan(&ns, std::slice::from_ref(&file), true).unwrap();
    let accepted = p.join("accepted.json");
    write(&accepted, &serde_json::to_string(&plan).unwrap());
    let alias = p.with_file_name(format!(
        "{}-alias",
        p.file_name().unwrap().to_string_lossy()
    ));
    std::os::unix::fs::symlink(&p, &alias).unwrap();
    for hold in [p.join("before"), alias.join("before")] {
        assert!(fix::apply(&ns, &plan, &accepted, &p.join("ledger"), &hold).is_err());
        assert!(!p.join("ledger").exists());
        assert!(!p.join("before").exists());
    }
    assert_eq!(fs::read_to_string(file).unwrap(), "Q-ROC\n");
}
#[test]
fn switch_reverse_counterpart_is_resolved_journaled_and_restored() {
    let (p, ns) = fixture();
    let live = p.join("consumer");
    write(&live, "old");
    let old = p.join("04_memory/staged");
    let new = p.join("06_memory/staged");
    write(&old, "new");
    let set = p.join("set.tsv");
    write(
        &set,
        &format!(
            "seq\tkind\tlive\tstaged\tinverse\treason\n1\tfile\t{}\t{}\tjournal\tready\n",
            live.display(),
            new.display()
        ),
    );
    let j = p.join("j.tsv");
    migration::switch_set_with(&set, &p.join("before"), &j, false, Some(&ns)).unwrap();
    assert!(
        fs::read_to_string(&j)
            .unwrap()
            .contains(new.to_str().unwrap())
    );
    assert_eq!(fs::read_to_string(&live).unwrap(), "new");
    migration::inverse_with(&j, Some(&ns)).unwrap();
    assert_eq!(fs::read_to_string(live).unwrap(), "old");
}
#[test]
fn move_reverse_counterpart_and_missing_both_are_distinct() {
    let (p, ns) = fixture();
    write(&p.join("04_memory/source"), "bytes");
    let map = p.join("map.tsv");
    write(
        &map,
        &format!(
            "seq\tfrom\tto\tkind\treason\n1\t{}\t{}\trecord\tready\n",
            p.join("06_memory/source").display(),
            p.join("destination").display()
        ),
    );
    let j = p.join("j.tsv");
    migration::move_map_with(&map, &j, false, Some(&ns)).unwrap();
    assert!(fs::read_to_string(&j).unwrap().contains("resolved_from"));
    migration::inverse_with(&j, Some(&ns)).unwrap();
    assert_eq!(
        fs::read_to_string(p.join("04_memory/source")).unwrap(),
        "bytes"
    );
    write(
        &map,
        &format!(
            "seq\tfrom\tto\tkind\treason\n1\t{}\t{}\trecord\tready\n",
            p.join("06_memory/absent").display(),
            p.join("dest2").display()
        ),
    );
    assert!(migration::move_map_with(&map, &p.join("j2.tsv"), true, Some(&ns)).is_err());
}
#[test]
fn staged_preflight_counts_arrivals_resolves_consumer_list_and_still_refuses_unmapped_paths() {
    let (p, _) = fixture();
    let list = p.join("04_memory/design/SWITCH.tsv");
    write(&list, "seq\tkind\tlive\tstaged\tinverse\treason\n");
    write(
        &list.parent().unwrap().join("MOVE_IN.tsv"),
        &format!(
            "seq\tfrom\tto\tkind\treason\n1\t{}\t{}\trecord\tready\n",
            p.join("previous/voice").display(),
            p.join("06_memory/voice").display()
        ),
    );
    let manifest = p.join("ovrin.build.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
    v["roots"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"role":"old_memory","dir":"04_memory"}));
    v["consumers"] = serde_json::json!({"role":"memory","path":"design/SWITCH.tsv"});
    v["landing"] = serde_json::json!([{"role":"memory","path":"voice"}]);
    v["topology"] = serde_json::json!({"specification":"06_memory/voice/spec.md"});
    write(&manifest, &v.to_string());
    let checks = preflight::run(&manifest, "staged").unwrap();
    assert!(checks.iter().all(|c| c.status != "FAIL"), "{checks:?}");
    assert_eq!(checks[8].status, "INFO");
    assert_eq!(checks[8].checked, 2);
    assert!(!checks[8].resolved_from.is_empty());
    v["landing"] = serde_json::json!([{"role":"memory","path":"unmapped"}]);
    write(&manifest, &v.to_string());
    assert_eq!(
        preflight::run(&manifest, "staged").unwrap()[0].status,
        "FAIL"
    );
}
#[test]
fn preflight_counterpart_lookup_handles_both_directions() {
    for (declared, present) in [
        ("04_memory/spec.md", "06_memory/spec.md"),
        ("06_memory/spec.md", "04_memory/spec.md"),
    ] {
        let (p, _) = fixture();
        write(&p.join(present), "Spec.\n");
        let m = p.join("ovrin.build.json");
        let mut v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&m).unwrap()).unwrap();
        if p.join("04_memory").is_dir() {
            v["roots"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({"role":"old_memory","dir":"04_memory"}));
        }
        v["topology"] = serde_json::json!({"specification":declared});
        write(&m, &v.to_string());
        let checks = preflight::run(&m, "staged").unwrap();
        assert_eq!(checks[0].status, "PASS");
        assert!(
            checks[8]
                .resolved_from
                .iter()
                .any(|r| r.resolved_from == p.join(declared) && r.path == p.join(present))
        );
    }
}

#[test]
fn fix_never_proposes_historical_name_rewrites_inside_chronicles() {
    let (p, ns) = fixture();
    let file = p.join("00_constitution/canon_chronicles/lineage.md");
    write(&file, "Q-ROC\n");
    let plan = fix::plan(&ns, &[file], true).unwrap();
    assert!(plan.changes.is_empty());
    assert!(plan.skipped[0].reason.contains("Chronicles"));
}

#[test]
fn staged_readme_link_resolves_to_its_current_home_counterpart_read_only() {
    let (p, _) = fixture();
    let host = p.join("host");
    let stage = host.join("namespace_next");
    write(
        &host.join("04_memory/design/SWITCH.tsv"),
        "seq\tkind\tlive\tstaged\tinverse\treason\n",
    );
    fs::create_dir_all(host.join("04_memory/runs")).unwrap();
    write(
        &stage.join("ovrin.build.json"),
        r#"{"roots":[{"role":"memory","dir":"06_memory"}],"retired_roots":{"04_memory":"06_memory"},"consumers":{"role":"memory","path":"design/SWITCH.tsv","staging_source":{"base":"home","path":"04_memory/design/SWITCH.tsv"}}}"#,
    );
    write(&stage.join("06_memory/README.md"), "[Run history](runs/)\n");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ovrin-ns"))
        .env("HOME", &host)
        .args([
            "--root",
            stage.to_str().unwrap(),
            "preflight",
            "--mode",
            "staged",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let checks: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(checks[2]["status"], "PASS");
    assert!(
        checks[8]["resolved_from"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["path"] == host.join("04_memory/runs").to_str().unwrap())
    );
    assert!(!stage.join("06_memory/runs").exists());
}
