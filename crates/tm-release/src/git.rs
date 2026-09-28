//! Git plumbing for change detection (SCAFFOLD-PUBLISH-AUTH.md §5).
//!
//! A thin [`GitRepo`] wrapper shells out to `git -C <root> …`; the parsing is
//! split into pure functions ([`parse_tag_lines`], [`highest_semver_tag`],
//! [`parse_commits`], [`parse_name_only`]) tested on canned output so the logic
//! is covered without a working tree. When there is no prior tag for an artefact,
//! ranges run "since the beginning" (`HEAD` / `git ls-files`).

use std::path::PathBuf;
use std::process::Command;

use semver::Version;

use crate::error::ReleaseError;

/// Field separator between a commit's subject (`%s`) and body (`%b`).
const FIELD_SEP: char = '\u{0}';
/// Record separator between commits.
const RECORD_SEP: char = '\u{1e}';

/// One commit's Conventional-Commit inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub subject: String,
    pub body: String,
}

/// A `git -C <root>` handle.
pub struct GitRepo {
    root: PathBuf,
}

impl GitRepo {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Run `git -C <root> <args>` and return stdout, mapping a non-zero exit or a
    /// spawn failure into [`ReleaseError`].
    fn run(&self, args: &[&str]) -> Result<String, ReleaseError> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .map_err(ReleaseError::GitSpawn)?;
        if !out.status.success() {
            return Err(ReleaseError::GitCommand {
                args: args.join(" "),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// List tags matching `<prefix>v*` (e.g. `v*` for poly, `modules/foo/v*` for
    /// a mono artefact).
    pub fn tags_with_prefix(&self, prefix: &str) -> Result<Vec<String>, ReleaseError> {
        let pattern = format!("{prefix}v*");
        let out = self.run(&["tag", "--list", &pattern])?;
        Ok(parse_tag_lines(&out))
    }

    /// The highest semver tag for `prefix`, as `(tag, version)`, or `None` when
    /// the artefact has never been tagged.
    pub fn highest_tag(&self, prefix: &str) -> Result<Option<(String, Version)>, ReleaseError> {
        let tags = self.tags_with_prefix(prefix)?;
        Ok(highest_semver_tag(&tags, prefix))
    }

    /// Commits in `<since_tag>..HEAD` (or all of `HEAD` when `since_tag` is
    /// `None`), scoped to `path` (`.` for the whole repo).
    pub fn commits_since(
        &self,
        since_tag: Option<&str>,
        path: &str,
    ) -> Result<Vec<Commit>, ReleaseError> {
        let range = match since_tag {
            Some(tag) => format!("{tag}..HEAD"),
            None => "HEAD".to_string(),
        };
        // The format arg carries git's literal `%x00`/`%x1e` placeholders (a real
        // nul in an argv entry is rejected by the OS); git emits the separator
        // bytes in its output, which `parse_commits` then splits on.
        let format = "--pretty=format:%s%x00%b%x1e";
        let out = self.run(&["log", &range, format, "--", path])?;
        Ok(parse_commits(&out))
    }

    /// Files changed in `<since_tag>..HEAD` scoped to `path`. With no prior tag,
    /// "changed since the beginning" is every tracked file under `path`.
    pub fn changed_files(
        &self,
        since_tag: Option<&str>,
        path: &str,
    ) -> Result<Vec<String>, ReleaseError> {
        let out = match since_tag {
            Some(tag) => self.run(&["diff", "--name-only", &format!("{tag}..HEAD"), "--", path])?,
            None => self.run(&["ls-files", "--", path])?,
        };
        Ok(parse_name_only(&out))
    }
}

// ── pure parsers (tested on canned output) ──────────────────────────────────────

/// Split `git tag --list` output into trimmed, non-empty tag names.
pub fn parse_tag_lines(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Pick the highest semver tag from `tags`, stripping `prefix` + the leading `v`
/// before parsing. Tags that don't parse as semver2 are ignored.
pub fn highest_semver_tag(tags: &[String], prefix: &str) -> Option<(String, Version)> {
    tags.iter()
        .filter_map(|tag| {
            let rest = tag.strip_prefix(prefix)?;
            let ver = rest.strip_prefix('v')?;
            Version::parse(ver).ok().map(|v| (tag.clone(), v))
        })
        .max_by(|a, b| a.1.cmp(&b.1))
}

/// Parse `git log --pretty=format:%s\0%b\x1e` output into commits.
pub fn parse_commits(out: &str) -> Vec<Commit> {
    out.split(RECORD_SEP)
        .map(|record| record.trim_start_matches('\n'))
        .filter(|record| !record.is_empty())
        .map(|record| {
            let mut parts = record.splitn(2, FIELD_SEP);
            let subject = parts.next().unwrap_or("").trim().to_string();
            let body = parts.next().unwrap_or("").to_string();
            Commit { subject, body }
        })
        .collect()
}

/// Split `git diff --name-only` / `git ls-files` output into de-duplicated,
/// trimmed, non-empty paths (order preserved).
pub fn parse_name_only(out: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for line in out.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !seen.iter().any(|s| s == line) {
            seen.push(line.to_string());
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tag_lines_trims_and_filters() {
        let out = "v1.0.0\n\nmodules/foo/v1.2.0\n  v0.1.0  \n";
        assert_eq!(
            parse_tag_lines(out),
            vec!["v1.0.0", "modules/foo/v1.2.0", "v0.1.0"]
        );
    }

    #[test]
    fn highest_semver_tag_poly() {
        let tags = vec!["v0.1.0".into(), "v1.4.0".into(), "v1.3.9".into()];
        let (tag, ver) = highest_semver_tag(&tags, "").unwrap();
        assert_eq!(tag, "v1.4.0");
        assert_eq!(ver, Version::new(1, 4, 0));
    }

    #[test]
    fn highest_semver_tag_mono_scopes_by_prefix() {
        let tags = vec![
            "modules/foo/v1.0.0".into(),
            "modules/foo/v1.2.0".into(),
            "modules/bar/v9.0.0".into(),
        ];
        let (tag, ver) = highest_semver_tag(&tags, "modules/foo/").unwrap();
        assert_eq!(tag, "modules/foo/v1.2.0");
        assert_eq!(ver, Version::new(1, 2, 0));
    }

    #[test]
    fn highest_semver_tag_none_when_absent() {
        assert!(highest_semver_tag(&[], "").is_none());
        let tags = vec!["nightly".into(), "v".into()];
        assert!(highest_semver_tag(&tags, "").is_none());
    }

    #[test]
    fn parse_commits_splits_subject_and_body() {
        // Two commits, record-separated; the second has a multi-line body.
        let out = format!(
            "feat: add{FIELD_SEP}{RECORD_SEP}\nfix: bug{FIELD_SEP}BREAKING CHANGE: nope\nmore{RECORD_SEP}"
        );
        let commits = parse_commits(&out);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].subject, "feat: add");
        assert_eq!(commits[0].body, "");
        assert_eq!(commits[1].subject, "fix: bug");
        assert!(commits[1].body.contains("BREAKING CHANGE: nope"));
    }

    #[test]
    fn parse_commits_empty_is_empty() {
        assert!(parse_commits("").is_empty());
    }

    #[test]
    fn parse_name_only_dedups() {
        let out = "modules/foo/main.tf\nmodules/foo/main.tf\nREADME.md\n";
        assert_eq!(
            parse_name_only(out),
            vec!["modules/foo/main.tf", "README.md"]
        );
    }
}
