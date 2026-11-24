//! Runtime manifest admission; directories route to admitted descendants.
use crate::{Result, manifest::Namespace};
use globset::{GlobBuilder, GlobMatcher};
use serde_json::Value;
use std::path::{Path, PathBuf};

struct Rule {
    paths: Vec<GlobMatcher>,
    except: Vec<GlobMatcher>,
    rank: u8,
}
fn globs(value: &Value) -> Result<Vec<GlobMatcher>> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| {
            let pattern = v
                .as_str()
                .ok_or_else(|| crate::Error::Invalid("index glob must be a string".into()))?;
            Ok(GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()?
                .compile_matcher())
        })
        .collect()
}
impl Rule {
    fn parse(value: &Value) -> Result<Self> {
        let rank = match value["action"].as_str() {
            Some("embed") => 1,
            Some("list_only") => 2,
            Some("never") => 3,
            _ => return Err(crate::Error::Invalid("invalid index action".into())),
        };
        Ok(Self {
            paths: globs(&value["paths"])?,
            except: globs(&value["except_paths"])?,
            rank,
        })
    }
    fn matches(&self, path: &Path, directory: bool) -> bool {
        let matches = |g: &GlobMatcher| {
            g.is_match(path) || (directory && g.is_match(format!("{}/", path.display())))
        };
        self.paths.iter().any(matches) && !self.except.iter().any(matches)
    }
}
pub struct Policy {
    root: PathBuf,
    roots: Vec<(PathBuf, Vec<Rule>)>,
}
impl Policy {
    pub fn load(ns: &Namespace) -> Result<Option<Self>> {
        let Some(roles) = ns.value.pointer("/index/roles").and_then(Value::as_object) else {
            return Ok(None);
        };
        let mut roots = Vec::new();
        for (role, base) in ns.roots() {
            let mut rules = Vec::new();
            for v in roles
                .get(&role)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .chain(
                    ns.value["index"]["exclusions"]
                        .as_array()
                        .into_iter()
                        .flatten(),
                )
            {
                let mut rule = Rule::parse(v)?;
                // Role-scoped exceptions apply only to the named roles.
                if v["except_roles"]
                    .as_array()
                    .is_some_and(|a| !a.iter().any(|r| r.as_str() == Some(&role)))
                {
                    rule.except.clear();
                }
                rules.push(rule);
            }
            roots.push((base, rules));
        }
        Ok(Some(Self {
            root: ns.root.clone(),
            roots,
        }))
    }
    pub fn admits(&self, path: &Path, directory: bool) -> bool {
        if directory && path == self.root {
            return true;
        }
        let Some((base, rules)) = self.roots.iter().find(|(base, _)| path.starts_with(base)) else {
            return false;
        };
        let Ok(rel) = path.strip_prefix(base) else {
            return false;
        };
        let rank = rules
            .iter()
            .filter(|r| r.matches(rel, directory))
            .map(|r| r.rank)
            .max()
            .unwrap_or(0);
        if directory { rank < 2 } else { rank == 1 }
    }
}
