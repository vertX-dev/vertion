//! `--wrap`: copy project files into an intermediate folder so we can safely
//! treat the project root as input without colliding with the output folder.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapMode {
    Temp,
    Perm,
}

impl WrapMode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "temp" => Ok(WrapMode::Temp),
            "perm" => Ok(WrapMode::Perm),
            other => Err(format!(
                "invalid --wrap mode `{}` (expected `temp` or `perm`)",
                other
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            WrapMode::Temp => "temp",
            WrapMode::Perm => "perm",
        }
    }
}

pub const DEFAULT_WRAP_NAME: &str = ".vertion_wrap";

/// Copy every file under `source_root` into `wrap_dir`, skipping anything in
/// `ignored` (absolute paths). The wrap dir itself is also implicitly excluded.
pub fn wrap_project(source_root: &Path, wrap_dir: &Path, ignored: &[PathBuf]) -> io::Result<usize> {
    let source_abs = absolute(source_root);
    let wrap_abs = absolute(wrap_dir);
    if wrap_abs.starts_with(&source_abs) {
        // Wrap dir is inside source — it will be skipped by the walk below since
        // we treat it as an implicit exclusion, but we still need to create it.
    }
    if wrap_abs.exists() {
        // Clear it to avoid stale carry-over from a previous run.
        clear_dir(&wrap_abs)?;
    }
    fs::create_dir_all(&wrap_abs)?;

    let mut copied = 0usize;
    for entry in WalkDir::new(&source_abs).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let abs = absolute(path);
        // Skip the wrap dir itself.
        if abs.starts_with(&wrap_abs) {
            continue;
        }
        // Skip ignored paths.
        if ignored.iter().any(|ig| abs.starts_with(ig)) {
            continue;
        }
        let rel = match abs.strip_prefix(&source_abs) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        let dest = wrap_abs.join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&abs, &dest)?;
        copied += 1;
    }
    Ok(copied)
}

/// Delete the wrap directory. No-op if it doesn't exist. Used after `temp`-mode builds.
pub fn cleanup_wrap(wrap_dir: &Path) -> io::Result<()> {
    if wrap_dir.exists() {
        fs::remove_dir_all(wrap_dir)?;
    }
    Ok(())
}

fn clear_dir(path: &Path) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            fs::remove_dir_all(&p)?;
        } else {
            fs::remove_file(&p)?;
        }
    }
    Ok(())
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpdir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("vertion-wrap-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn wrap_copies_files_excluding_ignored() {
        let root = tmpdir("wrap_basic");
        write_file(&root.join("a.js"), "keep");
        write_file(&root.join("sub/b.js"), "keep");
        write_file(&root.join("build/out.js"), "ignored");
        write_file(&root.join("node_modules/lib.js"), "ignored");

        let wrap = root.join(".vertion_wrap");
        let ignored = vec![root.join("build"), root.join("node_modules")];
        let copied = wrap_project(&root, &wrap, &ignored).unwrap();
        assert_eq!(copied, 2);
        assert!(wrap.join("a.js").exists());
        assert!(wrap.join("sub/b.js").exists());
        assert!(!wrap.join("build").exists());
        assert!(!wrap.join("node_modules").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn wrap_does_not_recurse_into_itself() {
        let root = tmpdir("wrap_self");
        write_file(&root.join("a.js"), "a");
        let wrap = root.join(".vertion_wrap");
        // First wrap creates files inside wrap dir.
        wrap_project(&root, &wrap, &[]).unwrap();
        // Second wrap shouldn't double-copy the wrap dir's contents.
        let copied = wrap_project(&root, &wrap, &[]).unwrap();
        assert_eq!(copied, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cleanup_wrap_removes_directory() {
        let root = tmpdir("wrap_cleanup");
        let wrap = root.join(".vertion_wrap");
        fs::create_dir_all(&wrap).unwrap();
        write_file(&wrap.join("a.js"), "x");
        cleanup_wrap(&wrap).unwrap();
        assert!(!wrap.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cleanup_wrap_is_noop_when_missing() {
        let root = tmpdir("wrap_cleanup_missing");
        let wrap = root.join("nope");
        cleanup_wrap(&wrap).unwrap();
        let _ = fs::remove_dir_all(&root);
    }
}
