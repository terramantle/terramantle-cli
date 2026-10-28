//! `tm-scaffold` — the repo-scaffolding engine behind `terramantle init` /
//! `terramantle upgrade` (design: docs/cli/SCAFFOLD-PUBLISH-AUTH.md §1–§3).
//!
//! The crate is split into pure, unit-testable pieces so the CLI layer stays a
//! thin shell:
//!
//! * [`manifest`] — the `terramantle.hcl` model, rendered on `init`, parsed on `upgrade`.
//! * [`vcs`] — infer GitHub vs GitLab from the `origin` remote.
//! * [`render`] — render the desired file set (CI pipeline, CODEOWNERS, skeleton) from a [`Manifest`].
//! * [`plan`] — the lock-free, terraform-plan-style desired-vs-disk diff engine (`+ ~ -`),
//!   using each managed file's provenance header (not a lock file) to recognise ownership.

pub mod backend;
pub mod error;
pub mod manifest;
pub mod pins;
pub mod plan;
pub mod render;
pub mod vcs;

pub use backend::{workspace_from_backend, workspace_from_text};
pub use error::ScaffoldError;
pub use manifest::{
    Artefact, ChangeBase, CiAuth, CiConfig, Discovery, Engine, Manifest, Structure, Terramate,
    VcsProvider, Versioning, DEFAULT_TERRAMATE_VERSION, DEFAULT_TOFU_VERSION, TERRAMATE_STACK_NAME,
};
pub use plan::{compute, Action, ApplyOutcome, Plan, PlanEntry};
pub use render::{desired_files, merge_gitignore, tf_token_var, FileClass, GeneratedFile};
