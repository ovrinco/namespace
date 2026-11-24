use ovrin_ns::{
    document::{self, Catalogue},
    manifest::Namespace,
    migration,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn write(p: &Path, s: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}
fn fixture() -> (PathBuf, Namespace) {
    let p = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "w5-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&p).unwrap();
    write(
        &p.join("ovrin.build.json"),
        r#"{"roots":[{"role":"constitution","dir":"00_constitution"},{"role":"governance","dir":"01_governance"},{"role":"memory","dir":"06_memory"}],"retired_roots":{"04_memory":"06_memory","02_divisions":null},"records":{"entries":[{"role":"memory","paths":["**"]}]}}"#,
    );
    write(
        &p.join("01_governance/registry/names.toml"),
        "[[name]]\nform='QREAM'\nwrong=['qream']\n[[name]]\nform='QROC'\nwrong=['Q-ROC']\n[[name]]\nform='Phod'\nwrong=['PHOD']\n[[retired]]\nname='RevRun'\ncurrent='Soliton'\n",
    );
    write(
        &p.join("01_governance/registry/slop.toml"),
        include_str!("fixtures/slop.toml"),
    );
    let ns = Namespace::load(&p.join("ovrin.build.json")).unwrap();
    (p, ns)
}
#[test]
fn code_keeps_lowercase_commands_but_refuses_wrong_retired_and_slop() {
    let (p, ns) = fixture();
    let cat = Catalogue::load(&ns).unwrap();
    let file = p.join("page.md");
    for text in [
        "`qream submit`",
        "```sh\nqream submit\n```",
        "``qream ` submit``",
    ] {
        assert!(
            cat.findings(&file, text, "core").unwrap().is_empty(),
            "{text}"
        );
    }
    for (text, id) in [
        ("`Q-ROC`", "name:QROC"),
        ("```\nPHOD\n```", "name:Phod"),
        ("`RevRun`", "retired:RevRun"),
        ("```\nrevrun\n```", "retired:RevRun"),
        ("`TODO`", "SL06"),
        ("`/path/RevRun/file`", "retired:RevRun"),
    ] {
        let f = cat.findings(&file, text, "core").unwrap();
        assert_eq!(f.len(), 1, "{text}: {f:?}");
        assert_eq!(f[0].entry, id);
    }
}
fn allowance(p: &Path, body: &str) {
    write(&p.join("01_governance/registry/allow/test.toml"), body);
}
#[test]
fn counted_allowance_is_subtracted_reported_and_stale_counts_do_not_hide_hits() {
    let (p, ns) = fixture();
    let page = p.join("history.md");
    write(&page, "RevRun RevRun\n");
    allowance(
        &p,
        "[[allow]]\nfile='history.md'\nentry='retired:RevRun'\ncount=1\nreason='Dated historical source context'\n",
    );
    let report = document::inspect(&ns, std::slice::from_ref(&page), "core", None).unwrap();
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.allowed[0].count, 1);
    assert!(!report.allowed[0].reason.is_empty());
    allowance(
        &p,
        "[[allow]]\nfile='history.md'\nentry='retired:RevRun'\ncount=3\nreason='Dated historical source context'\n",
    );
    let report = document::inspect(&ns, &[page], "core", None).unwrap();
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|f| f.entry == "allowance:stale")
            .count(),
        1
    );
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|f| f.entry == "retired:RevRun")
            .count(),
        2
    );
    assert!(report.allowed.is_empty());
}
#[test]
fn invalid_allowances_missing_reason_missing_file_and_constitution_each_are_findings() {
    let (p, ns) = fixture();
    write(&p.join("history.md"), "Historical context.\n");
    write(&p.join("00_constitution/canon.md"), "Canon.\n");
    for body in [
        "[[allow]]\nfile='history.md'\nentry='retired:RevRun'\ncount=1\n",
        "[[allow]]\nfile='absent.md'\nentry='retired:RevRun'\ncount=1\nreason='Historical context'\n",
        "[[allow]]\nfile='00_constitution/canon.md'\nentry='retired:RevRun'\ncount=1\nreason='Historical context'\n",
    ] {
        allowance(&p, body);
        let r = document::inspect(&ns, &[p.join("history.md")], "core", None).unwrap();
        assert_eq!(r.findings.len(), 1, "{r:?}");
        assert_eq!(r.findings[0].entry, "allowance:invalid");
    }
}
#[test]
fn allowances_cannot_overdraw_across_files_or_touch_unchecked_class() {
    let (p, ns) = fixture();
    write(&p.join("history.md"), "RevRun\n");
    write(&p.join("other.md"), "Clean\n");
    for name in ["a", "b"] {
        write(
            &p.join(format!("01_governance/registry/allow/{name}.toml")),
            "[[allow]]\nfile='history.md'\nentry='retired:RevRun'\ncount=1\nreason='Historical context'\n",
        );
    }
    let r = document::inspect(&ns, &[p.join("history.md")], "core", None).unwrap();
    assert_eq!(r.allowed.len(), 1);
    assert_eq!(r.findings.len(), 1);
    assert_eq!(r.findings[0].entry, "allowance:stale");
    let r = document::inspect(&ns, &[p.join("other.md")], "spine", None).unwrap();
    assert!(r.findings.is_empty());
    assert!(r.allowed.is_empty());
}
#[test]
fn constitution_changelog_is_a_record_for_checks_and_fixes() {
    let (p, ns) = fixture();
    let page = p.join("00_constitution/CHANGELOG.md");
    write(&page, "Q-ROC TODO RevRun\n");
    assert!(ns.is_record(&page));
    let r = document::inspect(&ns, std::slice::from_ref(&page), "core", None).unwrap();
    assert_eq!(r.checked, 0);
    assert_eq!(r.refused_records, vec![page.clone()]);
    assert!(r.findings.is_empty());
    assert!(
        ovrin_ns::fix::plan(&ns, &[page], true)
            .unwrap()
            .changes
            .is_empty()
    );
}
#[test]
fn changed_since_checks_only_the_governed_file_touched_after_stamp() {
    let (p, ns) = fixture();
    let old = p.join("01_governance/old.md");
    let new = p.join("01_governance/new.md");
    let spine = p.join("notes.md");
    let stamp = p.join("stamp");
    write(&old, "Q-ROC\n");
    write(&new, "Q-ROC\n");
    write(&spine, "Q-ROC\n");
    write(&stamp, "");
    let before = SystemTime::now() - Duration::from_secs(20);
    let time = SystemTime::now() - Duration::from_secs(10);
    for (file, mtime) in [(&old, before), (&stamp, time)] {
        fs::File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(mtime))
            .unwrap();
    }
    let r = document::inspect(&ns, std::slice::from_ref(&p), "core", Some(&stamp)).unwrap();
    assert_eq!(r.checked, 1);
    assert_eq!(r.findings.len(), 1);
    assert_eq!(r.findings[0].file, new);
}
#[test]
fn moved_switch_path_resolves_and_is_journaled_and_inverted() {
    let (p, ns) = fixture();
    let live = p.join("consumer");
    let old = p.join("04_memory/staged");
    let staged = p.join("06_memory/staged");
    write(&live, "old bytes");
    write(&staged, "new bytes");
    let set = p.join("set.tsv");
    write(
        &set,
        &format!(
            "seq\tkind\tlive\tstaged\tinverse\treason\n1\tfile\t{}\t{}\tjournal\tready\n",
            live.display(),
            old.display()
        ),
    );
    let j = p.join("j.tsv");
    assert_eq!(
        migration::switch_set_with(&set, &p.join("before"), &j, true, Some(&ns)).unwrap(),
        1
    );
    assert!(!j.exists());
    migration::switch_set_with(&set, &p.join("before"), &j, false, Some(&ns)).unwrap();
    assert_eq!(fs::read_to_string(&live).unwrap(), "new bytes");
    let text = fs::read_to_string(&j).unwrap();
    assert!(text.contains("resolved_from"));
    assert!(text.contains(old.to_str().unwrap()));
    migration::inverse_with(&j, Some(&ns)).unwrap();
    assert_eq!(fs::read_to_string(live).unwrap(), "old bytes");
}
#[test]
fn moved_map_and_journal_resolve_and_inverse_uses_moved_parent() {
    let (p, ns) = fixture();
    let old = p.join("04_memory");
    let new = p.join("06_memory");
    fs::create_dir_all(&old).unwrap();
    write(&old.join("source"), "bytes");
    let map = new.join("map.tsv");
    write(
        &map,
        &format!(
            "seq\tfrom\tto\tkind\treason\n1\t{}\t{}\trecord\tready\n",
            old.join("source").display(),
            new.join("destination").display()
        ),
    );
    fs::rename(old.join("source"), new.join("source")).unwrap();
    let j = new.join("j.tsv");
    migration::move_map_with(&old.join("map.tsv"), &j, false, Some(&ns)).unwrap();
    assert!(fs::read_to_string(&j).unwrap().contains("resolved_from"));
    migration::inverse_with(&old.join("j.tsv"), Some(&ns)).unwrap();
    assert_eq!(fs::read_to_string(new.join("source")).unwrap(), "bytes");
}
#[test]
fn no_successor_and_null_successor_are_refused_without_an_act() {
    let (p, ns) = fixture();
    let set = p.join("set.tsv");
    for root in ["04_memory", "02_divisions"] {
        write(
            &set,
            &format!(
                "seq\tkind\tlive\tstaged\tinverse\treason\n1\tfile\t{}\t{}\tjournal\tready\n",
                p.join("live").display(),
                p.join(root).join("missing").display()
            ),
        );
        let error =
            migration::switch_set_with(&set, &p.join("before"), &p.join("j.tsv"), true, Some(&ns))
                .unwrap_err()
                .to_string();
        assert!(error.contains("seq 1"));
        assert!(error.contains("absent under original and successor"));
        assert!(!p.join("j.tsv").exists());
        assert!(!p.join("before").exists());
    }
}
#[test]
fn held_row_anywhere_refuses_the_whole_run_before_every_act() {
    let (p, ns) = fixture();
    let staged = p.join("staged");
    write(&staged, "new");
    let live = p.join("live");
    write(&live, "old");
    let set = p.join("set.tsv");
    for (staged2, reason) in [
        ("", "ready"),
        (staged.to_str().unwrap(), "HOLD: owner supplies release"),
    ] {
        write(
            &set,
            &format!(
                "seq\tkind\tlive\tstaged\tinverse\treason\n1\tfile\t{}\t{}\tjournal\tready\n2\tfile\t{}\t{staged2}\tjournal\t{reason}\n",
                live.display(),
                staged.display(),
                p.join("other").display()
            ),
        );
        let e =
            migration::switch_set_with(&set, &p.join("before"), &p.join("j.tsv"), false, Some(&ns))
                .unwrap_err();
        assert!(e.to_string().contains("seq 2 refused: held row"));
        assert_eq!(fs::read_to_string(&live).unwrap(), "old");
        assert!(!p.join("before").exists());
        assert!(!p.join("j.tsv").exists());
    }
}

