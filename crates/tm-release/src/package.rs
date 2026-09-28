//! Deterministic `tar.gz` packaging + hashing (SCAFFOLD-PUBLISH-AUTH.md §6.2–§6.3).
//!
//! An artefact directory is archived reproducibly so its `sha256` is stable: the
//! file set is walked and **sorted by path**, mtimes are zeroed, uid/gid are 0,
//! and modes are normalised (`0644` files / `0755` dirs). The gzip header carries
//! no filename and a zero mtime (flate2's default), so two calls over the same
//! tree yield byte-identical archives with an identical hash.
//!
//! Excluded from every archive: `.git/`, `.terraform/`, `.terramantle/`,
//! `*.tfstate`, `*.tfstate.*`, and — unless [`PackageOptions::include_examples`] —
//! `examples/`.

use std::path::{Path, PathBuf};

use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::error::ReleaseError;

/// Packaging knobs (§6.2).
#[derive(Debug, Clone, Copy, Default)]
pub struct PackageOptions {
    /// Include `examples/` directories in the archive (default: exclude).
    pub include_examples: bool,
}

/// One archive entry, path-relative to the artefact root.
struct Entry {
    rel: String,
    abs: PathBuf,
    is_dir: bool,
}

/// Package `dir` into a deterministic `tar.gz`, returning `(bytes, sha256_hex)`.
pub fn package_dir(dir: &Path, opts: &PackageOptions) -> Result<(Vec<u8>, String), ReleaseError> {
    let mut entries = Vec::new();
    collect(dir, Path::new(""), opts, &mut entries)?;
    // Sort by path so parents precede children and ordering is stable.
    entries.sort_by(|a, b| a.rel.cmp(&b.rel));

    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for entry in &entries {
        let mut header = tar::Header::new_gnu();
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        if entry.is_dir {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_mode(0o755);
            header.set_size(0);
            let path = format!("{}/", entry.rel);
            header.set_path(&path).map_err(ReleaseError::Archive)?;
            header.set_cksum();
            builder
                .append(&header, std::io::empty())
                .map_err(ReleaseError::Archive)?;
        } else {
            let data = std::fs::read(&entry.abs).map_err(|source| ReleaseError::Read {
                path: entry.abs.clone(),
                source,
            })?;
            header.set_mode(0o644);
            header.set_size(data.len() as u64);
            header.set_path(&entry.rel).map_err(ReleaseError::Archive)?;
            header.set_cksum();
            builder
                .append(&header, &data[..])
                .map_err(ReleaseError::Archive)?;
        }
    }
    let encoder = builder.into_inner().map_err(ReleaseError::Archive)?;
    let bytes = encoder.finish().map_err(ReleaseError::Archive)?;
    let hash = sha256_hex(&bytes);
    Ok((bytes, hash))
}

