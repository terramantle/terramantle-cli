//! Change detection: expand a manifest into its artefact set and compute each
//! one's last version, changed files, conventional bump, and proposed next
//! version (SCAFFOLD-PUBLISH-AUTH.md §5).
//!
//! poly → one artefact rooted at `.` tagged `v{X.Y.Z}`. mono → each
//! `discovery.include` glob match (minus `exclude`) is an independently versioned
//! artefact tagged `{module}@{X.Y.Z}`.

use std::path::Path;

use semver::Version;
use serde::Serialize;
use tm_scaffold::{Manifest, Structure};

use crate::conventional::{classify_commit, max_bump};
use crate::error::ReleaseError;
use crate::git::GitRepo;
use crate::tag::tag_prefix;
use crate::version::{apply_bump, BumpLevel};

/// The first version stamped when an artefact has never been tagged: `1.0.0`.
/// The initial tag is unconditional (`<module>@1.0.0`); only the *next* release
/// is driven by Conventional Commits since that tag.
fn initial_version() -> Version {
    Version::new(1, 0, 0)
}

/// One discovered artefact, before change detection: display `name` (directory
/// basename) + `rel_dir` (relative to the repo root; `.` for poly).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artefact {
    pub name: String,
    pub rel_dir: String,
}

/// The full change-detection result for one artefact (all artefacts, changed or
/// not — [`changed_artefacts`] filters to the changed ones for the `changed`
/// command).
#[derive(Debug, Clone)]
pub struct ArtefactPlan {
    pub name: String,
    pub rel_dir: String,
    /// The last released version, or `None` when never tagged.
    pub current_version: Option<Version>,
    /// The last tag name (for the `<tag>..HEAD` range), or `None`.
    pub last_tag: Option<String>,
    /// Whether any files changed since the last tag (never-tagged ⇒ always true).
    pub changed: bool,
    pub changed_file_count: usize,
    /// The folded Conventional-Commit bump since the last tag.
    pub bump: BumpLevel,
    /// `apply_bump(current, bump)`, or `1.0.0` when never tagged (§5).
    pub next_version: Version,
}

/// The serde-serializable `changed` row (`-o json` contract, §5). Mirrors
/// [`ArtefactPlan`] with versions rendered as strings.
#[derive(Debug, Clone, Serialize)]
pub struct ChangedArtefact {
    pub name: String,
    pub rel_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_version: Option<String>,
    pub bump: BumpLevel,
    pub next_version: String,
    pub changed_file_count: usize,
}

impl From<&ArtefactPlan> for ChangedArtefact {
    fn from(p: &ArtefactPlan) -> Self {
        ChangedArtefact {
            name: p.name.clone(),
            rel_dir: p.rel_dir.clone(),
            current_version: p.current_version.as_ref().map(Version::to_string),
            bump: p.bump,
            next_version: p.next_version.to_string(),
            changed_file_count: p.changed_file_count,
        }
    }
}

/// Expand `manifest` into its artefact set (no git). poly → one artefact; mono →
/// the `discovery` glob matches (directories), sorted + de-duplicated.
pub fn artefacts(root: &Path, manifest: &Manifest) -> Result<Vec<Artefact>, ReleaseError> {
    match manifest.structure {
        Structure::Poly => {
            let name = root
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| ".".to_string());
            Ok(vec![Artefact {
                name,
                rel_dir: ".".to_string(),
            }])
        }
        Structure::Mono => {
            let discovery = manifest
                .discovery
                .as_ref()
                .ok_or(ReleaseError::MissingDiscovery)?;
            let mut dirs = expand_globs(root, &discovery.include, &discovery.exclude)?;
            dirs.sort();
            dirs.dedup();
            Ok(dirs
                .into_iter()
                .map(|rel_dir| {
                    let name = Path::new(&rel_dir)
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| rel_dir.clone());
                    Artefact { name, rel_dir }
                })
                .collect())
        }
    }
}

