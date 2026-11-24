//! Where the code behind each installed release is: the release index the manifest points at, read against each
//! release's own record and the project folder on disk.
use crate::{Error, Result, io, manifest::Namespace, read};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Deserialize)]
pub struct Index {
    #[serde(rename = "tool", default)]
    pub tools: Vec<Tool>,
}

/// One installed tool, as the release index names it. Paths are relative to the namespace root.
#[derive(Deserialize, Clone)]
pub struct Tool {
    pub name: String,
    /// The installed release folder, or the link that points at it.
    pub release: String,
    /// The project folder that should hold the code; absent when the registry gives the tool none.
    pub folder: Option<String>,
    /// Who keeps the release and its folder.
    pub owner: String,
    /// The release's own record, inside the release folder.
    pub record: Option<String>,
    /// How the record names its source: git_commit, file_sha256, tsv_manifests, tree_sha256 or none.
    pub identity: String,
    /// Dot path to the identity inside the record.
    pub field: Option<String>,
    /// A git repository other than the folder (relative to the root).
    pub repo: Option<String>,
    /// The subfolder of the folder the record's paths are relative to.
    pub base: Option<String>,
    /// Where the code was found when it is not in the folder; checked again on every run.
    pub elsewhere: Option<String>,
    pub note: Option<String>,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// The folder's working files are the release's source.
    CheckedOut,
    /// The folder's history holds the release's commit.
    InHistory,
    /// Not in the folder; found at the recorded other place.
    Elsewhere,
    /// Not in the folder and not at any recorded place.
    NotFound,
    /// The release records no source identity.
    Unrecorded,
}
impl State {
    /// Whether the code is reachable from the project folder.
    pub fn in_folder(self) -> bool {
        matches!(self, Self::CheckedOut | Self::InHistory)
    }
    pub fn words(self) -> &'static str {
        match self {
            Self::CheckedOut => "in this folder (checked out)",
            Self::InHistory => "in this folder's history",
            Self::Elsewhere => "NOT in this folder; found elsewhere",
            Self::NotFound => "NOT found",
            Self::Unrecorded => "the release records no source",
        }
    }
}

#[derive(Serialize)]
pub struct Found {
    pub tool: String,
    pub release: String,
    pub folder: Option<PathBuf>,
    pub owner: String,
    pub identity: String,
    pub state: State,
    pub detail: String,
    pub see: Option<String>,
    pub note: Option<String>,
}

pub fn index_path(ns: &Namespace) -> Result<PathBuf> {
    ns.value
        .pointer("/release_index/path")
        .and_then(Value::as_str)
        .map(|s| ns.root.join(s))
        .ok_or_else(|| Error::Invalid("the manifest has no release_index.path".into()))
}

pub fn load(ns: &Namespace) -> Result<Index> {
    Ok(toml::from_str(&read(&index_path(ns)?)?)?)
}

/// The tools whose project folder holds `path`, or every tool when `path` is none.
pub fn select<'a>(ns: &Namespace, index: &'a Index, path: Option<&Path>) -> Vec<&'a Tool> {
    index
        .tools
        .iter()
        .filter(|t| match (path, &t.folder) {
            (None, _) => true,
            (Some(p), Some(f)) => {
                let f = ns.root.join(f);
                p.starts_with(&f) || f.starts_with(p)
            }
            (Some(_), None) => false,
        })
        .collect()
}

