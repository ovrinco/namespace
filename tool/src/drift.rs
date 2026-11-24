//! One drift check: the registry, the namespace law, the layout specification and the house rules, each against the
//! registry and the disk, and every installed release against its project folder.
use crate::{Error, Result, canon::Canon, manifest::Namespace, preflight, read, releases};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Serialize)]
pub struct Section {
    pub name: &'static str,
    pub source: String,
    pub checked: usize,
    pub drift: Vec<Drift>,
}

#[derive(Serialize, Clone)]
pub struct Drift {
    /// Stable words for the item; a listed item names the same words.
    pub key: String,
    pub listed: Option<Listed>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Listed {
    pub drift: String,
    pub owner: String,
    pub why: String,
}

#[derive(Deserialize, Default)]
struct ListedFile {
    #[serde(default)]
    listed: Vec<Listed>,
}

fn source(ns: &Namespace, key: &str) -> Result<std::path::PathBuf> {
    ns.value
        .pointer(&format!("/drift_check/{key}"))
        .and_then(Value::as_str)
        .map(|s| ns.root.join(s))
        .ok_or_else(|| Error::Invalid(format!("the manifest has no drift_check.{key}")))
}

/// Root folder names and project paths the registry declares.
struct Registry {
    roots: BTreeSet<String>,
    projects: BTreeSet<String>,
}
impl Registry {
    fn new(ns: &Namespace) -> Self {
        let roots = ns
            .roots()
            .into_iter()
            .filter_map(|(_, p)| {
                p.strip_prefix(&ns.root)
                    .ok()
                    .map(|r| r.display().to_string())
            })
            .collect();
        let projects = ns.value["projects"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, v)| v["root"].as_str().map(str::to_string))
            .collect();
        Self { roots, projects }
    }
    fn in_division(&self, division: &str) -> BTreeSet<String> {
        self.projects
            .iter()
            .filter(|p| p.split('/').next() == Some(division))
            .cloned()
            .collect()
    }
}

fn compare_sets(
    out: &mut Vec<String>,
    what: &str,
    named: &BTreeSet<String>,
    declared: &BTreeSet<String>,
    source: &str,
) {
    for n in named.difference(declared) {
        out.push(format!("{source} names {what} {n}; not in the registry"));
    }
    for d in declared.difference(named) {
        out.push(format!(
            "the registry declares {what} {d}; not named in the {source}"
        ));
    }
}

