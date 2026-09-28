//! Tag naming (SCAFFOLD-PUBLISH-AUTH.md §5).
//!
//! poly → repo-wide `v{X}.{Y}.{Z}`; mono → `{module}@{X}.{Y}.{Z}`, which lets each
//! module version independently. `{module}` is the module directory basename.

use semver::Version;
use tm_scaffold::Structure;

/// The tag *prefix* used to list/scope a module's tags: `"v"` for poly (the
/// repo-wide `v*` tags), `"{name}@"` for a mono module (`name` = directory
/// basename). The version follows the prefix directly.
pub fn tag_prefix(structure: Structure, name: &str) -> String {
    match structure {
        Structure::Poly => "v".to_string(),
        Structure::Mono => format!("{name}@"),
    }
}

/// The full tag name for a version: `v{version}` (poly) or `{name}@{version}`
/// (mono). `version`'s `Display` renders pre-release/build metadata when present.
pub fn tag_name(structure: Structure, name: &str, version: &Version) -> String {
    format!("{}{version}", tag_prefix(structure, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly_tag_is_repo_wide() {
        let v = Version::new(1, 4, 0);
        assert_eq!(tag_prefix(Structure::Poly, "anything"), "v");
        assert_eq!(tag_name(Structure::Poly, "anything", &v), "v1.4.0");
    }

    #[test]
    fn mono_tag_is_module_at_version() {
        let v = Version::new(1, 4, 0);
        assert_eq!(tag_prefix(Structure::Mono, "vpc"), "vpc@");
        assert_eq!(tag_name(Structure::Mono, "vpc", &v), "vpc@1.4.0");
    }

    #[test]
    fn tag_name_includes_prerelease() {
        let v = Version::parse("2.0.0-rc.1").unwrap();
        assert_eq!(tag_name(Structure::Poly, "x", &v), "v2.0.0-rc.1");
        assert_eq!(tag_name(Structure::Mono, "db", &v), "db@2.0.0-rc.1");
    }
}