pub fn locate(ns: &Namespace, t: &Tool) -> Result<Found> {
    let release_path = ns.root.join(&t.release);
    let release = fs::canonicalize(&release_path)
        .unwrap_or_else(|_| release_path.clone())
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = t.folder.as_ref().map(|f| ns.root.join(f));
    let mut found = Found {
        tool: t.name.clone(),
        release,
        folder: folder.clone(),
        owner: t.owner.clone(),
        identity: String::new(),
        state: State::Unrecorded,
        detail: String::new(),
        see: None,
        note: t.note.clone(),
    };
    if !release_path.exists() {
        found.state = State::NotFound;
        found.detail = format!("release {} missing", release_path.display());
        return Ok(found);
    }
    let record = t.record.as_ref().map(|r| release_path.join(r));
    let elsewhere = t.elsewhere.as_ref().map(|e| ns.root.join(e));
    match t.identity.as_str() {
        "git_commit" => {
            let commit = field_str(record.as_deref(), t.field.as_deref())?;
            found.identity = format!("commit {commit}");
            let repo = t
                .repo
                .as_ref()
                .map(|r| ns.root.join(r))
                .or_else(|| folder.clone());
            match repo.as_deref().and_then(|r| git_find(r, &commit)) {
                Some(g) => {
                    found.state = if g.checked_out {
                        State::CheckedOut
                    } else {
                        State::InHistory
                    };
                    found.detail = g.detail;
                    found.see = Some(format!(
                        "git -C {} show --stat {}",
                        repo.as_deref().unwrap_or(Path::new(".")).display(),
                        g.full
                    ));
                }
                None => match elsewhere.as_deref().and_then(|e| git_find(e, &commit)) {
                    Some(g) => {
                        found.state = State::Elsewhere;
                        found.detail = format!("{}: {}", elsewhere_str(&elsewhere), g.detail);
                        found.see = Some(format!(
                            "git -C {} show --stat {}",
                            elsewhere_str(&elsewhere),
                            g.full
                        ));
                    }
                    None => {
                        found.state = State::NotFound;
                        found.detail = "commit in no repository checked".into();
                    }
                },
            }
        }
        "file_sha256" => {
            let map = field_value(record.as_deref(), t.field.as_deref())?;
            let map = map
                .as_object()
                .ok_or_else(|| Error::Invalid(format!("{}: field is not a path map", t.name)))?;
            let files: Vec<(String, String)> = map
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_lowercase())))
                .collect();
            found.identity = format!("{} file hashes", files.len());
            by_files(
                &mut found,
                folder.as_deref(),
                t,
                elsewhere.as_deref(),
                |base| compare(base, &files),
            );
        }
        "tsv_manifests" => {
            let mut sets = Vec::new();
            let mut names: Vec<_> = io(&release_path, fs::read_dir(&release_path))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| s.starts_with("SOURCE_MANIFEST.") && s.ends_with(".tsv"))
                })
                .collect();
            names.sort();
            for p in &names {
                sets.push(tsv_manifest(p)?);
            }
            let total: usize = sets.iter().map(|(_, f)| f.len()).sum();
            found.identity = format!("{} file hashes in {} manifests", total, sets.len());
            let (mut equal, mut differ) = (0, Vec::new());
            for (root, files) in &sets {
                let (e, d) = compare(root, files);
                equal += e;
                differ.extend(d);
            }
            set_by_count(&mut found, equal, &differ, total);
            if found.state != State::CheckedOut {
                found.state = State::NotFound;
            }
        }
        "tree_sha256" => {
            let want = field_str(record.as_deref(), t.field.as_deref())?;
            found.identity = format!("source tree {}", short(&want));
            let check = |base: &Path| tree_sha256(base).map(|h| h == want).unwrap_or(false);
            let here = folder
                .as_deref()
                .map(|f| f.join(t.base.as_deref().unwrap_or("")));
            if here.as_deref().is_some_and(check) {
                found.state = State::CheckedOut;
                found.detail = "the folder's source tree hash equals the release's".into();
            } else if elsewhere.as_deref().is_some_and(check) {
                found.state = State::Elsewhere;
                found.detail = elsewhere_str(&elsewhere);
            } else {
                found.state = State::NotFound;
                found.detail = "no folder checked has the release's source tree hash".into();
            }
        }
        "none" => {
            found.detail = t
                .note
                .clone()
                .unwrap_or_else(|| "no source identity is recorded".into());
            found.note = None;
        }
        other => {
            return Err(Error::Invalid(format!(
                "{}: unknown identity {other}",
                t.name
            )));
        }
    }
    Ok(found)
}