/// Compute the [`ArtefactPlan`] for every artefact (changed or not).
pub fn plan_all(root: &Path, manifest: &Manifest) -> Result<Vec<ArtefactPlan>, ReleaseError> {
    let git = GitRepo::new(root);
    let mut plans = Vec::new();
    for artefact in artefacts(root, manifest)? {
        let prefix = tag_prefix(manifest.structure, &artefact.name);
        let last = git.highest_tag(&prefix)?;
        let since_tag = last.as_ref().map(|(t, _)| t.as_str());
        let path = git_path(&artefact.rel_dir);

        let changed_files = git.changed_files(since_tag, &path)?;
        let commits = git.commits_since(since_tag, &path)?;
        let bump = max_bump(commits.iter().map(|c| classify_commit(&c.subject, &c.body)));

        // Never-tagged artefacts are always "changed" (there is nothing to diff
        // against — the whole thing is new).
        let changed = last.is_none() || !changed_files.is_empty();
        let (current_version, next_version) = match &last {
            Some((_, v)) => (Some(v.clone()), apply_bump(v, bump)),
            None => (None, initial_version()),
        };

        plans.push(ArtefactPlan {
            name: artefact.name,
            rel_dir: artefact.rel_dir,
            current_version,
            last_tag: last.map(|(t, _)| t),
            changed,
            changed_file_count: changed_files.len(),
            bump,
            next_version,
        });
    }
    Ok(plans)
}

/// The changed subset as serde rows (§5 `modules changed`). Unchanged artefacts
/// (a prior tag + no changed files) are skipped.
pub fn changed_artefacts(
    root: &Path,
    manifest: &Manifest,
) -> Result<Vec<ChangedArtefact>, ReleaseError> {
    Ok(plan_all(root, manifest)?
        .iter()
        .filter(|p| p.changed)
        .map(ChangedArtefact::from)
        .collect())
}

/// Compile a discovery glob, mapping a syntax error to [`ReleaseError::Glob`].
fn compile_glob(pattern: &str) -> Result<glob::Pattern, ReleaseError> {
    glob::Pattern::new(pattern).map_err(|e| ReleaseError::Glob {
        pattern: pattern.to_string(),
        message: e.to_string(),
    })
}

/// The `git -- <path>` argument for an artefact directory (`.` stays `.`).
fn git_path(rel_dir: &str) -> String {
    if rel_dir.is_empty() || rel_dir == "." {
        ".".to_string()
    } else {
        rel_dir.to_string()
    }
}

