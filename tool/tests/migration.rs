use ovrin_ns::{migration, preflight};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn scratch() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "migration-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&p).unwrap();
    p
}
fn write(p: &Path, s: &str) {
    fs::write(p, s).unwrap();
}
fn map(p: &Path, rows: &[(&str, &Path, &Path)]) -> PathBuf {
    let f = p.join("map.tsv");
    let mut s = "seq\tfrom\tto\tkind\treason\n".to_string();
    for (seq, a, b) in rows {
        s += &format!("{seq}\t{}\t{}\trecord\ttest\n", a.display(), b.display());
    }
    write(&f, &s);
    f
}
#[test]
fn occupied_move_is_refused_without_moving_source() {
    let p = scratch();
    let a = p.join("a");
    let b = p.join("b");
    write(&a, "original");
    write(&b, "occupied");
    let m = map(&p, &[("1", &a, &b)]);
    let e = migration::move_map(&m, &p.join("journal.tsv"), false).unwrap_err();
    assert!(e.to_string().contains("seq 1"));
    assert_eq!(fs::read_to_string(a).unwrap(), "original");
    assert_eq!(fs::read_to_string(b).unwrap(), "occupied");
}
#[test]
fn move_orders_sequences_preserves_dangling_link_and_inverts() {
    let p = scratch();
    let a = p.join("a");
    let b = p.join("b");
    let c = p.join("c");
    std::os::unix::fs::symlink("missing", &a).unwrap();
    let m = map(&p, &[("2", &b, &c), ("1", &a, &b)]);
    let j = p.join("journal.tsv");
    assert_eq!(migration::move_map(&m, &j, true).unwrap(), 2);
    assert!(!j.exists());
    assert_eq!(migration::move_map(&m, &j, false).unwrap(), 2);
    assert_eq!(migration::inverse(&j).unwrap(), 2);
    assert_eq!(fs::read_link(a).unwrap(), Path::new("missing"));
    assert!(b.symlink_metadata().is_err());
    assert!(c.symlink_metadata().is_err());
}
#[test]
fn switch_all_file_and_link_kinds_restore_bytes_and_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let p = scratch();
    let live = p.join("live");
    fs::create_dir(&live).unwrap();
    let stage = p.join("staged");
    fs::create_dir(&stage).unwrap();
    let mut rows = "seq\tkind\tlive\tstaged\tinverse\treason\n".to_string();
    for (i, kind) in [
        "file", "unit", "env", "pack", "trust", "link", "release", "key",
    ]
    .iter()
    .enumerate()
    {
        let a = live.join(kind);
        let b = stage.join(kind);
        if matches!(*kind, "link" | "release") {
            write(&b, "release");
            std::os::unix::fs::symlink("original-target", &a).unwrap();
        } else {
            write(&a, "old bytes");
            fs::set_permissions(&a, fs::Permissions::from_mode(0o751)).unwrap();
            if *kind != "key" {
                write(&b, "new bytes");
            }
        }
        rows += &format!(
            "{}\t{kind}\t{}\t{}\trestore journal\ttest\n",
            i + 1,
            a.display(),
            b.display()
        );
    }
    let set = p.join("set.tsv");
    write(&set, &rows);
    let j = p.join("switch.tsv");
    let before = p.join("before");
    assert_eq!(migration::switch_set(&set, &before, &j, true).unwrap(), 8);
    assert!(!before.exists());
    assert_eq!(migration::switch_set(&set, &before, &j, false).unwrap(), 8);
    assert_eq!(fs::read_to_string(live.join("file")).unwrap(), "new bytes");
    assert_eq!(migration::inverse(&j).unwrap(), 15);
    for kind in ["file", "unit", "env", "pack", "trust", "key"] {
        let a = live.join(kind);
        assert_eq!(fs::read_to_string(&a).unwrap(), "old bytes");
        assert_eq!(fs::metadata(a).unwrap().permissions().mode() & 0o777, 0o751);
    }
    for kind in ["link", "release"] {
        assert_eq!(
            fs::read_link(live.join(kind)).unwrap(),
            Path::new("original-target")
        );
    }
}
#[test]
fn inverse_refuses_modified_bytes_and_occupied_name() {
    let p = scratch();
    let a = p.join("a");
    let b = p.join("b");
    write(&a, "old");
    let m = map(&p, &[("1", &a, &b)]);
    let j = p.join("j.tsv");
    migration::move_map(&m, &j, false).unwrap();
    write(&b, "edited");
    assert!(migration::inverse(&j).is_err());
    assert_eq!(fs::read_to_string(b).unwrap(), "edited");
    assert!(!a.exists());
}
#[test]
fn post_detects_record_count_drift_and_retired_link_target() {
    let p = scratch();
    fs::create_dir(p.join("06_memory")).unwrap();
    write(&p.join("06_memory/record"), "1234");
    write(
        &p.join("census.tsv"),
        "path\tfiles\tbytes\n06_memory\t1\t4\n",
    );
    write(&p.join("consumer.sh"), "clean\n");
    write(
        &p.join("ovrin.build.json"),
        r#"{"roots":[{"role":"memory","dir":"06_memory"}],"records":{"entries":[{"role":"memory","paths":["**"]}],"census":"census.tsv"},"consumers":["consumer.sh"],"retired_roots":{"04_memory":"06_memory"}}"#,
    );
    let checks = preflight::run(&p.join("ovrin.build.json"), "post").unwrap();
    assert!(checks.iter().all(|c| c.status != "FAIL"), "{checks:?}");
    write(&p.join("06_memory/record"), "changed");
    write(&p.join("consumer.sh"), "a\nb\n04_memory/record\n");
    let checks = preflight::run(&p.join("ovrin.build.json"), "post").unwrap();
    assert_eq!(checks[7].status, "FAIL");
    assert!(
        checks[1]
            .findings
            .iter()
            .any(|s| s.contains("consumer.sh:3:"))
    );
    fs::create_dir(p.join("04_memory")).unwrap();
    std::os::unix::fs::symlink(p.join("04_memory"), p.join("current")).unwrap();
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(p.join("ovrin.build.json")).unwrap()).unwrap();
    v["consumers"] = serde_json::json!(["current"]);
    write(&p.join("ovrin.build.json"), &v.to_string());
    let checks = preflight::run(&p.join("ovrin.build.json"), "post").unwrap();
    assert!(
        checks[1]
            .findings
            .iter()
            .any(|s| s.contains("symbolic link target"))
    );
}
#[test]
fn worktree_switch_repairs_real_git_worktree_and_inverse() {
    use std::process::Command;
    let p = scratch();
    let repo = p.join("repo");
    let old = p.join("old");
    let new = p.join("new");
    let run = |args: &[&str]| {
        assert!(Command::new("git").args(args).status().unwrap().success());
    };
    run(&["init", repo.to_str().unwrap()]);
    run(&[
        "-C",
        repo.to_str().unwrap(),
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@invalid",
        "commit",
        "--allow-empty",
        "-m",
        "fixture",
    ]);
    run(&[
        "-C",
        repo.to_str().unwrap(),
        "worktree",
        "add",
        "--detach",
        old.to_str().unwrap(),
    ]);
    fs::rename(&old, &new).unwrap();
    let set = p.join("set.tsv");
    write(
        &set,
        &format!(
            "seq\tkind\tlive\tstaged\tinverse\treason\n1\tworktree\t{}\t{}\t{}\trepair\n",
            repo.display(),
            new.display(),
            old.display()
        ),
    );
    let j = p.join("j.tsv");
    migration::switch_set(&set, &p.join("before"), &j, false).unwrap();
    assert_eq!(
        fs::read_to_string(repo.join(".git/worktrees/old/gitdir"))
            .unwrap()
            .trim(),
        new.join(".git").to_str().unwrap()
    );
    fs::rename(&new, &old).unwrap();
    migration::inverse(&j).unwrap();
    assert_eq!(
        fs::read_to_string(repo.join(".git/worktrees/old/gitdir"))
            .unwrap()
            .trim(),
        old.join(".git").to_str().unwrap()
    );
}