pub fn run(manifest: &Path) -> Result<Vec<Section>> {
    let ns = Namespace::load(manifest)?;
    let reg = Registry::new(&ns);
    let mut sections = Vec::new();

    // Registry against the disk: the structural preflight checks, and the top-level entries nobody declared.
    let mut keys = Vec::new();
    let mut checked = 0;
    for c in preflight::run(manifest, "post")? {
        if matches!(c.id.as_str(), "P1" | "P2" | "P3" | "P4") {
            checked += c.checked;
            keys.extend(c.findings.iter().map(|f| format!("disk: {f}")));
        }
    }
    for e in Canon::new(&ns)?.foreign()? {
        checked += 1;
        if !e.host_listed
            && !keys
                .iter()
                .any(|k| k.contains(&e.path.display().to_string()))
        {
            keys.push(format!("disk: {}: {}", e.path.display(), e.reason));
        }
    }
    sections.push(section("registry ↔ disk", manifest, checked, keys));

    // The law's root map, its division memberships and the Citium topology, against the registry.
    let law = source(&ns, "law")?;
    let text = read(&law)?;
    let mut keys = Vec::new();
    let row = Regex::new(r"(?m)^\| `([^`]+)` \|(.*)\|\s*$")?;
    let at = Regex::new(r"at `([^`]+)`")?;
    let mut law_roots = BTreeSet::new();
    let mut members: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for c in row.captures_iter(&text) {
        let root = c[1].trim_end_matches('/').to_string();
        for m in at.captures_iter(&c[2]) {
            members
                .entry(root.clone())
                .or_default()
                .insert(m[1].trim_end_matches('/').to_string());
        }
        law_roots.insert(root);
    }
    compare_sets(&mut keys, "root", &law_roots, &reg.roots, "law");
    for (division, named) in &members {
        compare_sets(
            &mut keys,
            "project",
            named,
            &reg.in_division(division),
            "law",
        );
    }
    if let Some(topo) = text.split("## Citium Topology").nth(1) {
        let topo = topo.split("\n## ").next().unwrap_or("");
        let item = Regex::new(r"(?m)^- `([^`]+)`")?;
        let named: BTreeSet<_> = item
            .captures_iter(topo)
            .map(|c| format!("03_citium/{}", &c[1]))
            .collect();
        compare_sets(
            &mut keys,
            "project",
            &named,
            &reg.in_division("03_citium"),
            "law (Citium topology)",
        );
    }
    let checked = law_roots.len() + members.values().map(BTreeSet::len).sum::<usize>();
    sections.push(section("law ↔ registry", &law, checked, dedup(keys)));

    // The layout specification's tree of authored roots against the registry.
    let spec = source(&ns, "layout_spec")?;
    let text = read(&spec)?;
    let mut keys = Vec::new();
    let tree = text
        .split("## Authored institutional namespace")
        .nth(1)
        .and_then(|s| s.split("```text").nth(1))
        .and_then(|s| s.split("```").next())
        .ok_or_else(|| Error::Invalid(format!("{}: no tree of authored roots", spec.display())))?;
    let leaf = Regex::new(r"(?m)^[├└]── ([^/\s]+)/")?;
    let named: BTreeSet<_> = leaf.captures_iter(tree).map(|c| c[1].to_string()).collect();
    compare_sets(&mut keys, "root", &named, &reg.roots, "layout spec");
    sections.push(section("layout spec ↔ registry", &spec, named.len(), keys));

    // The house rules' institutional layout: every path it names exists, and it names every root and project.
    let rules = source(&ns, "house_rules")?;
    let text = read(&rules)?;
    let mut keys = Vec::new();
    let layout = text
        .split("## Institutional Layout")
        .nth(1)
        .map(|s| s.split("\n## ").next().unwrap_or(""))
        .ok_or_else(|| Error::Invalid(format!("{}: no institutional layout", rules.display())))?;
    let code = Regex::new(r"`([^`\s]+)`")?;
    let (mut roots, mut projects) = (BTreeSet::new(), BTreeSet::new());
    let mut checked = 0;
    for c in code.captures_iter(layout) {
        let p = c[1].trim_end_matches('/');
        if !p.contains('/') && !p.ends_with(".json") && !c[1].ends_with('/') {
            continue;
        }
        let first = p.split('/').next().unwrap_or(p);
        // Only paths from the namespace root: a registry root, a retired root, or a top-level entry.
        if !(reg.roots.contains(first)
            || ns.value["retired_roots"].get(first).is_some()
            || p.ends_with(".json")
            || ns.root.join(first).exists())
        {
            continue;
        }
        checked += 1;
        if !p.contains('*') && !ns.root.join(p).exists() {
            keys.push(format!("house rules name {p}, missing on disk"));
        }
        let parts: Vec<_> = p.split('/').collect();
        if reg.roots.contains(parts[0]) {
            roots.insert(parts[0].to_string());
            if parts.len() >= 2
                && reg
                    .projects
                    .iter()
                    .any(|r| r.split('/').next() == Some(parts[0]))
            {
                let two = format!("{}/{}", parts[0], parts[1]);
                if parts.len() == 2 || reg.projects.contains(&two) {
                    projects.insert(two);
                }
            }
        }
    }
    for r in reg.roots.difference(&roots) {
        keys.push(format!(
            "the registry declares root {r}; not named in the house rules"
        ));
    }
    compare_sets(
        &mut keys,
        "project",
        &projects,
        &reg.projects,
        "house rules",
    );
    sections.push(section(
        "house rules ↔ registry",
        &rules,
        checked,
        dedup(keys),
    ));

    // Every installed release against its project folder.
    let index = releases::load(&ns)?;
    let mut keys = Vec::new();
    for t in &index.tools {
        let f = releases::locate(&ns, t)?;
        if !f.state.in_folder() {
            keys.push(format!("release {}: {}", f.tool, f.state.words()));
        }
    }
    sections.push(section(
        "release sources",
        &releases::index_path(&ns)?,
        index.tools.len(),
        keys,
    ));

    let listed: ListedFile = match source(&ns, "listed") {
        Ok(p) if p.is_file() => toml::from_str(&read(&p)?)?,
        _ => ListedFile::default(),
    };
    for s in &mut sections {
        for d in &mut s.drift {
            d.listed = listed.listed.iter().find(|l| l.drift == d.key).cloned();
        }
    }
    Ok(sections)
}

fn dedup(mut keys: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    keys.retain(|k| seen.insert(k.clone()));
    keys
}

fn section(name: &'static str, source: &Path, checked: usize, keys: Vec<String>) -> Section {
    Section {
        name,
        source: source.display().to_string(),
        checked,
        drift: keys
            .into_iter()
            .map(|key| Drift { key, listed: None })
            .collect(),
    }
}
