//! The canon of the namespace: which folders the manifest owns, and their one shared colour in the file manager.
//! Classification is pure over the manifest and the path; only `foreign` and the colour functions read the disk.
use crate::{Error, Result, io, manifest::Namespace};
use globset::{GlobBuilder, GlobMatcher};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

/// The colour of the canon is yellow; today it is the yellow theme's folder picture, set per folder.
pub const ICON: &str = "/usr/share/icons/Yaru-yellow/256x256/places/folder.png";
/// The file manager's per-folder metadata keys that decide a folder's picture.
pub const KEYS: [&str; 2] = ["metadata::custom-icon", "metadata::custom-icon-name"];
/// The desktop's standard folders at the home root (the XDG defaults).
const STANDARD: [&str; 8] = [
    "Desktop",
    "Documents",
    "Downloads",
    "Music",
    "Pictures",
    "Public",
    "Templates",
    "Videos",
];
/// Harness guidance read from the home root by the agents' programs.
const GUIDANCE: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// One of the manifest's top-level roots.
    Root,
    /// A subtree the manifest declares: a project, its documentation, a landing place, a stratum member.
    Declared,
    /// Declared by the manifest, yet against its convention; never coloured.
    Exception,
    /// Ordinary content of a canon folder; never foreign.
    Nested,
    /// A known entry of the host: hidden program entries, the desktop's standard folders, the manifest.
    Host,
    /// Unknown to the manifest.
    Foreign,
}
impl Class {
    pub fn canon(self) -> bool {
        matches!(self, Self::Root | Self::Declared)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Verdict {
    pub class: Class,
    pub reason: String,
}
fn verdict(class: Class, reason: impl Into<String>) -> Verdict {
    Verdict {
        class,
        reason: reason.into(),
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub path: PathBuf,
    pub kind: &'static str,
    pub class: Class,
    pub reason: String,
    /// Whether the manifest's host list names the entry (it is left in place, not canon).
    pub host_listed: bool,
}

pub struct Canon {
    root: PathBuf,
    roots: BTreeMap<PathBuf, String>,
    declared: BTreeMap<PathBuf, String>,
    excepted: BTreeSet<PathBuf>,
    stratum: Option<PathBuf>,
    stratum_rules: Vec<(String, GlobMatcher)>,
    retired: BTreeSet<String>,
    staging: Option<String>,
    host_listed: BTreeSet<String>,
}

/// A name carrying a root number (`06_`, `02A_`); numbers belong to the namespace roots alone.
pub fn numbered(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() > 3
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && (b[2] == b'_' || (b[2].is_ascii_uppercase() && b.get(3) == Some(&b'_')))
}
fn literal(s: &str) -> bool {
    !s.is_empty() && !s.contains(['*', '?', '[', '{', '<'])
}
fn lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}
fn name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl Canon {
    pub fn new(ns: &Namespace) -> Result<Self> {
        let v = &ns.value;
        let roots: BTreeMap<_, _> = ns.roots().into_iter().map(|(r, p)| (p, r)).collect();
        let mut declared = BTreeMap::new();
        for (key, p) in v["projects"].as_object().into_iter().flatten() {
            for (field, what) in [
                ("root", "project"),
                ("documentation", "project documentation"),
            ] {
                if let Some(s) = p[field].as_str().filter(|s| literal(s)) {
                    declared.insert(ns.root.join(s), format!("{what} {key}"));
                }
            }
        }
        for l in v["landing"].as_array().into_iter().flatten() {
            let (Some(role), Some(path)) = (l["role"].as_str(), l["path"].as_str()) else {
                continue;
            };
            // A placeholder (`runs/<project>/...`) still declares the literal folders before it.
            let head: Vec<_> = path.split('/').take_while(|c| literal(c)).collect();
            if let (Ok(base), false) = (ns.role(role), head.is_empty()) {
                let kind = l["kind"].as_str().unwrap_or("landing");
                declared.insert(base.join(head.join("/")), format!("landing place {kind}"));
            }
        }
        let mut excepted = BTreeSet::new();
        let mut stratum_rules = Vec::new();
        for (role, rules) in v
            .pointer("/index/roles")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let Ok(base) = ns.role(role) else { continue };
            for rule in rules.as_array().into_iter().flatten() {
                for e in rule["except_paths"].as_array().into_iter().flatten() {
                    if let Some(s) = e.as_str().filter(|s| literal(s)) {
                        excepted.insert(base.join(s));
                    }
                }
                if role != "stratum" {
                    continue;
                }
                for p in rule["paths"].as_array().into_iter().flatten() {
                    let s = p
                        .as_str()
                        .ok_or_else(|| Error::Invalid("index glob is not a string".into()))?;
                    let g = GlobBuilder::new(s).literal_separator(true).build()?;
                    stratum_rules.push((s.to_string(), g.compile_matcher()));
                    if literal(s) && !s.contains('/') {
                        declared.insert(base.join(s), "stratum member".into());
                    }
                }
            }
        }
        // The folders on the way from a root to a declared folder are declared with it.
        let mut ways = BTreeMap::new();
        for (p, what) in &declared {
            for a in p.ancestors().skip(1) {
                if roots.contains_key(a) || !a.starts_with(&ns.root) || a == ns.root {
                    break;
                }
                ways.entry(a.to_path_buf())
                    .or_insert_with(|| format!("on the way to {what}"));
            }
        }
        for (p, what) in ways {
            declared.entry(p).or_insert(what);
        }
        let retired = v["retired_roots"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, _)| k.clone())
            .collect();
        let host_listed = v
            .pointer("/host/entries")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| e["entry"].as_str().map(String::from))
            .collect();
        Ok(Self {
            root: ns.root.clone(),
            stratum: ns.role("stratum").ok(),
            staging: v
                .pointer("/host/staging/entry")
                .and_then(Value::as_str)
                .map(String::from),
            roots,
            declared,
            excepted,
            stratum_rules,
            retired,
            host_listed,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn stratum(&self) -> Option<&Path> {
        self.stratum.as_deref()
    }
    /// Whether the manifest's stratum layout names this direct member of the stratum.
    fn stratum_known(&self, member: &str) -> bool {
        self.stratum_rules
            .iter()
            .any(|(s, g)| g.is_match(member) || s.split('/').next() == Some(member))
    }
    pub fn classify(&self, path: &Path) -> Verdict {
        let p = lexical(path);
        let Ok(rel) = p.strip_prefix(&self.root) else {
            return verdict(Class::Foreign, "outside the namespace root");
        };
        let parts: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let Some(first) = parts.first() else {
            return verdict(Class::Host, "the namespace root itself");
        };
        let top = self.root.join(first);
        if let Some(role) = self.roots.get(&top) {
            if parts.len() == 1 {
                return verdict(Class::Root, format!("root {role}"));
            }
            if self.excepted.contains(&p) {
                return verdict(Class::Exception, "excepted by the manifest's index");
            }
            if self.stratum.as_deref() == Some(top.as_path()) {
                let member = &parts[1];
                if numbered(member) {
                    let r = "carries a root number inside the stratum; numbers belong to the namespace roots";
                    return verdict(
                        Class::Exception,
                        if parts.len() == 2 {
                            r.into()
                        } else {
                            format!("inside {member}, which {r}")
                        },
                    );
                }
                if !self.stratum_known(member) {
                    let r = "not in the manifest's stratum layout";
                    return verdict(
                        Class::Foreign,
                        if parts.len() == 2 {
                            r.into()
                        } else {
                            format!("inside {member}, {r}")
                        },
                    );
                }
            }
            if let Some(what) = self.declared.get(&p) {
                if numbered(parts.last().map_or("", String::as_str)) {
                    return verdict(
                        Class::Exception,
                        format!("{what} carries a root number below the root level"),
                    );
                }
                return verdict(Class::Declared, what.clone());
            }
            return verdict(Class::Nested, format!("content of root {role}"));
        }
        let (kind, inside) = if parts.len() == 1 {
            ("", "")
        } else {
            ("inside ", first.as_str())
        };
        let say = |r: &str| {
            if inside.is_empty() {
                r.to_string()
            } else {
                format!("{kind}{inside}: {r}")
            }
        };
        if self.retired.contains(first) {
            return verdict(Class::Foreign, say("a retired root name"));
        }
        if first.starts_with('.') {
            return verdict(Class::Host, say("hidden program entry"));
        }
        if STANDARD.contains(&first.as_str()) {
            return verdict(Class::Host, say("standard desktop folder"));
        }
        if parts.len() == 1 && first == "ovrin.build.json" {
            return verdict(Class::Host, "the manifest");
        }
        if parts.len() == 1 && GUIDANCE.contains(&first.as_str()) {
            return verdict(Class::Host, "harness guidance");
        }
        if self.staging.as_deref() == Some(first.as_str()) {
            return verdict(Class::Host, say("the manifest's migration staging"));
        }
        verdict(Class::Foreign, say("not known to the manifest"))
    }
    /// The top-level entries of the namespace root and of the stratum that are foreign or a declared exception.
    pub fn foreign(&self) -> Result<Vec<Entry>> {
        let mut out = Vec::new();
        for dir in std::iter::once(self.root.as_path()).chain(self.stratum.as_deref()) {
            for e in io(dir, fs::read_dir(dir))? {
                let p = io(dir, e)?.path();
                let v = self.classify(&p);
                if matches!(v.class, Class::Foreign | Class::Exception) {
                    let m = io(&p, fs::symlink_metadata(&p))?;
                    let kind = if m.file_type().is_symlink() {
                        "link"
                    } else if m.is_dir() {
                        "directory"
                    } else {
                        "file"
                    };
                    out.push(Entry {
                        host_listed: dir == self.root && self.host_listed.contains(&name(&p)),
                        path: p,
                        kind,
                        class: v.class,
                        reason: v.reason,
                    });
                }
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }
    /// Every canon folder present on disk as a real directory (links are never coloured).
    pub fn folders(&self) -> Vec<PathBuf> {
        let mut out: Vec<_> = self
            .roots
            .keys()
            .chain(self.declared.keys())
            .filter(|p| self.classify(p).class.canon())
            .filter(|p| fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()))
            .cloned()
            .collect();
        out.sort();
        out.dedup();
        out
    }
    /// Folders whose children are searched for a stale canon colour: the namespace root and every canon folder's parent.
    fn neighbourhoods(&self) -> Vec<PathBuf> {
        let mut out: BTreeSet<PathBuf> = BTreeSet::from([self.root.clone()]);
        for p in self.roots.keys().chain(self.declared.keys()) {
            for a in p.ancestors().skip(1) {
                if !a.starts_with(&self.root) {
                    break;
                }
                out.insert(a.into());
            }
        }
        out.into_iter().filter(|p| p.is_dir()).collect()
    }
}

/// The file manager's picture keys of one path, as stored: `None` where unset.
pub type Meta = [Option<String>; 2];
fn gio(args: &[&str]) -> Result<String> {
    let out = Command::new("gio")
        .args(args)
        .output()
        .map_err(|source| Error::Io {
            path: "gio".into(),
            source,
        })?;
    if !out.status.success() {
        return Err(Error::Invalid(format!(
            "gio {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    String::from_utf8(out.stdout).map_err(|_| Error::Invalid("gio printed non-UTF-8".into()))
}
fn text(p: &Path) -> Result<&str> {
    p.to_str()
        .filter(|s| s.starts_with('/'))
        .ok_or_else(|| Error::Invalid(format!("{}: not an absolute UTF-8 path", p.display())))
}
pub fn meta(p: &Path) -> Result<Meta> {
    let out = gio(&["info", "-a", &KEYS.join(","), text(p)?])?;
    let mut m: Meta = [None, None];
    let mut attributes = false;
    for line in out.lines() {
        if line == "attributes:" {
            attributes = true;
            continue;
        }
        if !attributes {
            continue;
        }
        for (i, k) in KEYS.iter().enumerate() {
            if let Some(v) = line.strip_prefix(&format!("  {k}: ")) {
                m[i] = Some(v.to_string());
            }
        }
    }
    Ok(m)
}
fn put(p: &Path, m: &Meta) -> Result<()> {
    for (k, v) in KEYS.iter().zip(m) {
        match v {
            Some(v) => gio(&["set", text(p)?, k, v])?,
            None => gio(&["set", "-t", "unset", text(p)?, k])?,
        };
    }
    let now = meta(p)?;
    if &now != m {
        return Err(Error::Invalid(format!(
            "{}: metadata did not take",
            p.display()
        )));
    }
    Ok(())
}
pub fn coloured(icon: &str) -> Meta {
    [Some(format!("file://{icon}")), None]
}

#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub path: PathBuf,
    pub action: &'static str,
    pub before: Meta,
    pub after: Meta,
}
/// What applying the colour would change: canon folders not yet coloured, and non-canon folders still carrying it.
pub fn plan(canon: &Canon, icon: &str) -> Result<Vec<Change>> {
    let want = coloured(icon);
    let mut out = Vec::new();
    let folders = canon.folders();
    for p in &folders {
        let before = meta(p)?;
        let action = if before == want { "kept" } else { "colour" };
        out.push(Change {
            path: p.clone(),
            action,
            after: want.clone(),
            before,
        });
    }
    let folders: BTreeSet<_> = folders.into_iter().collect();
    for dir in canon.neighbourhoods() {
        let listing = gio(&["list", "-a", KEYS[0], text(&dir)?])?;
        let marked = format!("{}={}", KEYS[0], want[0].as_deref().unwrap_or_default());
        for line in listing.lines() {
            let fields: Vec<_> = line.split('\t').collect();
            if !fields.contains(&marked.as_str()) {
                continue;
            }
            let n = fields[0];
            let p = dir.join(n);
            if folders.contains(&p) {
                continue;
            }
            let before = meta(&p)?;
            if before[0] == want[0] {
                out.push(Change {
                    path: p,
                    action: "clear",
                    after: [None, before[1].clone()],
                    before,
                });
            }
        }
    }
    Ok(out)
}
fn cell(m: &Option<String>) -> String {
    serde_json::to_string(m).unwrap_or_else(|_| "null".into())
}
pub const RECORD_HEADER: &str = "path\tbefore_icon\tbefore_icon_name\tafter_icon\tafter_icon_name";
/// Apply a plan. Each row of the undo record is written and synced before its folder changes; the record must be new.
pub fn apply(changes: &[Change], record: &Path) -> Result<usize> {
    use std::io::Write;
    let mut f = io(
        record,
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(record),
    )?;
    io(record, writeln!(f, "{RECORD_HEADER}"))?;
    let mut n = 0;
    for c in changes.iter().filter(|c| c.action != "kept") {
        let path = serde_json::to_string(&text(&c.path)?)?;
        let row = [
            path,
            cell(&c.before[0]),
            cell(&c.before[1]),
            cell(&c.after[0]),
            cell(&c.after[1]),
        ];
        io(record, writeln!(f, "{}", row.join("\t")))?;
        io(record, f.sync_data())?;
        put(&c.path, &c.after)?;
        n += 1;
    }
    Ok(n)
}
fn parse(s: &str) -> Result<Option<String>> {
    Ok(serde_json::from_str(s)?)
}
/// Undo a record, last row first. A folder whose metadata changed since the apply is refused, not overwritten.
pub fn undo(record: &Path) -> Result<Vec<Change>> {
    let body = crate::read(record)?;
    let mut lines = body.lines();
    if lines.next() != Some(RECORD_HEADER) {
        return Err(Error::Invalid(format!(
            "{}: not a colour record",
            record.display()
        )));
    }
    let mut rows = Vec::new();
    for (i, line) in lines.enumerate() {
        let f: Vec<_> = line.split('\t').collect();
        if f.len() != 5 {
            return Err(Error::Invalid(format!(
                "{}: row {} has {} cells",
                record.display(),
                i + 1,
                f.len()
            )));
        }
        let path: String = serde_json::from_str(f[0])?;
        rows.push(Change {
            path: path.into(),
            action: "restore",
            before: [parse(f[3])?, parse(f[4])?],
            after: [parse(f[1])?, parse(f[2])?],
        });
    }
    for c in &rows {
        let now = meta(&c.path)?;
        if now != c.before {
            return Err(Error::Invalid(format!(
                "{}: metadata changed since the apply; nothing restored",
                c.path.display()
            )));
        }
    }
    for c in rows.iter().rev() {
        put(&c.path, &c.after)?;
    }
    Ok(rows)
}
