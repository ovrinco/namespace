//! Counterpart lookup uses the manifest's root names, never substring replacement.
use crate::{Error, Result, manifest::Namespace, read};
use globset::GlobBuilder;
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize)]
pub struct Resolution {
    pub resolved_from: PathBuf,
    pub path: PathBuf,
}
pub fn home(ns: &Namespace) -> PathBuf {
    if ns
        .value
        .pointer("/consumers/staging_source/base")
        .and_then(serde_json::Value::as_str)
        == Some("home")
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| ns.root.clone())
    } else {
        ns.root.clone()
    }
}
pub fn counterparts(ns: &Namespace, path: &Path) -> Vec<PathBuf> {
    let mut anchors = vec![ns.root.clone(), home(ns)];
    anchors.dedup();
    let mut candidates = Vec::new();
    for anchor in anchors {
        for (old, new) in ns.value["retired_roots"].as_object().into_iter().flatten() {
            let Some(new) = new.as_str() else {
                continue;
            };
            for (a, b) in [(old.as_str(), new), (new, old.as_str())] {
                if let Ok(rest) = path.strip_prefix(anchor.join(a)) {
                    candidates.push(anchor.join(b).join(rest));
                }
            }
        }
    }
    candidates
}
pub fn existing(ns: &Namespace, path: &Path) -> PathBuf {
    if path.symlink_metadata().is_ok() {
        return path.into();
    }
    counterparts(ns, path)
        .into_iter()
        .find(|p| p.symlink_metadata().is_ok())
        .unwrap_or_else(|| path.into())
}
/// Journals read when the manifest's `move_ledger` lists none of its own: the 19 Oct move's
/// three journals and the 28 Oct home cleanup's, relative to the ledger's role.
pub const DEFAULT_JOURNALS: &[&str] = &[
    "runs/namespace/2026-10-19_namespace_and_memory/design/MOVE_JOURNAL*.tsv",
    "runs/namespace/2026-10-28_clean_home/mapping/*.journal.tsv",
];
/// Longest chain of moves followed for one path.
const MAX_STEPS: usize = 64;
#[derive(Debug, Clone)]
struct Move {
    from: PathBuf,
    to: PathBuf,
    file: PathBuf,
    line: usize,
}
/// The applied acts of every move journal a reader consults, opened read only.
#[derive(Debug, Default)]
pub struct Journals {
    pub files: Vec<PathBuf>,
    moves: Vec<Move>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Found {
    pub input: PathBuf,
    pub path: Option<PathBuf>,
    pub how: Vec<String>,
}
fn expand(pattern: &Path) -> Result<Vec<PathBuf>> {
    let (Some(dir), Some(name)) = (pattern.parent(), pattern.file_name()) else {
        return Ok(vec![]);
    };
    let glob = GlobBuilder::new(&name.to_string_lossy())
        .literal_separator(true)
        .build()?
        .compile_matcher();
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(vec![]);
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| glob.is_match(n) && !n.to_string_lossy().ends_with(".inverse.tsv"))
                && p.is_file()
        })
        .collect();
    out.sort();
    Ok(out)
}
impl Journals {
    /// The manifest's `move_ledger` journal, its `journals` globs (or [`DEFAULT_JOURNALS`]),
    /// then `extra`. Named ledger files that are absent are skipped; an absent `extra` is an error.
    pub fn load(ns: &Namespace, extra: &[PathBuf]) -> Result<Self> {
        let ledger = &ns.value["move_ledger"];
        let base = match ledger["role"].as_str() {
            Some(role) => ns.role(role)?,
            None => ns.root.clone(),
        };
        let mut files = Vec::new();
        if let Some(p) = ledger["path"].as_str() {
            files.push(base.join(p));
        }
        let globs: Vec<String> = match ledger["journals"].as_array() {
            Some(list) => list
                .iter()
                .map(|v| {
                    v.as_str().map(String::from).ok_or_else(|| {
                        Error::Invalid("move_ledger.journals entry is not a string".into())
                    })
                })
                .collect::<Result<_>>()?,
            None if ledger.is_object() => DEFAULT_JOURNALS.iter().map(|s| s.to_string()).collect(),
            None => vec![],
        };
        for g in globs {
            files.extend(expand(&base.join(g))?);
        }
        files.retain(|f| f.is_file());
        for f in extra {
            if !f.is_file() {
                return Err(Error::Invalid(format!(
                    "journal {} is not a file",
                    f.display()
                )));
            }
            files.push(f.clone());
        }
        let mut seen = std::collections::HashSet::new();
        files.retain(|f| seen.insert(f.clone()));
        let mut moves = Vec::new();
        for f in &files {
            for (i, line) in read(f)?.lines().enumerate() {
                let mut cols = line.splitn(3, '\t');
                let (Some(_), Some(result), Some(act)) = (cols.next(), cols.next(), cols.next())
                else {
                    continue;
                };
                if !matches!(result.trim(), "applied" | "pending") {
                    continue;
                }
                let Ok(act) = serde_json::from_str::<serde_json::Value>(act) else {
                    continue;
                };
                if let (Some(from), Some(to)) = (act["from"].as_str(), act["to"].as_str()) {
                    moves.push(Move {
                        from: from.into(),
                        to: to.into(),
                        file: f.clone(),
                        line: i + 1,
                    });
                }
            }
        }
        // Most specific move first: a file moved on its own outranks its folder's later move.
        moves.sort_by(|a, b| {
            b.from
                .components()
                .count()
                .cmp(&a.from.components().count())
        });
        Ok(Self { files, moves })
    }
}
/// Where `path` is now: unchanged when it exists, else through the retired roots and the move
/// journals, following a chain of moves to its end. Read only; nothing is written or linked.
pub fn resolve(ns: &Namespace, journals: &Journals, path: &Path) -> Found {
    let found = |p: PathBuf, how| Found {
        input: path.into(),
        path: Some(p),
        how,
    };
    if path.symlink_metadata().is_ok() {
        return found(path.into(), vec!["exists".into()]);
    }
    let start = std::path::absolute(path).unwrap_or_else(|_| path.into());
    let mut queue = std::collections::VecDeque::from([(start.clone(), Vec::<String>::new())]);
    let mut seen = std::collections::HashSet::from([start]);
    let mut steps = 0;
    while let Some((p, how)) = queue.pop_front() {
        if !how.is_empty() && p.symlink_metadata().is_ok() {
            return found(p, how);
        }
        steps += 1;
        if steps > MAX_STEPS {
            break;
        }
        let roots = counterparts(ns, &p)
            .into_iter()
            .map(|c| (c, "root".to_string()));
        let moved = journals.moves.iter().filter_map(|m| {
            p.strip_prefix(&m.from).ok().map(|rest| {
                let to = if rest.as_os_str().is_empty() {
                    m.to.clone()
                } else {
                    m.to.join(rest)
                };
                (to, format!("journal {}:{}", m.file.display(), m.line))
            })
        });
        for (next, via) in roots.chain(moved) {
            if seen.insert(next.clone()) {
                let mut how = how.clone();
                how.push(via);
                queue.push_back((next, how));
            }
        }
    }
    Found {
        input: path.into(),
        path: None,
        how: vec![],
    }
}
