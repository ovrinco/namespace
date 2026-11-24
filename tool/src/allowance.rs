//! Counted, source-visible allowances. Invalid entries remain findings.
use crate::{
    Result,
    document::{CheckReport, Finding},
    io,
    manifest::Namespace,
    read,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};
#[derive(Debug, Serialize)]
pub struct Allowed {
    pub file: PathBuf,
    pub entry: String,
    pub count: usize,
    pub reason: String,
    pub source: PathBuf,
}
#[derive(Default, Deserialize)]
struct Registry {
    #[serde(default)]
    allow: Vec<Entry>,
}
#[derive(Default, Deserialize)]
struct Entry {
    #[serde(default)]
    file: String,
    #[serde(default)]
    entry: String,
    #[serde(default)]
    count: Option<usize>,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    classes: Option<Vec<String>>,
}
fn finding(report: &mut CheckReport, file: &Path, entry: &str, detail: String) {
    report.findings.push(Finding {
        file: file.into(),
        line: 1,
        entry: entry.into(),
        detail: Some(detail),
    });
}
pub fn apply(
    ns: &Namespace,
    checked: &BTreeSet<PathBuf>,
    report: &mut CheckReport,
    class: &str,
) -> Result<()> {
    let directory = ns.role("governance")?.join("registry/allow");
    if !directory.exists() {
        return Ok(());
    }
    let mut sources = Vec::new();
    for entry in io(&directory, fs::read_dir(&directory))? {
        let p = io(&directory, entry)?.path();
        if p.extension().is_some_and(|s| s == "toml") {
            sources.push(p);
        }
    }
    sources.sort();
    let mut used: BTreeMap<(PathBuf, String), usize> = BTreeMap::new();
    let raw = report.findings.clone();
    for source in sources {
        if !io(&source, fs::symlink_metadata(&source))?.is_file() {
            finding(
                report,
                &source,
                "allowance:invalid",
                "allowance must be a regular file".into(),
            );
            continue;
        }
        let registry: Registry = match toml::from_str(&read(&source)?) {
            Ok(v) => v,
            Err(e) => {
                finding(report, &source, "allowance:invalid", e.to_string());
                continue;
            }
        };
        if registry.allow.is_empty() {
            finding(
                report,
                &source,
                "allowance:invalid",
                "expected at least one [[allow]] entry".into(),
            );
            continue;
        }
        for entry in registry.allow {
            let rel = Path::new(&entry.file);
            let file = ns.root.join(rel);
            let mut errors = Vec::new();
            if entry.classes.as_ref().is_some_and(|classes| {
                classes.is_empty()
                    || classes
                        .iter()
                        .any(|c| !matches!(c.as_str(), "core" | "spine"))
            }) {
                errors.push("classes must be a nonempty list of core/spine");
            }
            if entry.reason.trim().is_empty() {
                errors.push("missing reason");
            }
            if entry.entry.trim().is_empty() {
                errors.push("missing catalogue entry id");
            }
            if entry.count.is_none() {
                errors.push("missing count");
            }
            if rel.as_os_str().is_empty()
                || rel.is_absolute()
                || rel.components().any(|c| !matches!(c, Component::Normal(_)))
            {
                errors.push("file must be namespace-relative without traversal");
            }
            if rel.starts_with("00_constitution")
                || ns
                    .role("constitution")
                    .is_ok_and(|root| file.starts_with(root))
            {
                errors.push("constitution allowances are forbidden; lineage belongs in Chronicles");
            }
            let canonical = if file.is_file() {
                Some(io(&file, fs::canonicalize(&file))?)
            } else {
                None
            };
            if canonical.is_none() {
                errors.push("missing file");
            }
            if let Some(canonical) = &canonical {
                if !canonical.starts_with(&ns.root) {
                    errors.push("target resolves outside namespace");
                }
                if ns
                    .role("constitution")
                    .is_ok_and(|root| canonical.starts_with(root))
                {
                    errors.push("target resolves inside constitution");
                }
            }
            if !errors.is_empty() {
                finding(
                    report,
                    &source,
                    "allowance:invalid",
                    format!("{} {}: {}", entry.file, entry.entry, errors.join("; ")),
                );
                continue;
            }
            if entry
                .classes
                .as_ref()
                .is_some_and(|classes| !classes.iter().any(|c| c == class))
            {
                continue;
            }
            let file = canonical.expect("checked nonempty above");
            if !checked.contains(&file) {
                continue;
            }
            let key = (file.clone(), entry.entry.clone());
            let actual = raw
                .iter()
                .filter(|f| f.file == file && f.entry == entry.entry)
                .count();
            let consumed = *used.get(&key).unwrap_or(&0);
            let requested = entry.count.expect("validated count above");
            if requested > actual.saturating_sub(consumed) {
                finding(
                    report,
                    &source,
                    "allowance:stale",
                    format!(
                        "{} {}: count {} exceeds {} remaining occurrences ({} actual)",
                        entry.file,
                        entry.entry,
                        requested,
                        actual.saturating_sub(consumed),
                        actual
                    ),
                );
                continue;
            }
            let mut remaining = requested;
            report.findings.retain(|f| {
                if remaining > 0 && f.file == file && f.entry == entry.entry {
                    remaining -= 1;
                    false
                } else {
                    true
                }
            });
            used.insert(key, consumed + requested);
            report.allowed.push(Allowed {
                file,
                entry: entry.entry,
                count: requested,
                reason: entry.reason,
                source: source.clone(),
            });
        }
    }
    Ok(())
}
