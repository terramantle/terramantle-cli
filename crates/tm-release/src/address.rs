//! Deriving a module's registry address from its directory name.
//!
//! Terraform's registry protocol addresses a module as
//! `<namespace>/<name>/<provider>` — `/v1/modules/{ns}/{name}/{provider}/{version}`.
//! The namespace is the org, so the repo only has to supply the other two.
//!
//! v1 derived `name` from the directory basename and had no source at all for
//! `provider`, so `modules publish` hard-errored asking for `--provider` — which
//! the generated pipeline never passed, and every modules repo failed on its
//! first merge to the default branch.
//!
//! The segment comes from the directory name instead, using the convention the
//! public registry already enforces: `terraform-<provider>-<name>`.
//! `modules/terraform-aws-vpc` publishes as `<org>/vpc/aws`. That keeps the two
//! halves of the address in one place the user already controls, rather than in
//! a flag they have to remember and a config file that can disagree with it.
//!
//! The **tag prefix is deliberately unaffected**: tags are cut from
//! `Artefact::name`, the raw basename, so adopting this does not rewrite a
//! repo's release history.

/// A module's registry address, minus the namespace (which is the org).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleAddress {
    /// The `<name>` segment.
    pub name: String,
    /// The `<provider>` segment — the target system, e.g. `aws`.
    pub provider: String,
}

/// Why a directory name could not be turned into an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressError {
    /// Not in `terraform-<provider>-<name>` form.
    NotConventional { dir: String },
}

impl std::fmt::Display for AddressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddressError::NotConventional { dir } => write!(
                f,
                "cannot derive the registry address from directory '{dir}': expected the \
                 `terraform-<provider>-<name>` convention (e.g. `terraform-aws-vpc` publishes \
                 as <org>/vpc/aws). Rename the directory, or pass --provider to set the \
                 provider segment explicitly."
            ),
        }
    }
}

impl std::error::Error for AddressError {}

/// Parse `terraform-<provider>-<name>`.
///
/// The provider is exactly one segment and the name is everything after it, so
/// `terraform-aws-vpc-endpoints` is `vpc-endpoints` on `aws` — matching the
/// public registry, where hyphens in a module name are ordinary.
pub fn from_dir_name(dir: &str) -> Result<ModuleAddress, AddressError> {
    let err = || AddressError::NotConventional {
        dir: dir.to_string(),
    };
    let rest = dir.strip_prefix("terraform-").ok_or_else(err)?;
    let (provider, name) = rest.split_once('-').ok_or_else(err)?;
    if provider.is_empty() || name.is_empty() {
        return Err(err());
    }
    Ok(ModuleAddress {
        name: name.to_string(),
        provider: provider.to_string(),
    })
}

/// The address for an artefact directory, with `--provider` overriding the
/// derived segment.
///
/// The override exists for the repo that does not follow the convention: with
/// it, the directory basename is the name and the flag is the provider, which
/// is exactly v1's behaviour.
pub fn resolve(
    dir_basename: &str,
    provider_override: Option<&str>,
) -> Result<ModuleAddress, AddressError> {
    match (from_dir_name(dir_basename), provider_override) {
        // Convention plus an explicit flag: the flag wins on the provider, but
        // the derived name still beats the raw directory, or `--provider aws`
        // on `terraform-aws-vpc` would publish `terraform-aws-vpc/aws`.
        (Ok(addr), Some(p)) => Ok(ModuleAddress {
            name: addr.name,
            provider: p.to_string(),
        }),
        (Ok(addr), None) => Ok(addr),
        (Err(_), Some(p)) => Ok(ModuleAddress {
            name: dir_basename.to_string(),
            provider: p.to_string(),
        }),
        (Err(e), None) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(name: &str, provider: &str) -> ModuleAddress {
        ModuleAddress {
            name: name.into(),
            provider: provider.into(),
        }
    }

    #[test]
    fn parses_the_registry_convention() {
        assert_eq!(
            from_dir_name("terraform-aws-vpc").unwrap(),
            addr("vpc", "aws")
        );
        assert_eq!(
            from_dir_name("terraform-google-gke").unwrap(),
            addr("gke", "google")
        );
    }

    /// A hyphenated module name is ordinary on the public registry; only the
    /// first segment after the prefix is the provider.
    #[test]
    fn name_keeps_its_hyphens() {
        assert_eq!(
            from_dir_name("terraform-aws-vpc-endpoints").unwrap(),
            addr("vpc-endpoints", "aws")
        );
    }

    #[test]
    fn rejects_anything_not_conventional() {
        for dir in [
            "vpc",
            "tf-aws-vpc",
            "terraform-aws",
            "terraform-",
            "terraform--vpc",
        ] {
            assert!(from_dir_name(dir).is_err(), "{dir} should not parse");
        }
    }

    /// The message has to say what to rename it to, and that --provider exists.
    #[test]
    fn the_error_is_actionable() {
        let msg = from_dir_name("vpc").unwrap_err().to_string();
        assert!(msg.contains("terraform-<provider>-<name>"));
        assert!(msg.contains("--provider"));
        assert!(msg.contains("vpc"));
    }

    /// With the flag on a conventional directory, the derived NAME must still
    /// win — otherwise `terraform-aws-vpc` would publish as
    /// `terraform-aws-vpc/aws`, with the prefix embedded in the address.
    #[test]
    fn override_replaces_only_the_provider_segment() {
        assert_eq!(
            resolve("terraform-aws-vpc", Some("awscc")).unwrap(),
            addr("vpc", "awscc")
        );
    }

    /// The escape hatch for a repo that does not follow the convention: the
    /// basename becomes the name, exactly as v1 behaved.
    #[test]
    fn override_rescues_a_nonconventional_directory() {
        assert_eq!(resolve("vpc", Some("aws")).unwrap(), addr("vpc", "aws"));
    }

    #[test]
    fn no_convention_and_no_override_is_an_error() {
        assert!(resolve("vpc", None).is_err());
    }
}
