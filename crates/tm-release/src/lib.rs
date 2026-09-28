//! `tm-release` — change detection, versioning, and deterministic packaging
//! behind `terramantle modules changed` / `modules publish`
//! (design: docs/cli/SCAFFOLD-PUBLISH-AUTH.md §5–§6).
//!
//! Pure, unit-tested pieces so the CLI layer stays a thin shell:
//!
//! * [`version`] — strict semver2 parse + [`BumpLevel`] increments.
//! * [`conventional`] — Conventional-Commit → bump classification.
//! * [`git`] — a thin `git -C <root>` wrapper + pure output parsers.
//! * [`tag`] — poly `v{X.Y.Z}` / mono `{module}@{X.Y.Z}` tag naming.
//! * [`changed`] — expand a manifest into artefacts and compute each one's
//!   last version, changed set, bump, and proposed next version.
//! * [`package`] — reproducible `tar.gz` + `sha256` of an artefact dir.

pub mod changed;
pub mod conventional;
pub mod error;
pub mod git;
pub mod package;
pub mod tag;
pub mod version;

pub use changed::{
    artefacts, changed_artefacts, plan_all, Artefact, ArtefactPlan, ChangedArtefact,
};
pub use conventional::{classify_commit, max_bump};
pub use error::ReleaseError;
pub use git::{Commit, GitRepo};
pub use package::{package_dir, sha256_hex, sha256sums_line, PackageOptions};
pub use tag::{tag_name, tag_prefix};
pub use version::{apply_bump, parse_strict, BumpLevel, Version};
