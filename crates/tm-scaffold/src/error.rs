//! Error type for the scaffold crate.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ScaffoldError {
    #[error("invalid {field} value '{value}' (expected one of: {expected})")]
    BadEnum {
        field: &'static str,
        value: String,
        expected: &'static str,
    },

    #[error("could not infer the VCS provider from the git remote; pass --vcs github|gitlab")]
    VcsUndetermined,

    #[error("a repo manifest already exists at {0}; use `terramantle upgrade` to re-scaffold")]
    ManifestExists(PathBuf),

    #[error("no repo manifest found at {0}; run `terramantle init` first")]
    ManifestMissing(PathBuf),

    #[error("no org resolved; pass --org, set TERRAMANTLE_ORG, or select a context")]
    MissingOrg,

    #[error("this command must be run from inside a git repository")]
    NotAGitRepo,

    #[error("failed to parse HCL: {0}")]
    Hcl(#[from] hcl::Error),

    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to (de)serialize the manifest lock: {0}")]
    Lock(#[from] serde_json::Error),
}
