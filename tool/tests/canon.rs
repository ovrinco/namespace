use ovrin_ns::{
    canon::{self, Canon, Class},
    manifest::Namespace,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
const MANIFEST: &str = r#"{
  "roots": [
    {"role": "governance", "dir": "01_governance"},
    {"role": "ovrin", "dir": "02_ovrin"},
    {"role": "memory", "dir": "06_memory"},
    {"role": "stratum", "dir": "80_stratum"},
    {"role": "docs", "dir": "docs"}
  ],
  "projects": {
    "soliton": {"root": "02_ovrin/soliton", "documentation": "docs/ovrin/soliton"}
  },
  "landing": [
    {"kind": "campaign_outputs", "role": "memory", "path": "runs/<project>/<campaign>/<task>"},
    {"kind": "notes_inbox", "role": "memory", "path": "notes/inbox"},
    {"kind": "documentation", "role": "docs", "path": "<division>/<project>"}
  ],
  "index": {"roles": {
    "stratum": [
      {"paths": ["**/README*"]},
      {"paths": ["06_assets"]},
      {"paths": ["models"]}
    ],
    "memory": [{"paths": ["tracker/**"], "except_paths": ["tracker/store"]}]
  }},
  "host": {"staging": {"entry": "namespace_next"}, "entries": [{"entry": "scratch"}]},
  "retired_roots": {"02A_ovrin": "02_ovrin"}
}"#;
const DIRS: [&str; 22] = [
    "01_governance/skills",
    "02_ovrin/soliton/src",
    "02_ovrin/stray",
    "06_memory/runs/x",
    "06_memory/notes/inbox",
    "06_memory/tracker/store",
    "80_stratum/06_assets",
    "80_stratum/models",
    "80_stratum/runtimes",
    "docs/ovrin/soliton",
    "docs/ideas",
    ".config",
    "Documents",
    "scratch",
    "02A_ovrin",
    "namespace_next",
    "Work",
    "01_governance/skills/deep",
    "02_ovrin/soliton/src/inner",
    "80_stratum/models/m",
    "06_memory/chat",
    "scratch/inside",
];
fn fixture() -> PathBuf {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let root = base.join(format!(
        "canon-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    for d in DIRS {
        fs::create_dir_all(root.join(d)).unwrap();
    }
    fs::write(root.join("ovrin.build.json"), MANIFEST).unwrap();
    fs::write(root.join("song.mp3"), b"").unwrap();
    fs::write(root.join("80_stratum/README.md"), b"").unwrap();
    root
}
fn load(root: &Path) -> Canon {
    Canon::new(&Namespace::load(&root.join("ovrin.build.json")).unwrap()).unwrap()
}
#[test]
fn representative_cases() {
    let root = fixture();
    let c = load(&root);
    for (p, class) in [
        ("01_governance", Class::Root),
        ("02_ovrin/soliton", Class::Declared),
        ("docs/ovrin/soliton", Class::Declared),
        ("docs/ovrin", Class::Declared),
        ("06_memory/runs", Class::Declared),
        ("06_memory/notes", Class::Declared),
        ("06_memory/notes/inbox", Class::Declared),
        ("80_stratum/models", Class::Declared),
        ("02_ovrin/soliton/src", Class::Nested),
        ("02_ovrin/soliton/src/inner", Class::Nested),
        ("02_ovrin/stray", Class::Nested),
        ("01_governance/skills/deep", Class::Nested),
        ("80_stratum/models/m", Class::Nested),
        ("80_stratum/README.md", Class::Nested),
        ("06_memory/tracker/store", Class::Exception),
        ("80_stratum/06_assets", Class::Exception),
        ("80_stratum/runtimes", Class::Foreign),
        ("scratch", Class::Foreign),
        ("scratch/inside", Class::Foreign),
        ("02A_ovrin", Class::Foreign),
        ("Work", Class::Foreign),
        ("song.mp3", Class::Foreign),
        (".config", Class::Host),
        ("Documents", Class::Host),
        ("ovrin.build.json", Class::Host),
        ("namespace_next", Class::Host),
    ] {
        assert_eq!(c.classify(&root.join(p)).class, class, "{p}");
    }
    assert_eq!(c.classify(Path::new("/elsewhere")).class, Class::Foreign);
    let found: Vec<_> = c
        .foreign()
        .unwrap()
        .into_iter()
        .map(|e| {
            (
                e.path.strip_prefix(&root).unwrap().display().to_string(),
                e.host_listed,
            )
        })
        .collect();
    let want = [
        ("02A_ovrin", false),
        ("80_stratum/06_assets", false),
        ("80_stratum/runtimes", false),
        ("Work", false),
        ("scratch", true),
        ("song.mp3", false),
    ];
    assert_eq!(
        found,
        want.map(|(p, l)| (p.to_string(), l)).to_vec(),
        "only top-level strays are listed, never canon content"
    );
    let folders: Vec<_> = c
        .folders()
        .into_iter()
        .map(|p| p.strip_prefix(&root).unwrap().display().to_string())
        .collect();
    assert_eq!(
        folders,
        [
            "01_governance",
            "02_ovrin",
            "02_ovrin/soliton",
            "06_memory",
            "06_memory/notes",
            "06_memory/notes/inbox",
            "06_memory/runs",
            "80_stratum",
            "80_stratum/models",
            "docs",
            "docs/ovrin",
            "docs/ovrin/soliton",
        ]
    );
    fs::remove_dir_all(&root).unwrap();
}
/// The same cases on the installed manifest of the home folder, read only.
#[test]
fn representative_cases_on_the_home_manifest() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let m = home.join("ovrin.build.json");
    if !m.is_file() {
        return;
    }
    let c = Canon::new(&Namespace::load(&m).unwrap()).unwrap();
    for (p, class) in [
        ("01_governance", Class::Root),
        ("02_ovrin/soliton", Class::Declared),
        ("02_ovrin/soliton/src", Class::Nested),
        ("80_stratum/models", Class::Declared),
        ("80_stratum/06_assets", Class::Exception),
        ("scratch", Class::Foreign),
        (".config", Class::Host),
    ] {
        assert_eq!(c.classify(&home.join(p)).class, class, "{p}");
    }
}
fn gio_available() -> bool {
    Command::new("gio")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}
