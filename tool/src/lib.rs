pub mod canon;
pub mod cli;
pub mod document;
pub mod filenames;
pub mod fix;
pub mod manifest;
pub mod preflight;

use std::{
    fmt, fs,
    path::{Path, PathBuf},
};
#[derive(Debug)]
pub enum Error {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Json(serde_json::Error),
    Toml(toml::de::Error),
    Regex(regex::Error),
    Glob(globset::Error),
    Invalid(String),
}
pub type Result<T> = std::result::Result<T, Error>;
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Json(e) => e.fmt(f),
            Self::Toml(e) => e.fmt(f),
            Self::Regex(e) => e.fmt(f),
            Self::Glob(e) => e.fmt(f),
            Self::Invalid(s) => s.fmt(f),
        }
    }
}
impl std::error::Error for Error {}
macro_rules! from_error {
    ($t:ty,$v:ident) => {
        impl From<$t> for Error {
            fn from(e: $t) -> Self {
                Self::$v(e)
            }
        }
    };
}
from_error!(serde_json::Error, Json);
from_error!(toml::de::Error, Toml);
from_error!(regex::Error, Regex);
from_error!(globset::Error, Glob);
pub fn io<T>(p: &Path, r: std::io::Result<T>) -> Result<T> {
    r.map_err(|source| Error::Io {
        path: p.into(),
        source,
    })
}
pub fn read(p: &Path) -> Result<String> {
    io(p, fs::read_to_string(p))
}
pub fn absolute(p: &Path) -> Result<PathBuf> {
    io(p, fs::canonicalize(p))
}
/// Walk without following links. Explicit symlink inputs are refused as well.
pub fn files(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    fn visit(p: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        let m = io(p, fs::symlink_metadata(p))?;
        if m.file_type().is_symlink() {
            return Ok(());
        }
        if m.is_dir() {
            for e in io(p, fs::read_dir(p))? {
                let e = io(p, e)?;
                visit(&e.path(), out)?;
            }
        } else if m.is_file() {
            out.push(absolute(p)?);
        }
        Ok(())
    }
    let mut out = Vec::new();
    for p in paths {
        visit(p, &mut out)?;
    }
    out.sort();
    out.dedup();
    Ok(out)
}

pub mod migration;

pub mod allowance;

pub mod paths;

pub mod index;

pub mod releases;

pub mod drift;