/// Recursively collect entries under `dir`, tracking each one's repo-relative
/// path in `rel_base`.
fn collect(
    dir: &Path,
    rel_base: &Path,
    opts: &PackageOptions,
    out: &mut Vec<Entry>,
) -> Result<(), ReleaseError> {
    let read = std::fs::read_dir(dir).map_err(|source| ReleaseError::Read {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in read {
        let entry = entry.map_err(|source| ReleaseError::Read {
            path: dir.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let rel = rel_base.join(&name);
        if is_excluded(&rel, opts.include_examples) {
            continue;
        }
        let file_type = entry.file_type().map_err(|source| ReleaseError::Read {
            path: entry.path(),
            source,
        })?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if file_type.is_dir() {
            out.push(Entry {
                rel: rel_str,
                abs: entry.path(),
                is_dir: true,
            });
            collect(&entry.path(), &rel, opts, out)?;
        } else if file_type.is_file() {
            out.push(Entry {
                rel: rel_str,
                abs: entry.path(),
                is_dir: false,
            });
        }
        // Symlinks and other special files are skipped (not part of a module).
    }
    Ok(())
}

/// Whether a repo-relative path is excluded from the archive (§6.2).
fn is_excluded(rel: &Path, include_examples: bool) -> bool {
    for component in rel.components() {
        if let Some(name) = component.as_os_str().to_str() {
            match name {
                ".git" | ".terraform" | ".terramantle" => return true,
                "examples" if !include_examples => return true,
                _ => {}
            }
        }
    }
    if let Some(file) = rel.file_name().and_then(|s| s.to_str()) {
        if file.ends_with(".tfstate") || file.contains(".tfstate.") {
            return true;
        }
    }
    false
}

/// The lowercase hex `sha256` of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// One `SHA256SUMS` line in `sha256sum(1)` format: `"<hash>  <name>\n"` (two
/// spaces between the digest and the file name).
pub fn sha256sums_line(name: &str, hash: &str) -> String {
    format!("{hash}  {name}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Read;

    fn fixture(root: &Path) {
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("main.tf"), b"resource {}\n").unwrap();
        fs::write(root.join("sub/vars.tf"), b"variable {}\n").unwrap();
    }

    /// Decode a `tar.gz` into its sorted list of entry paths.
    fn entry_paths(bytes: &[u8]) -> Vec<String> {
        let gz = flate2::read::GzDecoder::new(bytes);
        let mut archive = tar::Archive::new(gz);
        let mut names: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn packaging_is_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let (bytes_a, hash_a) = package_dir(tmp.path(), &PackageOptions::default()).unwrap();
        let (bytes_b, hash_b) = package_dir(tmp.path(), &PackageOptions::default()).unwrap();
        assert_eq!(bytes_a, bytes_b, "same tree → identical archive bytes");
        assert_eq!(hash_a, hash_b, "same tree → identical hash");
        assert_eq!(hash_a.len(), 64);
    }

    #[test]
    fn excludes_noise_and_state_and_examples() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fixture(root);
        fs::create_dir_all(root.join(".terraform/providers")).unwrap();
        fs::write(root.join(".terraform/providers/x"), b"cache").unwrap();
        fs::write(root.join("terraform.tfstate"), b"{}").unwrap();
        fs::write(root.join("terraform.tfstate.backup"), b"{}").unwrap();
        fs::create_dir_all(root.join("examples/basic")).unwrap();
        fs::write(root.join("examples/basic/main.tf"), b"x").unwrap();

        let (bytes, _) = package_dir(root, &PackageOptions::default()).unwrap();
        let paths = entry_paths(&bytes);
        assert!(paths.iter().any(|p| p == "main.tf"));
        assert!(paths.iter().any(|p| p == "sub/vars.tf"));
        assert!(!paths.iter().any(|p| p.contains(".terraform")), "{paths:?}");
        assert!(!paths.iter().any(|p| p.contains("tfstate")), "{paths:?}");
        assert!(!paths.iter().any(|p| p.contains("examples")), "{paths:?}");
    }

    #[test]
    fn examples_kept_when_opted_in() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fixture(root);
        fs::create_dir_all(root.join("examples/basic")).unwrap();
        fs::write(root.join("examples/basic/main.tf"), b"x").unwrap();
        let (bytes, _) = package_dir(
            root,
            &PackageOptions {
                include_examples: true,
            },
        )
        .unwrap();
        let paths = entry_paths(&bytes);
        assert!(paths.iter().any(|p| p.contains("examples")), "{paths:?}");
    }

    #[test]
    fn archived_file_content_is_intact() {
        let tmp = tempfile::tempdir().unwrap();
        fixture(tmp.path());
        let (bytes, _) = package_dir(tmp.path(), &PackageOptions::default()).unwrap();
        let gz = flate2::read::GzDecoder::new(&bytes[..]);
        let mut archive = tar::Archive::new(gz);
        let mut found = false;
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            if entry.path().unwrap().to_string_lossy() == "main.tf" {
                let mut s = String::new();
                entry.read_to_string(&mut s).unwrap();
                assert_eq!(s, "resource {}\n");
                found = true;
            }
        }
        assert!(found, "main.tf missing from archive");
    }

    #[test]
    fn sha256sums_line_is_sha256sum_format() {
        assert_eq!(
            sha256sums_line("m-1.0.0.tar.gz", "abc"),
            "abc  m-1.0.0.tar.gz\n"
        );
    }
}