#[test]
fn partial_move_failure_retains_reversible_prior_acts() {
    let p = scratch();
    let a = p.join("a");
    let b = p.join("b");
    let c = p.join("c");
    let d = p.join("d");
    write(&a, "first");
    write(&c, "second");
    write(&d, "occupied");
    let m = map(&p, &[("1", &a, &b), ("2", &c, &d)]);
    let j = p.join("journal.tsv");
    assert!(
        migration::move_map(&m, &j, false)
            .unwrap_err()
            .to_string()
            .contains("seq 2")
    );
    assert_eq!(migration::inverse(&j).unwrap(), 1);
    assert_eq!(fs::read_to_string(a).unwrap(), "first");
    assert_eq!(fs::read_to_string(c).unwrap(), "second");
    assert_eq!(fs::read_to_string(d).unwrap(), "occupied");
}
#[test]
fn pending_forward_and_interrupted_inverse_resume_without_clobber() {
    let p = scratch();
    let a = p.join("a");
    let b = p.join("b");
    let c = p.join("c");
    write(&a, "payload");
    let m = map(&p, &[("1", &a, &b), ("2", &b, &c)]);
    let j = p.join("j.tsv");
    migration::move_map(&m, &j, false).unwrap();
    let body = fs::read_to_string(&j)
        .unwrap()
        .replace("\tapplied\t", "\tpending\t");
    write(&j, &body);
    write(&a, "occupied");
    assert!(migration::inverse(&j).is_err());
    assert_eq!(fs::read_to_string(&b).unwrap(), "payload");
    let undo = j.with_extension("inverse.tsv");
    let interrupted = fs::read_to_string(&undo)
        .unwrap()
        .replace("\tapplied\t", "\tpending\t");
    write(&undo, &interrupted);
    fs::rename(&a, p.join("occupied.saved")).unwrap();
    assert_eq!(migration::inverse(&j).unwrap(), 1);
    assert_eq!(fs::read_to_string(a).unwrap(), "payload");
    assert_eq!(migration::inverse(&j).unwrap(), 0);
}
#[test]
fn failed_switch_never_changes_live_file() {
    let p = scratch();
    let live = p.join("live");
    write(&live, "unchanged");
    let set = p.join("set.tsv");
    write(
        &set,
        &format!(
            "seq\tkind\tlive\tstaged\tinverse\treason\n1\tfile\t{}\t{}\tjournal\ttest\n",
            live.display(),
            p.join("missing").display()
        ),
    );
    let j = p.join("journal.tsv");
    assert!(migration::switch_set(&set, &p.join("before"), &j, false).is_err());
    assert_eq!(fs::read_to_string(live).unwrap(), "unchanged");
    assert!(!j.exists()); // refused before every act, including journal creation
}
#[test]
fn missing_record_census_is_a_failure() {
    let p = scratch();
    fs::create_dir(p.join("06_memory")).unwrap();
    write(
        &p.join("ovrin.build.json"),
        r#"{"roots":[{"role":"memory","dir":"06_memory"}],"records":{"entries":[{"role":"memory","paths":["**"]}]},"consumers":[]}"#,
    );
    let checks = preflight::run(&p.join("ovrin.build.json"), "post").unwrap();
    assert_eq!(checks[7].status, "FAIL");
    assert!(checks[7].findings[0].contains("records.census missing"));
}

