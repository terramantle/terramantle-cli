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

    /// The user dismissed a wizard prompt (Esc / Ctrl-C). Distinct from an
    /// error so the caller can map it to the "confirmation refused" exit
    /// rather than reporting a failure the user deliberately chose.
    #[error("cancelled")]
    Cancelled,

    /// A prompt could not be shown or a helper process failed. Carries the
    /// underlying message as text so this crate need not depend on `inquire`.
    #[error("{0}")]
    Prompt(String),

    /// A v1 manifest. 0.9.0 renamed the `ci` block (`tofu`/`terraform_versions`/
    /// `terraform_docs` → `engine`/`tofu_versions`/`docs`) and there is no
    /// migration, so say exactly that rather than letting serde report a
    /// missing field.
    #[error(
        "`{field}` is a v1 manifest field; terramantle 0.9.0 changed the layout and does not \
         migrate. Delete {filename} and re-run `terramantle init`.",
        filename = crate::manifest::Manifest::FILENAME
    )]
    ManifestTooOld { field: &'static str },

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