fn elsewhere_str(e: &Option<PathBuf>) -> String {
    e.as_deref()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

fn short(s: &str) -> &str {
    &s[..s.len().min(12)]
}

fn by_files(
    found: &mut Found,
    folder: Option<&Path>,
    t: &Tool,
    elsewhere: Option<&Path>,
    cmp: impl Fn(&Path) -> (usize, Vec<String>),
) {
    let total: usize = found
        .identity
        .split(' ')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let here = folder.map(|f| f.join(t.base.as_deref().unwrap_or("")));
    let (e, d) = here.as_deref().map(&cmp).unwrap_or((0, vec![]));
    set_by_count(found, e, &d, total);
    if found.state == State::CheckedOut {
        return;
    }
    let first = |d: &[String]| d.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
    let in_folder = if here.is_some() {
        format!(
            "in the folder {e} of {total} equal (first differing: {})",
            first(&d)
        )
    } else {
        "no project folder to look in".to_string()
    };
    found.state = State::NotFound;
    found.detail = in_folder.clone();
    if let Some(other) = elsewhere {
        let (e2, d2) = cmp(other);
        if d2.is_empty() && e2 == total {
            found.state = State::Elsewhere;
            found.detail = format!("{in_folder}; all {total} equal at {}", other.display());
        } else {
            found.detail = format!(
                "{in_folder}; at {} {e2} of {total} equal (first differing: {})",
                other.display(),
                first(&d2)
            );
        }
    }
}

fn set_by_count(found: &mut Found, equal: usize, differ: &[String], total: usize) {
    if differ.is_empty() && equal == total {
        found.state = State::CheckedOut;
        found.detail = format!("{equal} of {total} files equal");
    } else {
        found.state = State::NotFound;
        let first: Vec<_> = differ.iter().take(5).cloned().collect();
        found.detail = format!(
            "{equal} of {total} files equal; first differing: {}",
            first.join(", ")
        );
    }
}

fn field_value(record: Option<&Path>, field: Option<&str>) -> Result<Value> {
    let record = record.ok_or_else(|| Error::Invalid("identity needs a record".into()))?;
    let v: Value = serde_json::from_str(&read(record)?)?;
    let mut cur = &v;
    for part in field.unwrap_or("").split('.').filter(|s| !s.is_empty()) {
        cur = cur
            .get(part)
            .ok_or_else(|| Error::Invalid(format!("{}: no field {part}", record.display())))?;
    }
    Ok(cur.clone())
}

fn field_str(record: Option<&Path>, field: Option<&str>) -> Result<String> {
    let record_path = record;
    if let Some(r) = record_path
        && r.extension().is_none_or(|e| e != "json")
    {
        // A plain-text record holds the identity alone (as SOURCE_REVISION does).
        return Ok(read(r)?.trim().to_string());
    }
    field_value(record, field)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Error::Invalid("identity field is not a string".into()))
}

struct GitHit {
    full: String,
    checked_out: bool,
    detail: String,
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git_find(repo: &Path, commit: &str) -> Option<GitHit> {
    if !repo.is_dir() {
        return None;
    }
    let full = git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{commit}^{{commit}}"),
        ],
    )?;
    let head = git(repo, &["rev-parse", "HEAD"]).unwrap_or_default();
    let clean =
        git(repo, &["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| s.is_empty());
    let ancestor = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["merge-base", "--is-ancestor", &full, "HEAD"])
        .status()
        .is_ok_and(|s| s.success());
    let branches = git(
        repo,
        &["branch", "--contains", &full, "--format=%(refname:short)"],
    )
    .unwrap_or_default();
    let mut names: Vec<&str> = branches.lines().collect();
    let count = names.len();
    names.truncate(3);
    let current = git(repo, &["branch", "--show-current"]).unwrap_or_default();
    let checked_out = head == full && clean;
    let detail = format!(
        "commit {} {}; on {} branch{} ({}{}); checked out: {}",
        short(&full),
        if checked_out {
            "is the checked-out commit"
        } else if ancestor {
            "is inside the checked-out history"
        } else {
            "is in the history, not the checked-out branch"
        },
        count,
        if count == 1 { "" } else { "es" },
        names.join(", "),
        if count > 3 { ", …" } else { "" },
        if current.is_empty() {
            "detached".to_string()
        } else {
            current
        },
    );
    Some(GitHit {
        full,
        checked_out,
        detail,
    })
}