#[test]
fn enclosing_retired_name_is_not_a_retired_link_target() {
    let p = scratch().join("04_memory").join("namespace");
    fs::create_dir_all(&p).unwrap();
    fs::create_dir(p.join("06_memory")).unwrap();
    std::os::unix::fs::symlink(p.join("06_memory"), p.join("current")).unwrap();
    write(
        &p.join("ovrin.build.json"),
        r#"{"roots":[{"role":"memory","dir":"06_memory"}],"retired_roots":{"04_memory":"06_memory"},"consumers":["current"]}"#,
    );
    let checks = preflight::run(&p.join("ovrin.build.json"), "post").unwrap();
    assert_eq!(checks[1].status, "PASS");
    assert_eq!(checks[1].checked, 1);
}

#[test]
fn dry_move_tracks_children_after_parent_moves() {
    let p = scratch();
    let old = p.join("old");
    let hold = p.join("hold");
    fs::create_dir(&old).unwrap();
    write(&old.join("child"), "original");
    let out = p.join("child");
    let m = map(&p, &[("1", &old, &hold), ("2", &hold.join("child"), &out)]);
    let journal = p.join("journal.tsv");
    assert_eq!(migration::move_map(&m, &journal, true).unwrap(), 2);
    assert!(!journal.exists());
    assert!(old.join("child").is_file());
    assert!(!hold.exists());
    assert_eq!(migration::move_map(&m, &journal, false).unwrap(), 2);
    assert_eq!(fs::read_to_string(out).unwrap(), "original");
}