#[test]
fn timestamp_scans_prune_manifest_exclusions_and_records() {
    let (p, mut ns) = fixture();
    ns.value["index"] = serde_json::json!({"roles":{
        "governance":[{"paths":["**"],"action":"embed"}],
        "memory":[{"paths":["**"],"action":"embed"}]},
        "exclusions":[{"paths":["**/vendor/**"],"action":"never"}]});
    let stamp = p.join("stamp");
    write(&stamp, "");
    fs::File::options()
        .write(true)
        .open(&stamp)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(10)))
        .unwrap();
    write(
        &p.join("06_memory/README.md"),
        "Q-ROC
",
    );
    write(
        &p.join("01_governance/vendor/README.md"),
        "Q-ROC
",
    );
    write(
        &p.join("01_governance/new.md"),
        "Q-ROC
",
    );
    let r = document::inspect(&ns, std::slice::from_ref(&p), "core", Some(&stamp)).unwrap();
    assert_eq!(r.checked, 1);
    assert_eq!(r.findings[0].file, p.join("01_governance/new.md"));
}
#[test]
fn existing_original_wins_over_successor_and_inverse_can_follow_a_later_root_move() {
    let (p, ns) = fixture();
    let old = p.join("04_memory");
    let new = p.join("06_memory");
    write(&old.join("source"), "original");
    write(&new.join("source"), "successor");
    let map = p.join("map.tsv");
    write(
        &map,
        &format!(
            "seq\tfrom\tto\tkind\treason\n1\t{}\t{}\trecord\tready\n",
            old.join("source").display(),
            old.join("destination").display()
        ),
    );
    let j = old.join("j.tsv");
    migration::move_map_with(&map, &j, false, Some(&ns)).unwrap();
    assert_eq!(
        fs::read_to_string(old.join("destination")).unwrap(),
        "original"
    );
    assert_eq!(fs::read_to_string(new.join("source")).unwrap(), "successor");
    // Preserve the competing successor before the synthetic root relocation.
    fs::rename(new.join("source"), p.join("successor.saved")).unwrap();
    fs::rename(old.join("destination"), new.join("destination")).unwrap();
    fs::rename(&j, new.join("j.tsv")).unwrap();
    fs::rename(&old, p.join("old-root.hold")).unwrap();
    migration::inverse_with(&j, Some(&ns)).unwrap();
    assert_eq!(fs::read_to_string(new.join("source")).unwrap(), "original");
    assert!(
        fs::read_to_string(new.join("j.inverse.tsv"))
            .unwrap()
            .contains("resolved_from")
    );
}

#[test]
fn lowercase_project_keys_and_punctuation_free_commands_are_technical_forms() {
    let (p, _) = fixture();
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(p.join("ovrin.build.json")).unwrap()).unwrap();
    manifest["canonical_project_keys"] = serde_json::json!(["gxst", "zetl"]);
    write(&p.join("ovrin.build.json"), &manifest.to_string());
    write(
        &p.join("01_governance/registry/names.toml"),
        "[[name]]\nform='Z-ETL'\nwrong=['zetl']\n[[name]]\nform='GXST_OS'\nwrong=['gxst']\n[[name]]\nform='QROC'\nwrong=['q-roc','Q-ROC']\n",
    );
    let ns = Namespace::load(&p.join("ovrin.build.json")).unwrap();
    let cat = Catalogue::load(&ns).unwrap();
    assert!(
        cat.findings(&p.join("page.md"), "`zetl run`\n```sh\ngxst\n```", "core")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        cat.findings(&p.join("page.md"), "`Q-ROC`\n`q-roc`", "core")
            .unwrap()
            .len(),
        2
    );
}
