use crate::{
    Result,
    document::{bounded, whole},
    io,
    manifest::Namespace,
    read,
};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
#[derive(Debug, Serialize)]
pub struct Check {
    pub id: String,
    pub name: String,
    pub status: String,
    pub checked: usize,
    pub findings: Vec<String>,
    pub resolved_from: Vec<crate::paths::Resolution>,
}
impl Check {
    fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            status: "PASS".into(),
            checked: 0,
            findings: vec![],
            resolved_from: Vec::new(),
        }
    }
    fn finish(&mut self, info: bool) {
        self.status = if info {
            "INFO"
        } else if self.findings.is_empty() {
            "PASS"
        } else {
            "FAIL"
        }
        .into();
    }
}
struct Context<'a> {
    ns: &'a Namespace,
    mode: &'a str,
    list: Option<PathBuf>,
    incoming: Vec<PathBuf>,
    arriving: Vec<String>,
    resolved: Vec<crate::paths::Resolution>,
}
impl<'a> Context<'a> {
    fn new(ns: &'a Namespace, mode: &'a str) -> Result<Self> {
        let mut context = Self {
            ns,
            mode,
            list: None,
            incoming: Vec::new(),
            arriving: Vec::new(),
            resolved: Vec::new(),
        };
        let consumers = &ns.value["consumers"];
        if let (Some(role), Some(path)) = (consumers["role"].as_str(), consumers["path"].as_str()) {
            let primary = ns.role(role)?.join(path);
            let mut list = context.resolve(&primary);
            if !list.exists()
                && let Some(source) = consumers["staging_source"]["path"].as_str()
            {
                let candidate = crate::paths::home(ns).join(source);
                let candidate = crate::paths::existing(ns, &candidate);
                if candidate.is_file() {
                    context.resolved.push(crate::paths::Resolution {
                        resolved_from: primary,
                        path: candidate.clone(),
                    });
                    list = candidate;
                }
            }
            if mode == "staged" {
                let map = list.parent().unwrap_or(&ns.root).join("MOVE_IN.tsv");
                if map.is_file() {
                    let text = read(&map)?;
                    let mut lines = text.lines();
                    let header: Vec<_> = lines.next().unwrap_or("").split('\t').collect();
                    let column = header.iter().position(|s| *s == "to").ok_or_else(|| {
                        crate::Error::Invalid("move-in map needs a to column".into())
                    })?;
                    for (line, row) in lines.enumerate() {
                        let cells: Vec<_> = row.split('\t').collect();
                        let to = cells.get(column).ok_or_else(|| {
                            crate::Error::Invalid(format!(
                                "{}:{}: short move-in row",
                                map.display(),
                                line + 2
                            ))
                        })?;
                        context.incoming.push(context.project(Path::new(to)));
                    }
                }
            }
            context.list = Some(list);
        }
        Ok(context)
    }
    fn project(&self, p: &Path) -> PathBuf {
        if p.starts_with(&self.ns.root) {
            return p.into();
        }
        if let Ok(relative) = p.strip_prefix(crate::paths::home(self.ns)) {
            return self.ns.root.join(relative);
        }
        p.into()
    }
    fn resolve(&mut self, p: &Path) -> PathBuf {
        let mut actual = crate::paths::existing(self.ns, p);
        if !actual.exists()
            && self.mode == "staged"
            && let Ok(relative) = p.strip_prefix(&self.ns.root)
        {
            let host = crate::paths::home(self.ns).join(relative);
            let candidate = crate::paths::existing(self.ns, &host);
            // Resolve only a true renamed-root counterpart, not an unrelated missing staged kernel file.
            if candidate != host && candidate.exists() {
                actual = candidate;
            }
        }

        if actual != p {
            self.resolved.push(crate::paths::Resolution {
                resolved_from: p.into(),
                path: actual.clone(),
            });
        }
        actual
    }
    fn present(&mut self, p: &Path, why: &str) -> bool {
        if p.exists() {
            return true;
        }
        let projected = self.project(p);
        if self.mode == "staged"
            && let Some(destination) = self
                .incoming
                .iter()
                .find(|to| projected == **to || projected.starts_with(to))
        {
            self.arriving.push(format!(
                "{}: {why}; arriving at move-in via {}",
                p.display(),
                destination.display()
            ));
            return true;
        }
        self.resolve(p).exists()
    }
}
fn path_fields(v: &Value, key: &str, out: &mut Vec<(String, String)>) {
    if key == "planned" {
        return;
    }
    if let Some(s) = v.as_str() {
        let leaf = key.rsplit('.').next().unwrap_or(key);
        if matches!(
            leaf,
            "root" | "documentation" | "specification" | "generated_root" | "verification_root"
        ) || key.starts_with("registries.")
            || key.starts_with("compiler.")
        {
            out.push((key.into(), s.into()));
        }
    } else if let Some(m) = v.as_object() {
        for (k, v) in m {
            path_fields(
                v,
                &if key.is_empty() {
                    k.clone()
                } else {
                    format!("{key}.{k}")
                },
                out,
            )
        }
    }
}
fn numbered(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() > 3
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && (b[2] == b'_' || (b[2].is_ascii_uppercase() && b.get(3) == Some(&b'_')))
}
fn children(p: &Path) -> Result<Vec<PathBuf>> {
    if !p.is_dir() {
        return Ok(vec![]);
    }
    io(p, fs::read_dir(p))?
        .map(|e| io(p, e).map(|e| e.path()))
        .collect()
}
fn consumer_walk(p: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let m = io(p, fs::symlink_metadata(p))?;
    if m.file_type().is_symlink() {
        out.push(p.into());
        return Ok(());
    }
    if m.is_file() {
        out.push(p.into());
        return Ok(());
    }
    for c in children(p)? {
        if matches!(
            c.file_name().and_then(|s| s.to_str()),
            Some(
                "vendor"
                    | "node_modules"
                    | "target"
                    | ".git"
                    | "sessions"
                    | "projects"
                    | "cache"
                    | "__pycache__"
                    | "site-packages"
            )
        ) {
            continue;
        }
        consumer_walk(&c, out)?;
    }
    Ok(())
}
fn consumers(ns: &Namespace, context: &mut Context<'_>) -> Result<(Vec<PathBuf>, Vec<String>)> {
    let mut roots = vec![];
    let mut errors = vec![];
    match &ns.value["consumers"] {
        Value::Array(a) => {
            for v in a {
                if let Some(s) = v.as_str().or_else(|| v["path"].as_str()) {
                    roots.push(ns.root.join(s));
                }
            }
        }
        Value::Object(v) => {
            if let (Some(role), Some(path)) = (
                v.get("role").and_then(Value::as_str),
                v.get("path").and_then(Value::as_str),
            ) {
                let list = context.list.clone().unwrap_or(ns.role(role)?.join(path));
                if !list.exists() {
                    errors.push(format!("{}: consumer list is not present", list.display()));
                } else {
                    let t = read(&list)?;
                    let mut lines = t.lines();
                    let header: Vec<_> = lines.next().unwrap_or("").split('\t').collect();
                    let col = header.iter().position(|s| *s == "live").ok_or_else(|| {
                        crate::Error::Invalid("consumers TSV needs live column".into())
                    })?;
                    let staged = header.iter().position(|s| *s == "staged");
                    let kind = header.iter().position(|s| *s == "kind");
                    for line in lines {
                        let cells: Vec<_> = line.split('\t').collect();
                        let Some(live) = cells.get(col) else {
                            continue;
                        };
                        if context.mode == "staged"
                            && let (Some(staged), Some(kind)) = (staged, kind)
                        {
                            let kind = cells.get(kind).copied().unwrap_or("");
                            let prepared = cells.get(staged).copied().unwrap_or("");
                            if prepared.is_empty() {
                                errors.push(format!("{}: empty staged consumer", list.display()));
                                continue;
                            }
                            if matches!(kind, "link" | "release") {
                                let target = expand(&ns.root, prepared);
                                if !context.present(&target, "planned consumer link target") {
                                    errors.push(format!(
                                        "{}: planned link target missing",
                                        target.display()
                                    ));
                                }
                            } else if kind == "key" {
                                let source = expand(&ns.root, live);
                                if !context.present(&source, "planned key rename source") {
                                    errors
                                        .push(format!("{}: key source missing", source.display()));
                                }
                            } else if kind == "worktree" {
                                // Paths are repaired by the later switch; validate their named source only.
                                let source = expand(&ns.root, live);
                                if !context.present(&source, "worktree source") {
                                    errors.push(format!(
                                        "{}: worktree source missing",
                                        source.display()
                                    ));
                                }
                            } else {
                                roots.push(expand(&ns.root, prepared));
                            }
                        } else {
                            roots.push(expand(&ns.root, live));
                        }
                    }
                }
            }
        }
        _ => {
            for s in [
                ".local/bin",
                ".config/systemd/user",
                ".config/hypr",
                ".config/omarchy",
                ".config/zeno",
                "Work/AGENTS.md",
                ".codex/AGENTS.md",
                ".claude/CLAUDE.md",
            ] {
                let p = ns.root.join(s);
                if p.exists() {
                    roots.push(p)
                }
            }
            for p in children(&ns.root.join(".codex"))? {
                if p.extension().is_some_and(|e| e == "toml") {
                    roots.push(p)
                }
            }
            for app in children(&ns.root.join(".local/lib"))? {
                let current = app.join("current");
                if current.symlink_metadata().is_ok() {
                    roots.push(current.clone());
                    if let Ok(active) = fs::canonicalize(&current) {
                        if active.is_file() {
                            roots.push(active);
                        } else if active.join("bin").is_dir() {
                            roots.push(active.join("bin"));
                        } else {
                            for p in children(&active)? {
                                if p.is_file() {
                                    roots.push(p)
                                }
                            }
                        }
                    }
                }
            }
            for p in children(&ns.root.join(".local/bin"))? {
                if p.symlink_metadata()
                    .is_ok_and(|m| m.file_type().is_symlink())
                    && let Ok(target) = fs::canonicalize(&p)
                    && target.is_file()
                {
                    roots.push(target);
                }
            }
        }
    }
    let mut out = vec![];
    for p in roots {
        let p = context.resolve(&p);
        if ns.is_record(&p) {
            continue;
        }
        if p.symlink_metadata().is_err() {
            errors.push(format!("{}: declared consumer missing", p.display()));
            continue;
        }
        consumer_walk(&p, &mut out)?;
    }
    out.sort();
    out.dedup();
    out.retain(|p| !ns.is_record(p));
    Ok((out, errors))
}
fn expand(root: &Path, s: &str) -> PathBuf {
    let home = root.to_string_lossy();
    let s = s
        .replace("%h", &home)
        .replace("${HOME}", &home)
        .replace("$HOME", &home);
    if let Some(t) = s.strip_prefix("~/") {
        root.join(t)
    } else {
        let p = PathBuf::from(s);
        if p.is_absolute() { p } else { root.join(p) }
    }
}
fn link_target(root: &Path, file: &Path, target: &str) -> Option<PathBuf> {
    let target = target.trim().trim_matches(['\'', '"', '<', '>']);
    if target.is_empty()
        || target.starts_with('#')
        || target.contains("://") && !target.starts_with("file://")
    {
        return None;
    }
    let target = target
        .strip_prefix("file://")
        .or_else(|| target.strip_prefix("file:"))
        .unwrap_or(target)
        .split('#')
        .next()
        .unwrap_or("");
    if target.contains('%') && !target.contains("%h") {
        return None;
    }
    if target.starts_with('/')
        || target.starts_with('~')
        || target.contains("%h")
        || target.contains("$HOME")
    {
        Some(expand(root, target))
    } else {
        Some(file.parent().unwrap_or(root).join(target))
    }
}
pub fn run(manifest: &Path, mode: &str) -> Result<Vec<Check>> {
    let ns = Namespace::load(manifest)?;
    let roots = ns.roots();
    let mut context = Context::new(&ns, mode)?;
    let mut checks = vec![
        Check::new("P1", "manifest ↔ disk"),
        Check::new("P2", "retired names"),
        Check::new("P3", "links resolve"),
        Check::new("P4", "root names"),
        Check::new("P5", "user-folder literals"),
        Check::new("P6", "host side"),
        Check::new("P7", "quiet"),
        Check::new("P8", "record census"),
        Check::new("P9", "arriving at move-in"),
    ];
    let declared: BTreeSet<_> = roots.iter().map(|(_, p)| p.clone()).collect();
    let disk: Vec<_> = children(&ns.root)?
        .into_iter()
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s == "docs" || numbered(s))
        })
        .collect();
    for p in &declared {
        checks[0].checked += 1;
        if !context.present(p, "declared root") {
            checks[0]
                .findings
                .push(format!("{}: declared root missing", p.display()));
        }
    }
    for p in &disk {
        if !declared.contains(p) {
            checks[0]
                .findings
                .push(format!("{}: root on disk, not in manifest", p.display()));
        }
    }
    let projects: Vec<_> = ns.value["projects"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, v)| v["root"].as_str())
        .map(|s| ns.root.join(s))
        .collect();
    let divisions: BTreeSet<_> = projects
        .iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .collect();
    for d in divisions {
        for p in children(&d)?.into_iter().filter(|p| p.is_dir()) {
            checks[0].checked += 1;
            if !projects.contains(&p) && !ns.is_record(&p) {
                checks[0].findings.push(format!(
                    "{}: division child on disk, not in manifest",
                    p.display()
                ));
            }
        }
    }
    let mut paths = vec![];
    path_fields(&ns.value, "", &mut paths);
    for (key, s) in paths {
        if s == "home" || s.contains('<') {
            continue;
        }
        checks[0].checked += 1;
        let p = ns.root.join(s);
        if !context.present(&p, &format!("declared by {key}")) {
            checks[0]
                .findings
                .push(format!("{}: declared by {key}, missing", p.display()));
        }
    }
    // Concrete landing places and record containers share P1's path invariant.
    if let Some(a) = ns.value["landing"].as_array() {
        for e in a {
            if let (Some(role), Some(path)) = (e["role"].as_str(), e["path"].as_str()) {
                if path.contains('<') || matches!(role, "host" | "project") {
                    continue;
                }
                let p = ns.role(role)?.join(path);
                checks[0].checked += 1;
                if !context.present(&p, "landing place") {
                    checks[0]
                        .findings
                        .push(format!("{}: landing place missing", p.display()));
                }
            }
        }
    }
    if let Some(a) = ns
        .value
        .pointer("/records/entries")
        .and_then(Value::as_array)
    {
        for e in a {
            if e["role"] == "host" {
                continue;
            }
            let base = ns.role(e["role"].as_str().unwrap_or(""))?;
            for g in e["paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let prefix = g.split('*').next().unwrap_or("").trim_end_matches('/');
                if prefix.is_empty() {
                    continue;
                }
                let p = base.join(prefix);
                checks[0].checked += 1;
                if !context.present(&p, "record container") {
                    checks[0]
                        .findings
                        .push(format!("{}: record container missing", p.display()));
                }
            }
        }
    }
    let (consumers, errors) = consumers(&ns, &mut context)?;
    checks[1].findings.extend(errors);
    let mut retired = vec![];
    if let Some(m) = ns.value["retired_roots"].as_object() {
        for k in m.keys() {
            if mode != "pre" || !ns.root.join(k).exists() {
                retired.push(k.clone())
            }
        }
    } else if !ns.root.join("02_divisions").exists() {
        retired.push("02_divisions".into());
    }
    if let Some(a) = ns
        .value
        .pointer("/identity_contract/retired_project_keys")
        .and_then(Value::as_array)
    {
        retired.extend(a.iter().filter_map(Value::as_str).map(str::to_owned));
    }
    let literals = Regex::new(r"/home/[^/\s]+")?;
    for p in &consumers {
        if p.symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            checks[1].checked += 1;
            checks[2].checked += 1;
            let target = io(p, fs::read_link(p))?;
            let target = if target.is_absolute() {
                target
            } else {
                p.parent().unwrap_or(&ns.root).join(target)
            };
            let mut normalized = PathBuf::new();
            for component in target.components() {
                match component {
                    std::path::Component::ParentDir => {
                        normalized.pop();
                    }
                    std::path::Component::CurDir => {}
                    other => normalized.push(other.as_os_str()),
                }
            }
            for key in &retired {
                if normalized.strip_prefix(&ns.root).is_ok_and(|relative| {
                    relative.components().any(|c| c.as_os_str() == key.as_str())
                }) {
                    checks[1]
                        .findings
                        .push(format!("{}: symbolic link target names {key}", p.display()));
                }
            }
            if !context.present(&normalized, "symbolic link target") {
                checks[2]
                    .findings
                    .push(format!("{}: broken symbolic link", p.display()));
            }
            continue;
        }
        let bytes = io(p, fs::read(p))?;
        let binary = bytes.iter().take(8192).any(|b| *b == 0);
        let text = String::from_utf8_lossy(&bytes);
        checks[1].checked += 1;
        checks[4].checked += 1;
        for key in &retired {
            if binary {
                let re = regex::bytes::Regex::new(&regex::escape(key))?;
                for m in re.find_iter(&bytes) {
                    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
                    if m.start() > 0 && word(bytes[m.start() - 1])
                        || m.end() < bytes.len() && word(bytes[m.end()])
                    {
                        continue;
                    }
                    let start = bytes[..m.start()]
                        .iter()
                        .rposition(|b| *b == 0 || b.is_ascii_whitespace())
                        .map_or(0, |x| x + 1);
                    let end = bytes[m.end()..]
                        .iter()
                        .position(|b| *b == 0 || b.is_ascii_whitespace())
                        .map_or(bytes.len(), |x| m.end() + x);
                    if bytes[start..end].contains(&b'/') {
                        checks[1].findings.push(format!(
                            "{}:byte{}: {key}",
                            p.display(),
                            m.start()
                        ));
                    }
                }
            } else {
                for m in bounded(key, false)?.find_iter(&text) {
                    if whole(&text, m.start(), m.end()) {
                        let line = text[..m.start()].bytes().filter(|b| *b == b'\n').count() + 1;
                        checks[1]
                            .findings
                            .push(format!("{}:{line}: {key}", p.display()));
                    }
                }
            }
        }
        let n = literals.find_iter(&text).count();
        if n > 0 {
            checks[4]
                .findings
                .push(format!("{}: {n} user-folder literals", p.display()));
        }
        if !binary
            && matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("service" | "path" | "socket" | "timer" | "desktop")
            )
        {
            for (line, s) in text.lines().enumerate() {
                let Some((key, val)) = s.split_once('=') else {
                    continue;
                };
                let key = key.trim();
                let exec = matches!(
                    key,
                    "ExecStart"
                        | "ExecStartPre"
                        | "ExecStartPost"
                        | "ExecReload"
                        | "ExecStop"
                        | "Exec"
                );
                let paths = matches!(
                    key,
                    "WorkingDirectory"
                        | "PathChanged"
                        | "PathModified"
                        | "ReadWritePaths"
                        | "Path"
                        | "Documentation"
                );
                if !exec && !paths {
                    continue;
                }
                let mut tokens = val.split_whitespace();
                let targets: Vec<_> = if exec {
                    tokens.next().into_iter().collect()
                } else {
                    tokens.collect()
                };
                for target in targets {
                    let target = target.trim_start_matches(['-', '+', '!', '@', ':']);
                    if exec && !target.contains('/') {
                        continue;
                    }
                    if let Some(t) = link_target(&crate::paths::home(&ns), p, target) {
                        checks[2].checked += 1;
                        if !context.present(&t, "unit/desktop target") {
                            checks[2].findings.push(format!(
                                "{}:{}: {} missing",
                                p.display(),
                                line + 1,
                                t.display()
                            ));
                        }
                    }
                }
            }
        }
    }
    let md = Regex::new(r"\]\(([^)\n]+)\)")?;
    let mut readmes: Vec<_> = roots.iter().map(|(_, p)| p.join("README.md")).collect();
    readmes.push(ns.root.join("01_governance/laws/namespace.md"));
    for p in readmes {
        if !p.exists() {
            continue;
        }
        for (line, s) in read(&p)?.lines().enumerate() {
            for c in md.captures_iter(s) {
                if let Some(t) = link_target(&ns.root, &p, &c[1]) {
                    checks[2].checked += 1;
                    if !context.present(&t, "Markdown link target") {
                        checks[2].findings.push(format!(
                            "{}:{}: {} missing",
                            p.display(),
                            line + 1,
                            t.display()
                        ));
                    }
                }
            }
        }
    }
    let normal = Regex::new(r"^[0-9]{2}_[a-z]+$")?;
    let mut numbers = BTreeMap::new();
    for p in &disk {
        let s = p.file_name().unwrap_or_default().to_string_lossy();
        if s == "docs" {
            continue;
        }
        checks[3].checked += 1;
        if !normal.is_match(&s) {
            checks[3]
                .findings
                .push(format!("{s}: root has a suffix or invalid spelling"));
        }
        if let Some(old) = numbers.insert(s[..2].to_string(), s.to_string()) {
            checks[3]
                .findings
                .push(format!("{s}: number shared with {old}"));
        }
    }
    let host: BTreeSet<_> = ns
        .value
        .pointer("/host/entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.as_str().or_else(|| e["entry"].as_str()))
        .collect();
    for p in children(&ns.root)? {
        checks[5].checked += 1;
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        if !declared.contains(&p) && !host.contains(name.as_ref()) && name != "ovrin.build.json" {
            checks[5]
                .findings
                .push(format!("{}: host entry not listed", p.display()));
        }
    }
    // Quiet is informational before migration. Staging has no live processes.
    if mode == "post" {
        for proc in children(Path::new("/proc"))? {
            if !proc
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
            {
                continue;
            }
            for link in std::iter::once(proc.join("cwd"))
                .chain(children(&proc.join("fd")).unwrap_or_default())
            {
                if let Ok(target) = fs::read_link(&link) {
                    checks[6].checked += 1;
                    if roots.iter().any(|(_, r)| target.starts_with(r)) {
                        checks[6].findings.push(format!(
                            "{}: {} held open",
                            link.display(),
                            target.display()
                        ));
                    }
                }
            }
        }
    } else {
        checks[6]
            .findings
            .push("not gating before migration; no process scan".into());
    }
    if mode == "post" {
        record_census(&ns, &mut checks[7])?;
    } else {
        checks[7]
            .findings
            .push("record census gates post mode only".into());
    }
    checks[8].checked = context.arriving.len();
    checks[8].findings = context.arriving;
    checks[8].resolved_from = context.resolved;
    for (i, c) in checks.iter_mut().enumerate() {
        c.finish(
            i == 8
                || i == 4
                || i == 5
                || (i == 3 && mode == "pre")
                || (i == 6 && mode != "post")
                || (i == 7 && mode != "post"),
        );
    }
    Ok(checks)
}

