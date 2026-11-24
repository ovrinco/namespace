use ovrin_ns::{
    drift,
    manifest::Namespace,
    releases::{self, State},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn write(p: &Path, s: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}
fn sha(s: &str) -> String {
    releases::sha256(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?}");
}
const LAW: &str = "# Namespace Law\n\n## Root Map\n\n| Root | Purpose |\n|---|---|\n| `00_constitution` | Identity |\n| `03_citium` | Citium at `03_citium/alpha`; only these |\n\n## Citium Topology\n\n- `alpha`\n\n## Placement Gate\n";
const SPEC: &str = "# Spec\n\n## Authored institutional namespace\n\n```text\n/home/x/\n├── 00_constitution/\n└── 03_citium/\n```\n";
const RULES: &str = "# Rules\n\n## Institutional Layout\n- `00_constitution/`: canon.\n- `03_citium/alpha/`: alpha.\n- `Work/`: his folder; holds `tries/`.\n\n## Next\n";
/// A namespace with two roots, two Citium projects, the four drift sources and a release index.
fn fixture() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "j06-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&p).unwrap();
    write(
        &p.join("ovrin.build.json"),
        r#"{"roots":[{"role":"constitution","dir":"00_constitution"},{"role":"citium","dir":"03_citium"}],
"projects":{"alpha":{"root":"03_citium/alpha"},"beta":{"root":"03_citium/beta"}},
"retired_roots":{},"records":{"entries":[]},"consumers":[],"host":{"entries":[{"entry":"Work","kind":"directory"},{"entry":"lib","kind":"directory"},{"entry":"other","kind":"directory"},{"entry":"rules.md","kind":"file"},{"entry":"law.md","kind":"file"},{"entry":"spec.md","kind":"file"},{"entry":"index.toml","kind":"file"},{"entry":"listed.toml","kind":"file"}]},
"release_index":{"path":"index.toml"},
"drift_check":{"law":"law.md","layout_spec":"spec.md","house_rules":"rules.md","listed":"listed.toml"}}"#,
    );
    write(&p.join("00_constitution/x.md"), "x\n");
    write(&p.join("law.md"), LAW);
    write(&p.join("spec.md"), SPEC);
    write(&p.join("rules.md"), RULES);
    fs::create_dir_all(p.join("Work")).unwrap();
    // alpha: a git project whose release names a commit.
    let alpha = p.join("03_citium/alpha");
    write(&alpha.join("src/main.rs"), "fn main() {}\n");
    git(&alpha, &["init", "-q"]);
    git(&alpha, &["add", "."]);
    git(&alpha, &["commit", "-qm", "one"]);
    let head = String::from_utf8(
        Command::new("git")
            .arg("-C")
            .arg(&alpha)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    write(
        &p.join("lib/alpha/current/release.json"),
        &format!(r#"{{"commit":"{}"}}"#, head.trim()),
    );
    // beta: a plain folder whose release maps files to hashes, and a manifest-form release with a link.
    let beta = p.join("03_citium/beta");
    write(&beta.join("a.txt"), "a\n");
    std::os::unix::fs::symlink("a.txt", beta.join("LICENSE")).unwrap();
    write(
        &p.join("lib/beta/current/release.json"),
        &format!(r#"{{"source":{{"a.txt":"{}"}}}}"#, sha("a\n")),
    );
    write(
        &p.join("lib/beta-tsv/current/SOURCE_MANIFEST.one.tsv"),
        &format!(
            "# source_json\t\"{}\"\n# symlinks_json\t[\"LICENSE\"]\npath\tsize\tsha256\na.txt\t2\t{}\nLICENSE\t5\t{}\n",
            beta.display(),
            sha("a\n"),
            sha("a.txt")
        ),
    );
    write(
        &p.join("index.toml"),
        r#"
[[tool]]
name = "alpha"
release = "lib/alpha/current"
folder = "03_citium/alpha"
owner = "alpha"
record = "release.json"
identity = "git_commit"
field = "commit"

[[tool]]
name = "beta"
release = "lib/beta/current"
folder = "03_citium/beta"
owner = "beta"
record = "release.json"
identity = "file_sha256"
field = "source"
elsewhere = "other"

[[tool]]
name = "beta-tsv"
release = "lib/beta-tsv/current"
folder = "03_citium/beta"
owner = "beta"
identity = "tsv_manifests"
"#,
    );
    p
}
fn state(ns: &Namespace, name: &str) -> State {
    let index = releases::load(ns).unwrap();
    let t = index.tools.iter().find(|t| t.name == name).unwrap();
    releases::locate(ns, t).unwrap().state
}
#[test]
fn each_release_is_found_from_its_project_folder() {
    let p = fixture();
    let ns = Namespace::load(&p.join("ovrin.build.json")).unwrap();
    assert_eq!(state(&ns, "alpha"), State::CheckedOut);
    assert_eq!(state(&ns, "beta"), State::CheckedOut);
    assert_eq!(state(&ns, "beta-tsv"), State::CheckedOut);
    let index = releases::load(&ns).unwrap();
    let here = releases::select(&ns, &index, Some(&p.join("03_citium/beta/a.txt")));
    assert_eq!(
        here.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["beta", "beta-tsv"]
    );
    // A later commit leaves the release in the folder's history, not checked out.
    write(&p.join("03_citium/alpha/src/main.rs"), "fn main() { }\n");
    git(&p.join("03_citium/alpha"), &["commit", "-qam", "two"]);
    assert_eq!(state(&ns, "alpha"), State::InHistory);
    // A changed file: not in the folder; then found at the recorded other place.
    write(&p.join("03_citium/beta/a.txt"), "changed\n");
    assert_eq!(state(&ns, "beta"), State::NotFound);
    write(&p.join("other/a.txt"), "a\n");
    assert_eq!(state(&ns, "beta"), State::Elsewhere);
    assert!(!State::Elsewhere.in_folder());
}
#[test]
fn drift_names_each_disagreement_and_its_listed_owner() {
    let p = fixture();
    let manifest = p.join("ovrin.build.json");
    let keys = |sections: &[drift::Section]| {
        sections
            .iter()
            .flat_map(|s| s.drift.iter().map(|d| d.key.clone()))
            .collect::<Vec<_>>()
    };
    let found = keys(&drift::run(&manifest).unwrap());
    let beta = "the registry declares project 03_citium/beta; not named in the law";
    assert!(found.contains(&beta.to_string()), "{found:#?}");
    assert!(
        found
            .iter()
            .any(|k| k.contains("03_citium/beta; not named in the house rules"))
    );
    assert!(
        !found.iter().any(|k| k.contains("layout spec")),
        "{found:#?}"
    );
    assert!(!found.iter().any(|k| k.contains("tries")), "{found:#?}");
    assert!(
        !found.iter().any(|k| k.starts_with("release ")),
        "{found:#?}"
    );
    // A stray root folder on disk is drift; a listed item carries its owner.
    fs::create_dir(p.join("05_stray")).unwrap();
    write(
        &p.join("listed.toml"),
        &format!(
            "[[listed]]\ndrift = \"{beta}\"\nowner = \"governance\"\nwhy = \"law edit proposed\"\n"
        ),
    );
    let sections = drift::run(&manifest).unwrap();
    assert!(keys(&sections).iter().any(|k| k.contains("05_stray")));
    let listed: Vec<_> = sections
        .iter()
        .flat_map(|s| &s.drift)
        .filter_map(|d| d.listed.as_ref().map(|l| (d.key.clone(), l.owner.clone())))
        .collect();
    assert_eq!(listed, [(beta.to_string(), "governance".to_string())]);
    // Agreement everywhere: no drift.
    fs::create_dir_all(p.join("other")).unwrap();
    fs::rename(p.join("05_stray"), p.join("other/05_stray")).unwrap();
    write(
        &p.join("law.md"),
        &LAW.replace(
            "`03_citium/alpha`;",
            "`03_citium/alpha` and Beta at `03_citium/beta`;",
        )
        .replace("- `alpha`\n", "- `alpha`\n- `beta`\n"),
    );
    write(
        &p.join("rules.md"),
        &RULES.replace("- `Work/`", "- `03_citium/beta/`: beta.\n- `Work/`"),
    );
    assert_eq!(keys(&drift::run(&manifest).unwrap()), Vec::<String>::new());
}
