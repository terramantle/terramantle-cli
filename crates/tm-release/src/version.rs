//! Semver2 parsing + bump computation (SCAFFOLD-PUBLISH-AUTH.md §5).
//!
//! Versioning is native to the Rust CLI (no Node commitizen at publish time): the
//! [`semver`] crate enforces strict semver2, and [`BumpLevel`] is the folded
//! result of the Conventional-Commit classifier ([`crate::conventional`]).

use serde::Serialize;

pub use semver::Version;

use crate::error::ReleaseError;

/// A version increment. Ordered `None < Patch < Minor < Major` so a set of
/// per-commit levels folds to the highest with [`Ord::max`] (§5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BumpLevel {
    None,
    Patch,
    Minor,
    Major,
}

/// Parse a **strict** semver2 string, rejecting anything the spec disallows
/// (e.g. `1.2`, `v1.2.3`, `1.2.3.4`). A non-semver2 `--version` is a hard error
/// the CLI maps to exit 2 (§5).
pub fn parse_strict(s: &str) -> Result<Version, ReleaseError> {
    let input = s.trim();
    Version::parse(input).map_err(|source| ReleaseError::Semver {
        input: input.to_string(),
        source,
    })
}

/// Apply a bump to a version, resetting the lower components per semver2:
/// major → `(X+1).0.0`, minor → `X.(Y+1).0`, patch → `X.Y.(Z+1)`. Pre-release
/// and build metadata are dropped by a bump (a bumped release is a clean version).
pub fn apply_bump(v: &Version, level: BumpLevel) -> Version {
    match level {
        BumpLevel::None => Version::new(v.major, v.minor, v.patch),
        BumpLevel::Patch => Version::new(v.major, v.minor, v.patch + 1),
        BumpLevel::Minor => Version::new(v.major, v.minor + 1, 0),
        BumpLevel::Major => Version::new(v.major + 1, 0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_strict_accepts_semver2() {
        assert_eq!(parse_strict("1.4.0").unwrap(), Version::new(1, 4, 0));
        assert_eq!(parse_strict("  0.1.0 ").unwrap(), Version::new(0, 1, 0));
        // Pre-release + build metadata are valid semver2.
        assert!(parse_strict("1.0.0-rc.1+build.7").is_ok());
    }

    #[test]
    fn parse_strict_rejects_non_semver2() {
        for bad in ["1.2", "v1.2.3", "1.2.3.4", "latest", ""] {
            assert!(
                matches!(parse_strict(bad), Err(ReleaseError::Semver { .. })),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn apply_bump_resets_lower_components() {
        let v = Version::new(1, 4, 2);
        assert_eq!(apply_bump(&v, BumpLevel::Patch), Version::new(1, 4, 3));
        assert_eq!(apply_bump(&v, BumpLevel::Minor), Version::new(1, 5, 0));
        assert_eq!(apply_bump(&v, BumpLevel::Major), Version::new(2, 0, 0));
        assert_eq!(apply_bump(&v, BumpLevel::None), Version::new(1, 4, 2));
    }

    #[test]
    fn bump_levels_order_highest_last() {
        assert!(BumpLevel::Major > BumpLevel::Minor);
        assert!(BumpLevel::Minor > BumpLevel::Patch);
        assert!(BumpLevel::Patch > BumpLevel::None);
        assert_eq!(
            [BumpLevel::Patch, BumpLevel::Major, BumpLevel::None]
                .into_iter()
                .max()
                .unwrap(),
            BumpLevel::Major
        );
    }
}
