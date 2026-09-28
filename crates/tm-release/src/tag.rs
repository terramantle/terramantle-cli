//! Tag naming (SCAFFOLD-PUBLISH-AUTH.md §5).
//!
//! poly → repo-wide `v{X}.{Y}.{Z}`; mono → path-prefixed, Go-module style
//! `{dir}/v{X}.{Y}.{Z}`, which lets each artefact version independently.

use semver::Version;
use tm_scaffold::Structure;

/// The tag *prefix* used to list/scope an artefact's tags: `""` for poly,
/// `"{dir}/"` for a mono artefact.
pub fn tag_prefix(structure: Structure, artefact_rel_dir: &str) -> String {
    match structure {
        Structure::Poly => String::new(),
        Structure::Mono => format!("{artefact_rel_dir}/"),
    }
}

/// The full tag name for a version: `v{version}` (poly) or `{dir}/v{version}`
/// (mono). `version`'s `Display` renders pre-release/build metadata when present.
pub fn tag_name(structure: Structure, artefact_rel_dir: &str, version: &Version) -> String {
    format!("{}v{version}", tag_prefix(structure, artefact_rel_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly_tag_is_repo_wide() {
        let v = Version::new(1, 4, 0);
        assert_eq!(tag_prefix(Structure::Poly, "."), "");
        assert_eq!(tag_name(Structure::Poly, ".", &v), "v1.4.0");
    }

    #[test]
    fn mono_tag_is_path_prefixed() {
        let v = Version::new(1, 4, 0);
        assert_eq!(tag_prefix(Structure::Mono, "modules/foo"), "modules/foo/");
        assert_eq!(
            tag_name(Structure::Mono, "modules/foo", &v),
            "modules/foo/v1.4.0"
        );
    }

    #[test]
    fn tag_name_includes_prerelease() {
        let v = Version::parse("2.0.0-rc.1").unwrap();
        assert_eq!(tag_name(Structure::Poly, ".", &v), "v2.0.0-rc.1");
    }
}
