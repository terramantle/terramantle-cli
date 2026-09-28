//! Error type for the release crate (SCAFFOLD-PUBLISH-AUTH.md §5–§6).

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    /// A `--version` (or a tag) failed a strict semver2 parse (§5: hard error).
    #[error("'{input}' is not a valid semver2 version: {source}")]
    Semver {
        input: String,
        #[source]
        source: semver::Error,
    },

    /// A discovery glob was malformed.
    #[error("invalid discovery glob '{pattern}': {message}")]
    Glob { pattern: String, message: String },

    /// A `mono` manifest reached the release engine without a discovery block
    /// (should be impossible — `Manifest::parse` fills the default).
    #[error("mono repo manifest has no discovery block")]
    MissingDiscovery,

    /// A `git` invocation failed to start (git not installed / not a repo).
    #[error("failed to run git: {0}")]
    GitSpawn(#[source] std::io::Error),

    /// A `git` invocation exited non-zero.
    #[error("git {args} failed: {stderr}")]
    GitCommand { args: String, stderr: String },

    /// A filesystem read/walk failed while packaging.
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Writing the deterministic archive failed.
    #[error("failed to build archive: {0}")]
    Archive(#[source] std::io::Error),
}
