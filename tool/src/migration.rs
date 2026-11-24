//! Ordered, no-clobber migration operations. Journals are write-ahead and synced.
use crate::{Error, Result, io, manifest::Namespace, read};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::{
    ffi::CString,
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn invalid(s: impl Into<String>) -> Error {
    Error::Invalid(s.into())
}
fn exists(p: &Path) -> bool {
    p.symlink_metadata().is_ok()
}
fn absolute(p: &Path) -> Result<()> {
    if !p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(invalid(format!(
            "absolute path without '..' required: {}",
            p.display()
        )));
    }
    Ok(())
}
#[derive(Default)]
struct Resolver<'a> {
    ns: Option<&'a Namespace>,
}
impl Resolver<'_> {
    fn path(
        &self,
        path: &Path,
        field: &str,
        required: bool,
        destination: bool,
        resolved: &mut BTreeMap<String, PathBuf>,
    ) -> Result<PathBuf> {
        if exists(path) {
            return Ok(path.into());
        }
        if let Some(ns) = self.ns {
            for counterpart in crate::paths::counterparts(ns, path) {
                if exists(&counterpart)
                    || destination
                        && !path.parent().is_some_and(Path::is_dir)
                        && counterpart.parent().is_some_and(Path::is_dir)
                {
                    preview(&format!(
                        "resolve {field} {} -> {}",
                        path.display(),
                        counterpart.display()
                    ));
                    resolved.insert(field.into(), path.into());
                    return Ok(counterpart);
                }
            }
        }
        if required {
            return Err(invalid(format!(
                "{}: absent under original and successor names",
                path.display()
            )));
        }
        Ok(path.into())
    }
}
/// The kernel provides the destination-exists check atomically, including dangling links.
#[allow(unsafe_code)]
pub fn rename_noreplace(from: &Path, to: &Path) -> Result<()> {
    let a = CString::new(from.as_os_str().as_bytes()).map_err(|_| invalid("NUL in source"))?;
    let b = CString::new(to.as_os_str().as_bytes()).map_err(|_| invalid("NUL in destination"))?;
    // SAFETY: both C strings remain alive and NUL-terminated during the call;
    // AT_FDCWD accepts these paths and RENAME_NOREPLACE never replaces a target.
    let rc = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            a.as_ptr(),
            libc::AT_FDCWD,
            b.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc != 0 {
        return Err(Error::Io {
            path: to.into(),
            source: std::io::Error::last_os_error(),
        });
    }
    for p in [from, to] {
        if let Some(parent) = p.parent() {
            io(parent, File::open(parent).and_then(|f| f.sync_all()))?;
        }
    }
    Ok(())
}
fn fingerprint(p: &Path) -> Result<String> {
    let m = io(p, fs::symlink_metadata(p))?;
    if m.file_type().is_symlink() {
        return Ok(format!("link:{}", io(p, fs::read_link(p))?.display()));
    }
    if m.is_file() {
        return Ok(format!(
            "file:{}:{:o}",
            blake3::hash(&io(p, fs::read(p))?),
            m.mode()
        ));
    }
    if m.is_dir() {
        return Ok(format!("dir:{}:{}", m.dev(), m.ino()));
    }
    Err(invalid(format!("unsupported object: {}", p.display())))
}
#[derive(Debug, Serialize, Deserialize)]
struct Act {
    from: PathBuf,
    to: PathBuf,
    fingerprint: String,
    #[serde(default)]
    copy: bool,
    #[serde(default)]
    resolved_from: BTreeMap<String, PathBuf>,
}
struct Journal {
    path: PathBuf,
    file: File,
    resolved_from: BTreeMap<String, PathBuf>,
}
impl Journal {
    fn create(path: &Path) -> Result<Self> {
        let mut file = io(
            path,
            OpenOptions::new().write(true).create_new(true).open(path),
        )?;
        io(path, file.write_all(b"time\tresult\tact\n"))?;
        io(path, file.sync_all())?;
        Ok(Self {
            path: path.into(),
            file,
            resolved_from: BTreeMap::new(),
        })
    }
    fn worktree(&mut self, live: &Path, target: &Path, inverse: &Path) -> Result<()> {
        let act = serde_json::json!({"worktree":live,"target":target,"inverse":inverse,"resolved_from":self.resolved_from});
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| invalid(e.to_string()))?
            .as_secs()
            .to_string();
        let start = io(&self.path, self.file.seek(SeekFrom::End(0)))?;
        io(
            &self.path,
            self.file
                .write_all(format!("{time}\tpending\t{act}\n").as_bytes()),
        )?;
        io(&self.path, self.file.sync_all())?;
        repair(live, target)?;
        io(
            &self.path,
            self.file
                .seek(SeekFrom::Start(start + time.len() as u64 + 1)),
        )?;
        io(&self.path, self.file.write_all(b"applied"))?;
        io(&self.path, self.file.sync_all())?;
        Ok(())
    }
    fn rename(&mut self, from: &Path, to: &Path) -> Result<()> {
        self.act(from, to, false)
    }
    fn copy(&mut self, from: &Path, to: &Path) -> Result<()> {
        self.act(from, to, true)
    }
    fn act(&mut self, from: &Path, to: &Path, copy: bool) -> Result<()> {
        let act = Act {
            from: from.into(),
            to: to.into(),
            fingerprint: fingerprint(from)?,
            copy,
            resolved_from: self.resolved_from.clone(),
        };
        if exists(to) {
            return Err(invalid(format!("occupied destination: {}", to.display())));
        }
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| invalid(e.to_string()))?
            .as_secs()
            .to_string();
        let start = io(&self.path, self.file.seek(SeekFrom::End(0)))?;
        let row = format!("{time}\tpending\t{}\n", serde_json::to_string(&act)?);
        io(&self.path, self.file.write_all(row.as_bytes()))?;
        io(&self.path, self.file.sync_all())?;
        if copy {
            reflink(from, to)?;
        } else {
            rename_noreplace(from, to)?;
        }
        io(
            &self.path,
            self.file
                .seek(SeekFrom::Start(start + time.len() as u64 + 1)),
        )?;
        io(&self.path, self.file.write_all(b"applied"))?;
        io(&self.path, self.file.sync_all())?;
        Ok(())
    }
}
fn table(path: &Path, header: &str) -> Result<Vec<Vec<String>>> {
    let text = read(path)?;
    let mut lines = text.lines();
    if lines.next() != Some(header) {
        return Err(invalid(format!(
            "{}: expected TSV header {header}",
            path.display()
        )));
    }
    let width = header.split('\t').count();
    let mut rows = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (i, line) in lines.enumerate() {
        let cells: Vec<String> = line.split('\t').map(str::to_owned).collect();
        if cells.len() != width {
            return Err(invalid(format!(
                "{}:{}: expected {width} columns",
                path.display(),
                i + 2
            )));
        }
        let n = cells[0]
            .parse::<u64>()
            .map_err(|_| invalid(format!("{}:{}: invalid seq", path.display(), i + 2)))?;
        if !seen.insert(n) {
            return Err(invalid(format!("duplicate seq {n}")));
        }
        rows.push((n, cells));
    }
    rows.sort_by_key(|r| r.0);
    Ok(rows.into_iter().map(|r| r.1).collect())
}
fn preview(effect: &str) {
    println!("WRITE {effect}");
}
pub fn move_map(map: &Path, journal: &Path, dry: bool) -> Result<usize> {
    move_map_with(map, journal, dry, None)
}
pub fn move_map_with(
    map: &Path,
    journal: &Path,
    dry: bool,
    ns: Option<&Namespace>,
) -> Result<usize> {
    let resolver = Resolver { ns };
    let mut base = BTreeMap::new();
    let map = resolver.path(map, "map", true, false, &mut base)?;
    let journal = resolver.path(journal, "journal", false, true, &mut base)?;
    let journal = journal.as_path();
    let mut rows = table(&map, "seq\tfrom\tto\tkind\treason")?;
    let mut resolutions = Vec::new();
    for r in &mut rows {
        let mut resolved = base.clone();
        r[1] = resolver
            .path(Path::new(&r[1]), "from", false, false, &mut resolved)?
            .to_string_lossy()
            .into_owned();
        r[2] = resolver
            .path(Path::new(&r[2]), "to", false, true, &mut resolved)?
            .to_string_lossy()
            .into_owned();
        resolutions.push(resolved);
        absolute(Path::new(&r[1]))?;
        absolute(Path::new(&r[2]))?;
        if !matches!(
            r[3].as_str(),
            "heavy" | "record" | "regenerable" | "legacy_to_archive" | "pre_evolution_to_archive"
        ) {
            return Err(invalid(format!("seq {}: unknown move kind {}", r[0], r[3])));
        }
        preview(&format!(
            "seq {} rename no-replace {} -> {}; journal {}",
            r[0],
            r[1],
            r[2],
            journal.display()
        ));
    }
    if dry {
        let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
        for r in &rows {
            let from = PathBuf::from(&r[1]);
            let to = PathBuf::from(&r[2]);
            let virtual_exists = |path: &Path| {
                let mut original = path.to_path_buf();
                for (moved_from, moved_to) in moves.iter().rev() {
                    if let Ok(suffix) = original.strip_prefix(moved_to) {
                        original = if suffix.as_os_str().is_empty() {
                            moved_from.clone()
                        } else {
                            moved_from.join(suffix)
                        };
                    } else if original.starts_with(moved_from) {
                        return false;
                    }
                }
                exists(&original)
            };
            if !virtual_exists(&from) || virtual_exists(&to) {
                return Err(invalid(format!(
                    "seq {} refused: source missing or target occupied",
                    r[0]
                )));
            }
            moves.push((from, to));
        }
        return Ok(rows.len());
    }
    let mut j = Journal::create(journal)?;
    for (r, resolved) in rows.iter().zip(resolutions) {
        j.resolved_from = resolved;
        j.rename(Path::new(&r[1]), Path::new(&r[2]))
            .map_err(|e| invalid(format!("seq {} refused: {e}", r[0])))?;
    }
    Ok(rows.len())
}
pub(crate) fn reflink(from: &Path, to: &Path) -> Result<()> {
    if !io(from, fs::symlink_metadata(from))?.is_file() {
        return Err(invalid(format!(
            "regular file required: {}",
            from.display()
        )));
    }
    if exists(to) {
        return Err(invalid(format!(
            "copy destination occupied: {}",
            to.display()
        )));
    }
    // cp -n additionally protects against a destination appearing after the precheck.
    let status = io(
        to,
        Command::new("cp")
            .args([
                "--reflink=always",
                "--preserve=all",
                "--no-clobber",
                "--no-target-directory",
                "--",
            ])
            .arg(from)
            .arg(to)
            .status(),
    )?;
    if !status.success() || fingerprint(from)? != fingerprint(to)? {
        return Err(invalid(format!("reflink copy failed: {}", to.display())));
    }
    io(to, File::open(to).and_then(|f| f.sync_all()))?;
    Ok(())
}
fn repair(live: &Path, target: &Path) -> Result<()> {
    let status = io(
        live,
        Command::new("git")
            .arg("-C")
            .arg(live)
            .args(["worktree", "repair", "--"])
            .arg(target)
            .status(),
    )?;
    if !status.success() {
        return Err(invalid(format!(
            "worktree repair failed: {}",
            live.display()
        )));
    }
    Ok(())
}
pub fn switch_set(set: &Path, before: &Path, journal: &Path, dry: bool) -> Result<usize> {
    switch_set_with(set, before, journal, dry, None)
}
pub fn switch_set_with(
    set: &Path,
    before: &Path,
    journal: &Path,
    dry: bool,
    ns: Option<&Namespace>,
) -> Result<usize> {
    let resolver = Resolver { ns };
    let mut base = BTreeMap::new();
    let set = resolver.path(set, "set", true, false, &mut base)?;
    let before = resolver.path(before, "before", false, true, &mut base)?;
    let journal = resolver.path(journal, "journal", false, true, &mut base)?;
    let before = before.as_path();
    let journal = journal.as_path();
    let mut rows = table(&set, "seq\tkind\tlive\tstaged\tinverse\treason")?;
    // Validate every hold before WRITE previews, mkdir, journal creation or any consumer act.
    for r in &rows {
        if r[3].trim().is_empty() || r[5].starts_with("HOLD") {
            return Err(invalid(format!("seq {} refused: held row", r[0])));
        }
    }
    absolute(before)?;
    let mut resolutions = Vec::new();
    for r in &mut rows {
        let mut resolved = base.clone();
        r[2] = resolver
            .path(Path::new(&r[2]), "live", false, false, &mut resolved)?
            .to_string_lossy()
            .into_owned();
        r[3] = resolver
            .path(
                Path::new(&r[3]),
                "staged",
                !matches!(r[1].as_str(), "key" | "worktree"),
                r[1] == "key",
                &mut resolved,
            )
            .map_err(|e| invalid(format!("seq {} refused: {e}", r[0])))?
            .to_string_lossy()
            .into_owned();
        resolutions.push(resolved);
        absolute(Path::new(&r[2]))?;
        absolute(Path::new(&r[3]))?;
        if !matches!(
            r[1].as_str(),
            "file" | "unit" | "env" | "pack" | "trust" | "key" | "link" | "release" | "worktree"
        ) {
            return Err(invalid(format!(
                "seq {}: unsupported switch kind {}",
                r[0], r[1]
            )));
        }
        if r[1] == "worktree" {
            absolute(Path::new(&r[4]))?;
        }
        preview(&format!(
            "seq {} {} {} -> {}; preserve original and reflink copies in {}; journal {}",
            r[0],
            r[1],
            r[2],
            r[3],
            before.display(),
            journal.display()
        ));
    }
    if dry {
        return Ok(rows.len());
    }
    // Refuse reuse: all backup and prepared names belong exclusively to this run.
    io(before, fs::create_dir(before))?;
    let mut j = Journal::create(journal)?;
    for (r, resolved) in rows.iter().zip(resolutions) {
        j.resolved_from = resolved;
        let live = Path::new(&r[2]);
        let staged = Path::new(&r[3]);
        let saved = before.join(format!("{}.original", r[0]));
        let result = (|| -> Result<()> {
            match r[1].as_str() {
                "key" => j.rename(live, staged)?,
                "worktree" => j.worktree(live, staged, Path::new(&r[4]))?,
                "link" | "release" => {
                    if !staged.exists() {
                        return Err(invalid(format!(
                            "link target missing: {}",
                            staged.display()
                        )));
                    }
                    let prepared = before.join(format!("{}.prepared", r[0]));
                    io(&prepared, std::os::unix::fs::symlink(staged, &prepared))?;
                    if exists(live) {
                        j.rename(live, &saved)?;
                    }
                    j.rename(&prepared, live)?;
                }
                _ => {
                    let prepared = before.join(format!("{}.prepared", r[0]));
                    j.copy(staged, &prepared)?;
                    if exists(live) {
                        j.copy(live, &before.join(format!("{}.bytes", r[0])))?;
                        j.rename(live, &saved)?;
                    }
                    j.rename(&prepared, live)?;
                }
            }
            Ok(())
        })();
        result.map_err(|e| invalid(format!("seq {} refused: {e}", r[0])))?;
    }
    Ok(rows.len())
}
pub fn inverse(journal: &Path) -> Result<usize> {
    inverse_with(journal, None)
}
pub fn inverse_with(journal: &Path, ns: Option<&Namespace>) -> Result<usize> {
    let resolver = Resolver { ns };
    let mut base = BTreeMap::new();
    let journal = resolver.path(journal, "journal", true, false, &mut base)?;
    let journal = journal.as_path();
    let text = read(journal)?;
    let mut lines = text.lines();
    if lines.next() != Some("time\tresult\tact") {
        return Err(invalid("unrecognized journal"));
    }
    let entries: Vec<_> = lines.collect();
    let undo = journal.with_extension("inverse.tsv");
    // An interrupted inverse appends to its own durable journal and resumes by endpoints.
    for line in entries.iter().rev() {
        preview(&format!("inverse {line}; journal {}", undo.display()));
    }
    let mut out = if exists(&undo) {
        Journal {
            path: undo.clone(),
            file: io(&undo, OpenOptions::new().read(true).write(true).open(&undo))?,
            resolved_from: base.clone(),
        }
    } else {
        Journal::create(&undo)?
    };
    let mut completed = std::collections::BTreeSet::new();
    let mut repaired = std::collections::BTreeSet::new();
    let undo_text = read(&undo)?;
    let mut line_offset = "time\tresult\tact\n".len() as u64;
    for segment in undo_text.split_inclusive('\n').skip(1) {
        let row = segment.trim_end_matches('\n');
        let result_offset = line_offset + row.split('\t').next().unwrap_or("").len() as u64 + 1;
        line_offset += segment.len() as u64;
        let cells: Vec<_> = row.splitn(3, '\t').collect();
        if cells.len() != 3 {
            return Err(invalid("malformed inverse journal"));
        }
        let value: serde_json::Value = serde_json::from_str(cells[2])?;
        if let (Some(live), Some(target)) = (value["worktree"].as_str(), value["target"].as_str()) {
            if cells[1] == "applied" {
                repaired.insert((live.to_owned(), target.to_owned()));
            }
            continue;
        }
        let mut a: Act = serde_json::from_value(value)?;
        let mut resolved = base.clone();
        a.from = resolver.path(&a.from, "from", false, true, &mut resolved)?;
        a.to = resolver.path(&a.to, "to", false, true, &mut resolved)?;
        out.resolved_from = resolved;
        let inferred = cells[1] == "pending"
            && !exists(&a.from)
            && exists(&a.to)
            && fingerprint(&a.to)? == a.fingerprint;
        if inferred {
            io(&undo, out.file.seek(SeekFrom::Start(result_offset)))?;
            io(&undo, out.file.write_all(b"applied"))?;
            io(&undo, out.file.sync_all())?;
        }
        if cells[1] == "applied" || inferred {
            completed.insert((a.to, a.from));
        }
    }
    let mut count = 0;
    for line in entries.into_iter().rev() {
        let fields: Vec<_> = line.splitn(3, '\t').collect();
        if fields.len() != 3 || !matches!(fields[1], "pending" | "applied") {
            return Err(invalid("malformed journal row"));
        }
        let value: serde_json::Value = serde_json::from_str(fields[2])?;
        if let Some(live) = value["worktree"].as_str() {
            let target = value["inverse"]
                .as_str()
                .ok_or_else(|| invalid("worktree inverse missing"))?;
            if repaired.contains(&(live.to_owned(), target.to_owned())) {
                continue;
            }
            let original = value["target"]
                .as_str()
                .ok_or_else(|| invalid("worktree target missing"))?;
            let mut resolved = base.clone();
            let live = resolver.path(Path::new(live), "worktree", true, false, &mut resolved)?;
            let target = resolver.path(Path::new(target), "inverse", true, false, &mut resolved)?;
            let original =
                resolver.path(Path::new(original), "target", false, true, &mut resolved)?;
            out.resolved_from = resolved;
            out.worktree(&live, &target, &original)?;
            count += 1;
            continue;
        }
        let mut a: Act = serde_json::from_value(value)?;
        let mut resolved = base.clone();
        a.from = resolver.path(&a.from, "from", false, true, &mut resolved)?;
        a.to = resolver.path(&a.to, "to", false, true, &mut resolved)?;
        out.resolved_from = resolved;
        absolute(&a.from)?;
        absolute(&a.to)?;
        if a.copy {
            continue;
        } // Backups and preparations are permanent evidence, never deleted.
        if completed.contains(&(a.from.clone(), a.to.clone())) {
            continue;
        }
        // A pending row may be either side of the syscall after an interruption.
        if exists(&a.from) && !exists(&a.to) && fingerprint(&a.from)? == a.fingerprint {
            continue;
        }
        if exists(&a.from) || !exists(&a.to) || fingerprint(&a.to)? != a.fingerprint {
            return Err(invalid(format!(
                "inverse refused: modified or occupied {} -> {}",
                a.to.display(),
                a.from.display()
            )));
        }
        out.rename(&a.to, &a.from)?;
        count += 1;
    }
    Ok(count)
}