/// Expand include globs to directories under `root`, dropping any that match an
/// exclude glob (matched against the repo-relative path).
fn expand_globs(
    root: &Path,
    include: &[String],
    exclude: &[String],
) -> Result<Vec<String>, ReleaseError> {
    // Each exclude glob is compiled as-is, plus a "directory form" with a trailing
    // `/*` or `/**` stripped, so `modules/_archived/*` also excludes the
    // `modules/_archived` directory itself (the artefact match), not only its
    // children.
    let mut excludes: Vec<glob::Pattern> = Vec::new();
    for pattern in exclude {
        excludes.push(compile_glob(pattern)?);
        if let Some(base) = pattern
            .strip_suffix("/**")
            .or_else(|| pattern.strip_suffix("/*"))
        {
            if !base.is_empty() {
                excludes.push(compile_glob(base)?);
            }
        }
    }

    let mut out = Vec::new();
    for pattern in include {
        let joined = root.join(pattern);
        let full = joined.to_string_lossy();
        let entries = glob::glob(&full).map_err(|e| ReleaseError::Glob {
            pattern: pattern.clone(),
            message: e.to_string(),
        })?;
        for entry in entries {
            let path = entry.map_err(|e| ReleaseError::Glob {
                pattern: pattern.clone(),
                message: e.to_string(),
            })?;
            if !path.is_dir() {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if excludes.iter().any(|p| p.matches(&rel)) {
                continue;
            }
            out.push(rel);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tm_scaffold::{Artefact as ArtefactType, CiConfig, Discovery, VcsProvider};

    fn mono_manifest() -> Manifest {
        Manifest {
            structure: Structure::Mono,
            artefact: ArtefactType::Modules,
            org: "acme".into(),
            api_url: None,
            vcs: VcsProvider::Github,
            discovery: Some(Discovery {
                include: vec!["modules/*".into()],
                exclude: vec!["modules/_archived/*".into()],
            }),
            ci: CiConfig::default(),
        }
    }

    fn poly_manifest() -> Manifest {
        Manifest {
            structure: Structure::Poly,
            artefact: ArtefactType::Modules,
            org: "acme".into(),
            api_url: None,
            vcs: VcsProvider::Github,
            discovery: None,
            ci: CiConfig::default(),
        }
    }

    /// Init a throwaway git repo with deterministic identity + no signing.
    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn commit_all(dir: &Path, message: &str) {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-m", message, "--no-gpg-sign"]);
    }

    #[test]
    fn artefacts_mono_expands_and_excludes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for m in ["foo", "bar", "_archived"] {
            fs::create_dir_all(root.join("modules").join(m)).unwrap();
        }
        // A stray file match must not be treated as an artefact directory.
        fs::write(root.join("modules").join("notes.txt"), b"x").unwrap();

        let mut names: Vec<String> = artefacts(root, &mono_manifest())
            .unwrap()
            .into_iter()
            .map(|a| a.name)
            .collect();
        names.sort();
        assert_eq!(names, vec!["bar", "foo"]);
    }

    #[test]
    fn artefacts_poly_is_single_root() {
        let tmp = tempfile::tempdir().unwrap();
        let arts = artefacts(tmp.path(), &poly_manifest()).unwrap();
        assert_eq!(arts.len(), 1);
        assert_eq!(arts[0].rel_dir, ".");
    }

    #[test]
    fn plan_detects_changes_and_bump_over_a_real_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git(root, &["init", "-q", "-b", "main"]);

        fs::create_dir_all(root.join("modules/foo")).unwrap();
        fs::create_dir_all(root.join("modules/bar")).unwrap();
        fs::write(root.join("modules/foo/main.tf"), b"# foo\n").unwrap();
        fs::write(root.join("modules/bar/main.tf"), b"# bar\n").unwrap();
        commit_all(root, "feat: initial");

        // Tag both artefacts at 1.0.0 (module@version scheme).
        git(root, &["tag", "foo@1.0.0"]);
        git(root, &["tag", "bar@1.0.0"]);

        // Change only foo, with a feat commit → minor bump for foo, bar unchanged.
        fs::write(root.join("modules/foo/main.tf"), b"# foo v2\n").unwrap();
        commit_all(root, "feat: extend foo");

        let plans = plan_all(root, &mono_manifest()).unwrap();
        let foo = plans.iter().find(|p| p.name == "foo").unwrap();
        let bar = plans.iter().find(|p| p.name == "bar").unwrap();

        assert!(foo.changed);
        assert_eq!(foo.bump, BumpLevel::Minor);
        assert_eq!(foo.current_version, Some(Version::new(1, 0, 0)));
        assert_eq!(foo.next_version, Version::new(1, 1, 0));

        assert!(!bar.changed);
        assert_eq!(bar.changed_file_count, 0);

        // `changed_artefacts` skips the unchanged bar.
        let changed = changed_artefacts(root, &mono_manifest()).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].name, "foo");
        assert_eq!(changed[0].next_version, "1.1.0");
        assert_eq!(changed[0].current_version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn plan_untagged_artefact_proposes_initial() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git(root, &["init", "-q", "-b", "main"]);
        fs::create_dir_all(root.join("modules/new")).unwrap();
        fs::write(root.join("modules/new/main.tf"), b"# new\n").unwrap();
        commit_all(root, "chore: scaffold");

        let plans = plan_all(root, &mono_manifest()).unwrap();
        let new = plans.iter().find(|p| p.name == "new").unwrap();
        assert!(new.changed, "never-tagged is always changed");
        assert_eq!(new.current_version, None);
        assert_eq!(new.next_version, Version::new(1, 0, 0));
    }
}
