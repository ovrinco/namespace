use ovrin_ns::{
    manifest::Namespace,
    paths::{self, Journals},
};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
fn write(p: &Path, s: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}
fn act(from: &Path, to: &Path) -> String {
    json!({"from":from,"to":to,"fingerprint":"file:0:100644","copy":false,"resolved_from":{}})
        .to_string()
}
/// Every file below `p` with its bytes, so a run can be shown to have written nothing.
fn snapshot(p: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out: Vec<_> = ovrin_ns::files(&[p.to_path_buf()])
        .unwrap()
        .into_iter()
        .map(|f| {
            let bytes = fs::read(&f).unwrap();
            (f, bytes)
        })
        .collect();
    out.sort();
    out
}
fn fixture(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("resolve-{name}-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    write(
        &root.join("ovrin.build.json"),
        &json!({
            "roots":[{"role":"memory","dir":"06_memory"}],
            "retired_roots":{"04_memory":"06_memory","02_divisions":null},
            "move_ledger":{"role":"memory","path":"j/MOVE_JOURNAL.tsv","journals":["j/*.journal.tsv"]}
        })
        .to_string(),
    );
    let m = root.join("06_memory");
    // Unchanged, and reached through the retired root.
    write(&m.join("a.md"), "a");
    // A chain of two moves: x/one.md went to y/one.md, then y went to 06_memory/z.
    write(&m.join("z/one.md"), "one");
    write(
        &m.join("j/MOVE_JOURNAL.tsv"),
        &format!(
            "time\tresult\tact\n1\tapplied\t{}\n",
            act(&root.join("x/one.md"), &root.join("y/one.md"))
        ),
    );
    write(
        &m.join("j/second.journal.tsv"),
        &format!(
            "time\tresult\tact\n2\tapplied\t{}\n3\tapplied\t{}\n",
            act(&root.join("y"), &m.join("z")),
            act(&m.join("old"), &m.join("new"))
        ),
    );
    write(&m.join("new/f.md"), "f");
    // A rolled-back move is never followed.
    write(
        &m.join("j/second.journal.inverse.tsv"),
        &format!(
            "time\tresult\tact\n4\tapplied\t{}\n",
            act(&root.join("gone.md"), &m.join("a.md"))
        ),
    );
    root
}
#[test]
fn resolves_roots_journal_chains_and_reports_missing() {
    let root = fixture("lib");
    let m = root.join("06_memory");
    let ns = Namespace::load(&root.join("ovrin.build.json")).unwrap();
    let j = Journals::load(&ns, &[]).unwrap();
    assert_eq!(j.files.len(), 2, "{:?}", j.files);
    let first = m.join("j/MOVE_JOURNAL.tsv");
    let second = m.join("j/second.journal.tsv");

    let same = paths::resolve(&ns, &j, &m.join("a.md"));
    assert_eq!(same.path, Some(m.join("a.md")));
    assert_eq!(same.how, ["exists"]);

    let retired = paths::resolve(&ns, &j, &root.join("04_memory/a.md"));
    assert_eq!(retired.path, Some(m.join("a.md")));
    assert_eq!(retired.how, ["root"]);

    let chain = paths::resolve(&ns, &j, &root.join("x/one.md"));
    assert_eq!(chain.path, Some(m.join("z/one.md")));
    assert_eq!(
        chain.how,
        [
            format!("journal {}:2", first.display()),
            format!("journal {}:2", second.display())
        ]
    );

    let both = paths::resolve(&ns, &j, &root.join("04_memory/old/f.md"));
    assert_eq!(both.path, Some(m.join("new/f.md")));
    assert_eq!(
        both.how,
        [
            "root".to_string(),
            format!("journal {}:3", second.display())
        ]
    );

    for gone in ["04_memory/missing.md", "02_divisions/x.md", "gone.md"] {
        let f = paths::resolve(&ns, &j, &root.join(gone));
        assert_eq!(f.path, None, "{gone}");
    }
    fs::remove_dir_all(&root).unwrap();
}
#[test]
fn command_exit_codes_json_stdin_and_writes_nothing() {
    let root = fixture("cli");
    let manifest = root.join("ovrin.build.json");
    let before = snapshot(&root);
    let bin = env!("CARGO_BIN_EXE_ovrin-ns");
    let run = |args: &[&str]| {
        Command::new(bin)
            .arg("resolve")
            .arg("--manifest")
            .arg(&manifest)
            .args(args)
            .output()
            .unwrap()
    };
    let old = root.join("04_memory/a.md");
    let ok = run(&[old.to_str().unwrap()]);
    assert_eq!(ok.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(ok.stdout).unwrap(),
        format!("{}\troot\n", root.join("06_memory/a.md").display())
    );

    let missing = root.join("04_memory/missing.md");
    let some = run(&["--json", old.to_str().unwrap(), missing.to_str().unwrap()]);
    assert_eq!(some.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&some.stdout).unwrap();
    assert_eq!(v[0]["how"], json!(["root"]));
    assert_eq!(v[1]["path"], serde_json::Value::Null);
    assert_eq!(v[1]["input"], json!(missing));

    let mut child = Command::new(bin)
        .args(["resolve", "--stdin", "--manifest"])
        .arg(&manifest)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{}\n\n{}",
        old.display(),
        missing.display()
    )
    .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!(
            "{}\troot\nnot found\t{}\n",
            root.join("06_memory/a.md").display(),
            missing.display()
        )
    );

    write(&root.join("bad/ovrin.build.json"), "{not json");
    let bad = Command::new(bin)
        .args(["resolve", "--manifest"])
        .arg(root.join("bad/ovrin.build.json"))
        .arg(&old)
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
    fs::remove_dir_all(root.join("bad")).unwrap();

    assert_eq!(snapshot(&root), before, "resolve wrote inside the tree");
    fs::remove_dir_all(&root).unwrap();
}
