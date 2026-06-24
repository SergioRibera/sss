//! Generic loader for TOML configs with `imports = [...]` support.
//!
//! Semantics:
//!   * relative paths resolve against the importing file's directory
//!   * `~/` expands to `$HOME`
//!   * missing files are skipped with a warning (never an error)
//!   * cycles are broken with a warning
//!   * within a single file, imports are merged left-to-right (later
//!     overrides earlier); the importing file overrides all its imports
//!
//! The actual TOML parsing is left to the caller via a closure so this
//! crate does not need to depend on `toml`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use clap::Parser;
use merge2::Merge;
use serde::{Deserialize, Serialize};

/// Top-level CLI/config fields shared by every binary in the workspace
/// (`sss`, `sss-code`, …). Flatten this into your `ClapConfig` with
/// `#[clap(flatten)]` + `#[serde(flatten)]` so `--config` and `imports`
/// behave identically across binaries.
#[derive(Clone, Debug, Default, Parser, Merge, Serialize, Deserialize)]
pub struct RootArgs {
    #[clap(long, help = "Set custom config file path")]
    #[serde(skip)]
    #[merge(skip)]
    pub config: Option<PathBuf>,
    /// Other TOML files to merge before this one. Paths resolve relative
    /// to the importing file's directory (or `~/` to `$HOME`). Missing
    /// files are skipped with a warning. Within a file, later imports
    /// override earlier ones; the importing file overrides all of its
    /// imports; CLI flags override everything.
    #[clap(skip)]
    #[serde(default)]
    #[merge(skip)]
    pub imports: Vec<PathBuf>,
}

/// Lets the loader pull the `imports` array out of a freshly-parsed
/// config so the recursion can keep walking. Implementors should `take`
/// (not clone) the field — the loader owns the value once it has been
/// pulled out.
pub trait HasImports {
    fn take_imports(&mut self) -> Vec<PathBuf>;
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError<E: std::error::Error + Send + Sync + 'static> {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(E),
}

pub fn load_with_imports<T, E, F>(path: &Path, parse: &F) -> Result<Option<T>, LoadError<E>>
where
    T: Merge + HasImports,
    E: std::error::Error + Send + Sync + 'static,
    F: Fn(&str) -> Result<T, E>,
{
    let mut visited = HashSet::new();
    load_inner(path, parse, &mut visited)
}

fn load_inner<T, E, F>(
    path: &Path,
    parse: &F,
    visited: &mut HashSet<PathBuf>,
) -> Result<Option<T>, LoadError<E>>
where
    T: Merge + HasImports,
    E: std::error::Error + Send + Sync + 'static,
    F: Fn(&str) -> Result<T, E>,
{
    let expanded = expand_user(path);
    let canon = match std::fs::canonicalize(&expanded) {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!("Skipping missing config import: {}", expanded.display());
            return Ok(None);
        }
        Err(e) => return Err(LoadError::Io(e)),
    };
    if !visited.insert(canon.clone()) {
        tracing::warn!("Skipping cyclic config import: {}", canon.display());
        return Ok(None);
    }
    let content = std::fs::read_to_string(&canon)?;
    let mut cfg = parse(&content).map_err(LoadError::Parse)?;
    let base_dir = canon
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let imports = cfg.take_imports();

    let mut acc: Option<T> = None;
    for imp in imports {
        let imp = expand_user(&imp);
        let imp = if imp.is_absolute() {
            imp
        } else {
            base_dir.join(imp)
        };
        if let Some(mut sub) = load_inner(&imp, parse, visited)? {
            match acc.as_mut() {
                Some(a) => a.merge(&mut sub),
                None => acc = Some(sub),
            }
        }
    }
    match acc.as_mut() {
        Some(a) => a.merge(&mut cfg),
        None => acc = Some(cfg),
    }
    Ok(acc)
}

fn expand_user(path: &Path) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path.to_path_buf();
    };
    let Some(rest) = s.strip_prefix("~/") else {
        return path.to_path_buf();
    };
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(rest),
        None => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use merge2::Merge;
    use serde::Deserialize;
    use std::fs;

    #[derive(Default, Deserialize)]
    struct Cfg {
        value: Option<String>,
        #[serde(default)]
        imports: Vec<PathBuf>,
    }

    impl Merge for Cfg {
        fn merge(&mut self, other: &mut Self) {
            if other.value.is_some() {
                self.value = other.value.take();
            }
        }
    }

    impl HasImports for Cfg {
        fn take_imports(&mut self) -> Vec<PathBuf> {
            std::mem::take(&mut self.imports)
        }
    }

    fn parse(s: &str) -> Result<Cfg, toml::de::Error> {
        toml::from_str(s)
    }

    #[test]
    fn relative_dotdot_imports_resolve_against_base_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a/b");
        fs::create_dir_all(&nested).unwrap();
        fs::write(tmp.path().join("base.toml"), "value = \"from-base\"\n").unwrap();
        let main = nested.join("main.toml");
        fs::write(&main, "imports = [\"../../base.toml\"]\n").unwrap();

        let cfg = load_with_imports(&main, &parse).unwrap().unwrap();
        assert_eq!(cfg.value.as_deref(), Some("from-base"));
    }

    #[test]
    fn tilde_imports_expand_home() {
        let tmp = tempfile::tempdir().unwrap();
        let fake_home = tmp.path().join("home");
        fs::create_dir_all(fake_home.join(".themes")).unwrap();
        fs::write(
            fake_home.join(".themes/colors.toml"),
            "value = \"from-home\"\n",
        )
        .unwrap();
        let main = tmp.path().join("main.toml");
        fs::write(&main, "imports = [\"~/.themes/colors.toml\"]\n").unwrap();

        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", &fake_home);
        let cfg = load_with_imports(&main, &parse).unwrap().unwrap();
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        assert_eq!(cfg.value.as_deref(), Some("from-home"));
    }
}
