//! `.terramantle/manifest.lock` — the per-file provenance record that turns
//! `upgrade` into a *diff* (SCAFFOLD-PUBLISH-AUTH.md §1, §3).
//!
//! For every file `init`/`upgrade` writes we record the template it came from and
//! the sha256 of the exact bytes we generated. On the next `upgrade` we compare
//! three hashes per file — the freshly-rendered desired bytes, the bytes on disk,
//! and this recorded "last generated" hash — to tell an untouched file (safe to
//! rewrite) apart from a user-edited one (must not clobber). See [`crate::plan`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::ScaffoldError;
use crate::render::{FileClass, GeneratedFile};

/// The lock file's location relative to the repo root.
pub const LOCK_PATH: &str = ".terramantle/manifest.lock";

/// sha256 of arbitrary bytes, lower-hex — the content-address used everywhere.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// One recorded file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockEntry {
    pub template: String,
    pub template_version: u32,
    /// sha256 of the bytes terramantle last generated for this path.
    pub sha256: String,
    /// `true` for scaffold-once files — `upgrade` never rewrites or prunes them.
    #[serde(default)]
    pub once: bool,
}

impl LockEntry {
    /// Build the lock entry for a freshly generated file.
    pub fn from_generated(f: &GeneratedFile) -> Self {
        Self {
            template: f.template.clone(),
            template_version: f.template_version,
            sha256: sha256_hex(f.content.as_bytes()),
            once: f.class == FileClass::Once,
        }
    }
}

/// The on-disk lock model (JSON). `files` is keyed by repo-root-relative path
/// using forward slashes so the lock is stable across platforms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockFile {
    pub version: u32,
    #[serde(default)]
    pub files: BTreeMap<String, LockEntry>,
}

impl Default for LockFile {
    fn default() -> Self {
        Self {
            version: 1,
            files: BTreeMap::new(),
        }
    }
}

/// Normalise a path to a forward-slash, repo-root-relative key.
pub fn key_of(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl LockFile {
    /// Load the lock for the repo rooted at `root`. A missing lock is an empty
    /// default (so `upgrade` can adopt a repo that predates the lock, treating
    /// everything as drifted rather than clobbering it).
    pub fn load(root: &Path) -> Result<Self, ScaffoldError> {
        let path = root.join(LOCK_PATH);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(ScaffoldError::from),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ScaffoldError::Read { path, source }),
        }
    }

    /// Persist the lock under `root`, creating `.terramantle/`.
    pub fn save(&self, root: &Path) -> Result<(), ScaffoldError> {
        let path = root.join(LOCK_PATH);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ScaffoldError::Write {
                path: path.clone(),
                source,
            })?;
        }
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text + "\n").map_err(|source| ScaffoldError::Write { path, source })
    }

    /// The recorded entry for a path, if any.
    pub fn get(&self, path: &Path) -> Option<&LockEntry> {
        self.files.get(&key_of(path))
    }

    /// Record (or replace) a generated file's provenance.
    pub fn record(&mut self, f: &GeneratedFile) {
        self.files
            .insert(key_of(&f.path), LockEntry::from_generated(f));
    }

    /// Drop a path from the lock (after a prune).
    pub fn forget(&mut self, path: &Path) {
        self.files.remove(&key_of(path));
    }

    /// Paths recorded as managed (non-once) — the prune candidates.
    pub fn managed_paths(&self) -> Vec<PathBuf> {
        self.files
            .iter()
            .filter(|(_, e)| !e.once)
            .map(|(k, _)| PathBuf::from(k))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen(path: &str, content: &str, once: bool) -> GeneratedFile {
        GeneratedFile {
            path: PathBuf::from(path),
            content: content.to_string(),
            template: "t".to_string(),
            template_version: 1,
            class: if once {
                FileClass::Once
            } else {
                FileClass::Managed
            },
        }
    }

    #[test]
    fn sha256_is_stable_and_distinct() {
        assert_eq!(sha256_hex(b"abc"), sha256_hex(b"abc"));
        assert_ne!(sha256_hex(b"abc"), sha256_hex(b"abd"));
    }

    #[test]
    fn record_get_forget_roundtrip() {
        let mut lock = LockFile::default();
        let f = gen("a/b.yml", "hello", false);
        lock.record(&f);
        let e = lock.get(&f.path).unwrap();
        assert_eq!(e.sha256, sha256_hex(b"hello"));
        assert!(!e.once);
        lock.forget(&f.path);
        assert!(lock.get(&f.path).is_none());
    }

    #[test]
    fn managed_paths_excludes_once() {
        let mut lock = LockFile::default();
        lock.record(&gen("ci.yml", "x", false));
        lock.record(&gen("main.tf", "y", true));
        let managed = lock.managed_paths();
        assert_eq!(managed, vec![PathBuf::from("ci.yml")]);
    }

    #[test]
    fn persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut lock = LockFile::default();
        lock.record(&gen("ci.yml", "x", false));
        lock.save(dir.path()).unwrap();
        let loaded = LockFile::load(dir.path()).unwrap();
        assert_eq!(loaded, lock);
    }

    #[test]
    fn missing_lock_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(LockFile::load(dir.path()).unwrap(), LockFile::default());
    }
}
