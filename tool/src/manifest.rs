use crate::{Error, Result, absolute, read};
use globset::{GlobBuilder, GlobMatcher};
use serde_json::Value;
use std::path::{Path, PathBuf};
pub struct Namespace {
    pub root: PathBuf,
    pub value: Value,
    records: Vec<(PathBuf, GlobMatcher)>,
}
impl Namespace {
    pub fn load(path: &Path) -> Result<Self> {
        let path = absolute(path)?;
        let root = path
            .parent()
            .ok_or_else(|| Error::Invalid("manifest needs a parent".into()))?
            .to_path_buf();
        let value: Value = serde_json::from_str(&read(&path)?)?;
        let mut ns = Self {
            root,
            value,
            records: vec![],
        };
        if let Some(entries) = ns
            .value
            .pointer("/records/entries")
            .and_then(Value::as_array)
        {
            for e in entries {
                let base = if e["role"] == "host" {
                    ns.root.clone()
                } else {
                    ns.role(e["role"].as_str().unwrap_or(""))?
                };
                if let Some(paths) = e["paths"].as_array() {
                    for p in paths {
                        let s = p
                            .as_str()
                            .ok_or_else(|| Error::Invalid("record glob is not a string".into()))?;
                        ns.records.push((
                            base.clone(),
                            GlobBuilder::new(s)
                                .literal_separator(true)
                                .build()?
                                .compile_matcher(),
                        ));
                    }
                }
            }
        }
        Ok(ns)
    }
    pub fn discover(root: Option<&Path>) -> Result<PathBuf> {
        let mut p = absolute(root.unwrap_or(Path::new(".")))?;
        if root.is_some() {
            return Ok(p.join("ovrin.build.json"));
        }
        loop {
            let m = p.join("ovrin.build.json");
            if m.is_file() {
                return Ok(m);
            }
            if !p.pop() {
                break;
            }
        }
        Err(Error::Invalid(
            "no nearest ovrin.build.json; use --root".into(),
        ))
    }
    pub fn roots(&self) -> Vec<(String, PathBuf)> {
        self.value["roots"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                let dir = r.as_str().or_else(|| r["dir"].as_str())?;
                Some((
                    r["role"].as_str().unwrap_or(dir).into(),
                    self.root.join(dir),
                ))
            })
            .collect()
    }
    pub fn role(&self, role: &str) -> Result<PathBuf> {
        self.roots()
            .into_iter()
            .find(|(r, _)| r == role)
            .map(|(_, p)| p)
            .ok_or_else(|| Error::Invalid(format!("missing root role {role}")))
    }
    pub fn is_record(&self, p: &Path) -> bool {
        if p == self
            .role("constitution")
            .unwrap_or_else(|_| self.root.join("00_constitution"))
            .join("CHANGELOG.md")
        {
            return true;
        }
        p.strip_prefix(&self.root)
            .is_ok_and(|rel| rel.components().any(|c| c.as_os_str() == "records"))
            || self.records.iter().any(|(base, g)| {
                p.strip_prefix(base).is_ok_and(|rel| {
                    g.is_match(rel) || rel.ancestors().skip(1).any(|a| g.is_match(a))
                })
            })
    }
    pub fn catalogues(&self) -> Result<(PathBuf, PathBuf)> {
        let base = self.role("governance")?.join("registry");
        Ok((base.join("names.toml"), base.join("slop.toml")))
    }
}