/// Each census row names a concrete moved record container, not a wildcard.
/// Format: path<TAB>files<TAB>bytes. The manifest's records.census locates it.
fn record_census(ns: &Namespace, check: &mut Check) -> Result<()> {
    let location = &ns.value["records"]["census"];
    let census = if let Some(s) = location.as_str() {
        ns.root.join(s)
    } else if let (Some(role), Some(path)) = (location["role"].as_str(), location["path"].as_str())
    {
        ns.role(role)?.join(path)
    } else {
        if ns.value["records"]["entries"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            check
                .findings
                .push("records.census missing; record counts cannot be proved".into());
        }
        return Ok(());
    };
    if !census.is_file() {
        check
            .findings
            .push(format!("{}: census missing", census.display()));
        return Ok(());
    }
    let text = read(&census)?;
    let mut lines = text.lines();
    if lines.next() != Some("path\tfiles\tbytes") {
        return Err(crate::Error::Invalid(
            "record census needs path, files, bytes TSV columns".into(),
        ));
    }
    fn count(path: &Path) -> Result<(u64, u64)> {
        let m = io(path, fs::symlink_metadata(path))?;
        if m.file_type().is_symlink() {
            return Ok((1, io(path, fs::read_link(path))?.as_os_str().len() as u64));
        }
        if m.is_file() {
            return Ok((1, m.len()));
        }
        let mut total = (0, 0);
        for p in children(path)? {
            let n = count(&p)?;
            total.0 += n.0;
            total.1 += n.1;
        }
        Ok(total)
    }
    let mut seen = BTreeSet::new();
    for (line, row) in lines.enumerate() {
        let cells: Vec<_> = row.split('\t').collect();
        if cells.len() != 3 {
            return Err(crate::Error::Invalid(format!(
                "{}:{}: invalid census row",
                census.display(),
                line + 2
            )));
        }
        let p = expand(&ns.root, cells[0]);
        if !p.starts_with(&ns.root)
            || p.components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            || !ns.is_record(&p)
        {
            return Err(crate::Error::Invalid(format!(
                "{}: census path must be a declared record",
                p.display()
            )));
        }
        if !seen.insert(p.clone()) {
            return Err(crate::Error::Invalid("duplicate census path".into()));
        }
        let files = cells[1]
            .parse::<u64>()
            .map_err(|_| crate::Error::Invalid("invalid census files".into()))?;
        let bytes = cells[2]
            .parse::<u64>()
            .map_err(|_| crate::Error::Invalid("invalid census bytes".into()))?;
        check.checked += 1;
        match count(&p) {
            Ok(actual) if actual == (files, bytes) => {}
            Ok(actual) => check.findings.push(format!(
                "{}: census expected {files} files/{bytes} bytes; found {} files/{} bytes",
                p.display(),
                actual.0,
                actual.1
            )),
            Err(e) => check.findings.push(e.to_string()),
        }
    }
    if let Some(entries) = ns.value["records"]["entries"].as_array() {
        for entry in entries {
            if entry["role"] == "host" {
                continue;
            }
            let base = ns.role(entry["role"].as_str().unwrap_or(""))?;
            for pattern in entry["paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let prefix = base.join(
                    pattern
                        .split('*')
                        .next()
                        .unwrap_or("")
                        .trim_end_matches('/'),
                );
                if !seen.iter().any(|p| prefix == *p || prefix.starts_with(p)) {
                    check.findings.push(format!(
                        "{}: record container lacks a covering census row",
                        prefix.display()
                    ));
                }
            }
        }
    }
    if check.checked == 0 {
        check
            .findings
            .push("empty census cannot prove record counts".into());
    }
    Ok(())
}