fn snapshot(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in paths {
        let o = Command::new("gio")
            .args(["info", "-a", "metadata::*"])
            .arg(p)
            .output()
            .unwrap();
        out.extend(o.stdout);
    }
    out
}
#[test]
fn apply_then_undo_restores_metadata_byte_for_byte() {
    if !gio_available() {
        eprintln!("gio missing: colour test skipped");
        return;
    }
    let root = fixture();
    let c = load(&root);
    let icon = root.join("yellow.png");
    fs::write(&icon, b"").unwrap();
    let icon = icon.to_str().unwrap().to_string();
    let want = canon::coloured(&icon);
    let set = |p: &Path, k: &str, v: &str| {
        assert!(
            Command::new("gio")
                .arg("set")
                .arg(p)
                .args([k, v])
                .status()
                .unwrap()
                .success()
        )
    };
    // A canon folder with a picture name of his own, and a stray still carrying the colour.
    set(&root.join("02_ovrin"), canon::KEYS[1], "folder-music");
    set(
        &root.join("scratch"),
        canon::KEYS[0],
        want[0].as_deref().unwrap(),
    );
    let all: Vec<_> = DIRS
        .iter()
        .map(|d| root.join(d))
        .chain(
            [
                "01_governance",
                "02_ovrin",
                "06_memory",
                "80_stratum",
                "docs",
            ]
            .map(|d| root.join(d)),
        )
        .chain(c.folders())
        .collect();
    let before = snapshot(&all);
    let plan = canon::plan(&c, &icon).unwrap();
    let record = root.join("colour-applied.tsv");
    let n = canon::apply(&plan, &record).unwrap();
    assert_eq!(n, c.folders().len() + 1);
    for p in c.folders() {
        assert_eq!(canon::meta(&p).unwrap(), want, "{}", p.display());
    }
    assert_eq!(canon::meta(&root.join("scratch")).unwrap(), [None, None]);
    for p in [
        "02_ovrin/soliton/src",
        "80_stratum/06_assets",
        "80_stratum/runtimes",
        "Work",
    ] {
        assert_eq!(canon::meta(&root.join(p)).unwrap(), [None, None], "{p}");
    }
    // A second apply changes nothing and the record must be new.
    let again = canon::plan(&c, &icon).unwrap();
    assert!(again.iter().all(|ch| ch.action == "kept"));
    assert!(canon::apply(&again, &record).is_err());
    canon::undo(&record).unwrap();
    assert_eq!(snapshot(&all), before, "metadata restored byte for byte");
    // A folder changed after the apply is refused, not overwritten.
    let record2 = root.join("colour-applied-2.tsv");
    canon::apply(&canon::plan(&c, &icon).unwrap(), &record2).unwrap();
    set(&root.join("docs"), canon::KEYS[1], "folder-videos");
    assert!(canon::undo(&record2).is_err());
    for p in all {
        for k in canon::KEYS {
            Command::new("gio")
                .args(["set", "-t", "unset"])
                .arg(&p)
                .arg(k)
                .status()
                .unwrap();
        }
    }
    fs::remove_dir_all(&root).unwrap();
}
