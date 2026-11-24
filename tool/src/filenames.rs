//! Authored canon filenames: casing, Chronicle books and basename uniqueness, from governance data.
use crate::{Result, document::Finding, manifest::Namespace, read};
use regex::Regex;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
#[derive(Deserialize)]
struct Rules {
    casing: String,
    #[serde(default)]
    exception: Vec<Exception>,
    chronicles: Chronicles,
}
#[derive(Deserialize)]
struct Exception {
    name: String,
}
#[derive(Deserialize)]
struct Chronicles {
    folder: String,
    book: String,
}
fn finding(file: &Path, entry: &str, detail: String) -> Finding {
    Finding {
        file: file.into(),
        line: 1,
        entry: entry.into(),
        detail: Some(detail),
    }
}
fn roman(s: &str) -> Option<usize> {
    let value = |c| match c {
        'i' => Some(1),
        'v' => Some(5),
        'x' => Some(10),
        'l' => Some(50),
        'c' => Some(100),
        _ => None,
    };
    let digits: Vec<i64> = s.chars().map(value).collect::<Option<_>>()?;
    let mut total = 0;
    for (i, d) in digits.iter().enumerate() {
        if digits.get(i + 1).is_some_and(|n| n > d) {
            total -= d;
        } else {
            total += d;
        }
    }
    usize::try_from(total).ok().filter(|n| *n > 0)
}
fn walk(p: &Path, skip: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let m = crate::io(p, std::fs::symlink_metadata(p))?;
    if m.file_type().is_symlink() || p == skip {
        return Ok(());
    }
    if m.is_file() {
        out.push(p.into());
    } else if m.is_dir() {
        for e in crate::io(p, std::fs::read_dir(p))? {
            walk(&crate::io(p, e)?.path(), skip, out)?;
        }
    }
    Ok(())
}
/// Findings for the selected core files; the uniqueness universe is the whole constitution
/// and the governance documents outside skills. Without `registry/filenames.toml`, none.
pub fn findings(ns: &Namespace, selected: &[PathBuf]) -> Result<Vec<Finding>> {
    let (Ok(constitution), Ok(governance)) = (ns.role("constitution"), ns.role("governance"))
    else {
        return Ok(vec![]);
    };
    let source = governance.join("registry/filenames.toml");
    if !source.is_file() {
        return Ok(vec![]);
    }
    let rules: Rules = toml::from_str(&read(&source)?)?;
    let casing = Regex::new(&rules.casing)?;
    let book = Regex::new(&rules.chronicles.book)?;
    let exempt: BTreeSet<&str> = rules.exception.iter().map(|e| e.name.as_str()).collect();
    let name = |p: &Path| p.file_name().map(|s| s.to_string_lossy().into_owned());
    let mut canon = Vec::new();
    if constitution.is_dir() {
        walk(&constitution, Path::new(""), &mut canon)?;
    }
    let mut universe = canon.clone();
    if governance.is_dir() {
        walk(&governance, &governance.join("skills"), &mut universe)?;
    }
    let canon_names: BTreeSet<String> = canon.iter().filter_map(|p| name(p)).collect();
    let mut seen: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for p in &universe {
        if let Some(n) = name(p)
            && canon_names.contains(&n)
            && !exempt.contains(n.as_str())
            && !ns.is_record(p)
        {
            seen.entry(n).or_default().push(p.clone());
        }
    }
    let chronicles = constitution.join(&rules.chronicles.folder);
    let mut books: BTreeMap<usize, Vec<PathBuf>> = BTreeMap::new();
    for p in canon
        .iter()
        .filter(|p| p.parent() == Some(chronicles.as_path()))
    {
        if let Some(n) = name(p)
            && let Some(c) = book.captures(&n)
            && let Some(v) = roman(&c["n"])
        {
            books.entry(v).or_default().push(p.clone());
        }
    }
    let mut out = Vec::new();
    for p in selected {
        let Some(n) = name(p) else { continue };
        if p.starts_with(&constitution) && !exempt.contains(n.as_str()) {
            if !casing.is_match(&n) {
                out.push(finding(
                    p,
                    "filename:casing",
                    format!("{n} is not lowercase_underscore"),
                ));
            }
            if p.parent() == Some(chronicles.as_path()) && n.to_lowercase().starts_with("book") {
                match book.captures(&n).and_then(|c| roman(&c["n"])) {
                    None => out.push(finding(
                        p,
                        "filename:chronicle",
                        format!("{n} is not book_<roman>_<title>.md"),
                    )),
                    Some(v) if (1..v).any(|k| !books.contains_key(&k)) => out.push(finding(
                        p,
                        "filename:chronicle",
                        format!("book {v} has a gap before it"),
                    )),
                    Some(v) if books[&v].len() > 1 => out.push(finding(
                        p,
                        "filename:chronicle",
                        format!("book {v} appears {} times", books[&v].len()),
                    )),
                    _ => {}
                }
            }
        }
        if let Some(all) = seen.get(&n)
            && all.len() > 1
            && all.contains(p)
        {
            let others: Vec<_> = all
                .iter()
                .filter(|o| *o != p)
                .map(|o| o.strip_prefix(&ns.root).unwrap_or(o).display().to_string())
                .collect();
            out.push(finding(
                p,
                "filename:duplicate",
                format!("canon basename {n} also at {}", others.join(", ")),
            ));
        }
    }
    Ok(out)
}