/// Marks a manifest entry recorded as a link rather than as a file.
const LINK: &str = "\u{1}link:";

/// Compare files under `base` with recorded hashes; links are followed, as a build reads through them.
fn compare(base: &Path, files: &[(String, String)]) -> (usize, Vec<String>) {
    let mut equal = 0;
    let mut differ = Vec::new();
    for (p, want) in files {
        let got = match p.strip_prefix(LINK) {
            Some(link) => fs::read_link(base.join(link))
                .ok()
                .map(|t| hex(&sha256(t.as_os_str().as_encoded_bytes()))),
            None => sha256_file(&base.join(p)),
        };
        match got {
            Some(h) if &h == want => equal += 1,
            _ => differ.push(p.trim_start_matches(LINK).to_string()),
        }
    }
    (equal, differ)
}

/// A Zeno-form manifest: `# source_json "<root>"`, then a header row `path size sha256`.
fn tsv_manifest(p: &Path) -> Result<(PathBuf, Vec<(String, String)>)> {
    let text = read(p)?;
    let mut root = None;
    let mut header: Option<Vec<&str>> = None;
    let mut links: Vec<String> = Vec::new();
    let mut files = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# source_json\t") {
            root = serde_json::from_str::<String>(rest).ok();
            continue;
        }
        if let Some(rest) = line.strip_prefix("# symlinks_json\t") {
            links = serde_json::from_str(rest).unwrap_or_default();
            continue;
        }
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        match &header {
            None => header = Some(cols),
            Some(h) => {
                let at = |name: &str| h.iter().position(|c| *c == name).and_then(|i| cols.get(i));
                if let (Some(path), Some(sha)) = (at("path"), at("sha256")) {
                    files.push((path.to_string(), sha.to_lowercase()));
                }
            }
        }
    }
    let root = root.ok_or_else(|| Error::Invalid(format!("{}: no source_json", p.display())))?;
    // A link the manifest names as a link is recorded by the hash of its target text.
    let files = files
        .into_iter()
        .map(|(path, sha)| {
            if links.contains(&path) {
                (format!("{LINK}{path}"), sha)
            } else {
                (path, sha)
            }
        })
        .collect();
    Ok((PathBuf::from(root), files))
}

/// The walk's release form: every file under the folder, folders in sorted path order, files sorted within;
/// each line is `relative path NUL sha256-hex LF`, hashed together.
pub fn tree_sha256(root: &Path) -> Option<String> {
    let mut dirs = vec![root.to_path_buf()];
    let mut i = 0;
    while i < dirs.len() {
        let d = dirs[i].clone();
        for e in fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            let m = fs::symlink_metadata(&p).ok()?;
            if m.is_dir() {
                dirs.push(p);
            }
        }
        i += 1;
    }
    dirs.sort_by(|a, b| a.as_os_str().cmp(b.as_os_str()));
    let mut lines = Vec::new();
    for d in &dirs {
        let mut names: Vec<_> = fs::read_dir(d)
            .ok()?
            .flatten()
            .filter(|e| fs::symlink_metadata(e.path()).is_ok_and(|m| !m.is_dir()))
            .filter(|e| !(e.path().is_dir()))
            .map(|e| e.file_name())
            .collect();
        names.sort();
        for n in names {
            let p = d.join(&n);
            let rel = p.strip_prefix(root).ok()?.to_string_lossy().into_owned();
            lines.extend_from_slice(rel.as_bytes());
            lines.push(0);
            lines.extend_from_slice(sha256_file(&p)?.as_bytes());
            lines.push(b'\n');
        }
    }
    Some(hex(&sha256(&lines)))
}

pub fn sha256_file(p: &Path) -> Option<String> {
    fs::read(p).ok().map(|b| hex(&sha256(&b)))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 (FIPS 180-4), our own forty lines so the lock file stays as it is.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (a, b) in h.iter_mut().zip(v) {
            *a = a.wrapping_add(b);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(&[b'a'; 1000])),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }
}
