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

pub mod error;
pub mod manifest;
pub mod plan;
pub mod render;
pub mod vcs;

pub use error::ScaffoldError;
pub use manifest::{
    Artefact, CiAuth, CiConfig, Discovery, Manifest, Structure, VcsProvider, Versioning,
};
pub use plan::{compute, Action, ApplyOutcome, Plan, PlanEntry};
pub use render::{desired_files, merge_gitignore, FileClass, GeneratedFile};
