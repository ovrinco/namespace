use crate::{
    Error, Result,
    document::{Catalogue, bounded, is_document, is_source_record, mask, whole, whole_name},
    io,
    manifest::Namespace,
    read,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub entry: String,
    pub replacement: String,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Change {
    pub file: PathBuf,
    pub before: String,
    pub after: String,
    pub edits: Vec<Edit>,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Skip {
    pub file: PathBuf,
    pub reason: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Plan {
    pub changes: Vec<Change>,
    pub skipped: Vec<Skip>,
}
pub fn hash(b: &[u8]) -> String {
    blake3::hash(b).to_hex().to_string()
}
fn rewrite(text: &str, edits: &[Edit]) -> String {
    let mut out = text.to_owned();
    for e in edits.iter().rev() {
        out.replace_range(e.start..e.end, &e.replacement);
    }
    out
}
pub fn plan(ns: &Namespace, paths: &[PathBuf], names: bool) -> Result<Plan> {
    if !ns
        .value
        .pointer("/records/entries")
        .is_some_and(serde_json::Value::is_array)
    {
        return Err(Error::Invalid(
            "fix requires an explicit records.entries policy".into(),
        ));
    }
    let mut plan = Plan {
        changes: vec![],
        skipped: vec![],
    };
    let catalogue = if names {
        Some(Catalogue::load(ns)?)
    } else {
        None
    };
    for p in crate::files(paths)? {
        let components: Vec<_> = p.components().map(|c| c.as_os_str()).collect();
        if names
            && components
                .windows(2)
                .any(|c| c[0] == "00_constitution" && c[1] == "canon_chronicles")
        {
            plan.skipped.push(Skip {
                file: p,
                reason: "Chronicles preserve historical name spellings".into(),
            });
            continue;
        }
        let reason = if p.file_name().is_some_and(|s| {
            let s = s.to_string_lossy();
            s.contains(".ovrin-ns.") && s.ends_with(".before")
        }) {
            Some("preserved fix backup")
        } else if ns.is_record(&p) {
            Some("record: refused")
        } else if names && !is_document(&p) {
            Some("not a prose document")
        } else if !names
            && matches!(
                p.extension()
                    .and_then(|s| s.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("db" | "sqlite" | "sqlite3" | "json")
            )
        {
            Some("data store")
        } else {
            None
        };
        if let Some(reason) = reason {
            plan.skipped.push(Skip {
                file: p,
                reason: reason.into(),
            });
            continue;
        }
        let bytes = io(&p, fs::read(&p))?;
        let text = match std::str::from_utf8(&bytes) {
            Ok(t) if !bytes.contains(&0) => t,
            _ => {
                plan.skipped.push(Skip {
                    file: p,
                    reason: "binary or non-UTF-8".into(),
                });
                continue;
            }
        };
        if is_source_record(text) {
            plan.skipped.push(Skip {
                file: p,
                reason: "source-linked record: refused".into(),
            });
            continue;
        }
        let prose = if names { mask(text)? } else { text.into() };
        let mut edits = vec![];
        if let Some(cat) = &catalogue {
            for rule in &cat.rules {
                if let Some(rep) = &rule.replacement {
                    for m in rule.re.find_iter(&prose) {
                        if whole_name(&prose, m.start(), m.end())
                            && !edits
                                .iter()
                                .any(|e: &Edit| e.start < m.end() && m.start() < e.end)
                        {
                            edits.push(Edit {
                                start: m.start(),
                                end: m.end(),
                                entry: rule.id.clone(),
                                replacement: rep.clone(),
                            });
                        }
                    }
                }
            }
        } else if let Some(roots) = ns.value["retired_roots"].as_object() {
            for (old, new) in roots {
                if let Some(new) = new.as_str() {
                    for m in bounded(old, false)?.find_iter(&prose) {
                        if whole(&prose, m.start(), m.end()) {
                            edits.push(Edit {
                                start: m.start(),
                                end: m.end(),
                                entry: format!("root:{old}"),
                                replacement: new.into(),
                            });
                        }
                    }
                }
            }
        }
        edits.sort_by_key(|e| e.start);
        if !edits.is_empty() {
            let after = hash(rewrite(text, &edits).as_bytes());
            plan.changes.push(Change {
                file: p,
                before: hash(&bytes),
                after,
                edits,
            });
        }
    }
    Ok(plan)
}
/// Accept a JSON plan (possibly with unselected changes removed). Recompute and
/// compare every accepted change before any write; arbitrary edits are refused.
pub fn apply(
    ns: &Namespace,
    current: &Plan,
    accepted: &Path,
    ledger: &Path,
    before: &Path,
) -> Result<usize> {
    let requested = if before.is_absolute() {
        before.to_path_buf()
    } else {
        crate::io(before, std::env::current_dir())?.join(before)
    };
    if requested
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Error::Invalid("before folder must not contain '..'".into()));
    }
    let ancestor = requested
        .ancestors()
        .find(|p| p.exists())
        .ok_or_else(|| Error::Invalid("before folder has no existing ancestor".into()))?;
    let backup_root = crate::absolute(ancestor)?.join(
        requested
            .strip_prefix(ancestor)
            .map_err(|_| Error::Invalid("invalid before folder".into()))?,
    );
    // A declared record hold is outside living text after home becomes the root.
    let record_hold = ns.is_record(&backup_root)
        && crate::index::Policy::load(ns)?.is_some_and(|policy| !policy.admits(&backup_root, true));
    if backup_root.starts_with(&ns.root) && !record_hold {
        return Err(Error::Invalid(
            "before folder must be outside the namespace".into(),
        ));
    }

    let selected: Plan = serde_json::from_str(&read(accepted)?)?;
    let ledger_abs = if ledger.exists() {
        crate::absolute(ledger)?
    } else {
        crate::absolute(ledger.parent().unwrap_or(Path::new(".")))?.join(
            ledger
                .file_name()
                .ok_or_else(|| Error::Invalid("ledger needs a filename".into()))?,
        )
    };
    if ns.is_record(&ledger_abs) && !(record_hold && ledger_abs.parent() == backup_root.parent()) {
        return Err(Error::Invalid("ledger destination is a record".into()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for change in &selected.changes {
        if !seen.insert(&change.file)
            || !current.changes.contains(change)
            || ns.is_record(&change.file)
            || change.file == ledger_abs
            || !change.file.starts_with(&ns.root)
        {
            return Err(Error::Invalid(format!(
                "unaccepted, stale or protected plan entry: {}",
                change.file.display()
            )));
        }
        if io(&change.file, fs::symlink_metadata(&change.file))?
            .file_type()
            .is_symlink()
        {
            return Err(Error::Invalid(
                "accepted file became a symbolic link".into(),
            ));
        }
        if hash(&io(&change.file, fs::read(&change.file))?) != change.before {
            return Err(Error::Invalid(format!(
                "stale file {}",
                change.file.display()
            )));
        }
    }
    if selected.changes.is_empty() {
        return Ok(0);
    }
    for change in &selected.changes {
        println!(
            "WRITE fix {} preserving prior bytes under {}; journal {}",
            change.file.display(),
            backup_root.display(),
            ledger.display()
        );
    }
    io(&backup_root, fs::create_dir_all(&backup_root))?;
    let mut log = io(
        ledger,
        OpenOptions::new().create(true).append(true).open(ledger),
    )?;
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Invalid("clock before epoch".into()))?
        .as_secs();
    for change in &selected.changes {
        let text = read(&change.file)?;
        if hash(text.as_bytes()) != change.before {
            return Err(Error::Invalid("file changed during apply".into()));
        }
        let after = rewrite(&text, &change.edits);
        let relative = change
            .file
            .strip_prefix(&ns.root)
            .map_err(|_| Error::Invalid("file outside namespace".into()))?;
        let backup = backup_root.join(relative).with_file_name(format!(
            "{}.ovrin-ns.{}.before",
            change
                .file
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            change.before
        ));
        io(
            &backup,
            fs::create_dir_all(
                backup
                    .parent()
                    .ok_or_else(|| Error::Invalid("backup needs a parent".into()))?,
            ),
        )?;
        if !backup.exists() {
            crate::migration::reflink(&change.file, &backup)?;
        } else if hash(&io(&backup, fs::read(&backup))?) != change.before {
            return Err(Error::Invalid("backup hash mismatch".into()));
        }
        let tmp = change
            .file
            .with_file_name(format!(".ovrin-ns-{}-{time}.tmp", std::process::id()));
        let mut f = io(
            &tmp,
            OpenOptions::new().write(true).create_new(true).open(&tmp),
        )?;
        io(&tmp, f.write_all(after.as_bytes()))?;
        io(
            &tmp,
            fs::set_permissions(
                &tmp,
                io(&change.file, fs::metadata(&change.file))?.permissions(),
            ),
        )?;
        io(&tmp, f.sync_all())?;
        io(&change.file, fs::rename(&tmp, &change.file))?;
        let filename = change
            .file
            .to_string_lossy()
            .replace(['\t', '\n', '\r'], " ");
        io(
            ledger,
            writeln!(
                log,
                "{time}\t{filename}\t{}\t{}\t{}\t{}",
                change.before,
                change.after,
                change.edits.len(),
                backup.display()
            ),
        )?;
        io(ledger, log.sync_all())?;
    }
    Ok(selected.changes.len())
}
