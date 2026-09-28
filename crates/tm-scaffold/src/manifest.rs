//! The repo manifest — `terramantle.hcl` (SCAFFOLD-PUBLISH-AUTH.md §1).
//!
//! `init`/`upgrade`/`publish` are deterministic only if there is a single source
//! of truth at the repo root. HCL is chosen over YAML to match the Terraform
//! ecosystem the user already lives in.
//!
//! The manifest is **written** by rendering a commented template (so the human
//! gets a self-documenting file — see [`Manifest::render`]) and **read** back with
//! `hcl-rs` serde deserialization (so a hand-edited file still parses). The two
//! directions share the [`Manifest`] model, and [`tests`] proves they round-trip.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

use crate::error::ScaffoldError;

/// Repo layout pattern (§2). `mono` = many independently-versioned artefacts in
/// one repo (path-prefixed tags); `poly` = exactly one artefact per repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Structure {
    Mono,
    Poly,
}

/// The artefact a repo produces. `states` is accepted as an alias for
/// `workspaces` (naming reconciliation, §0) but always normalises to
/// [`Artefact::Workspaces`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Artefact {
    Modules,
    Workspaces,
}

/// Version-control provider, inferred from the `origin` remote (§2). Selects
/// which CI file is emitted (GitHub Actions vs GitLab CI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcsProvider {
    Github,
    Gitlab,
}

/// CI auth grant the generated pipeline uses (§4.1). `oidc` is keyless; `bot`
/// uses client-credentials from CI secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiAuth {
    Oidc,
    Bot,
}

/// Version computation for `publish` (§5). `conventional` parses Conventional
/// Commits; `manual` requires an explicit `--version`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Versioning {
    Conventional,
    Manual,
}

macro_rules! str_enum {
    ($ty:ty, $($variant:ident => $lit:literal),+ $(,)? ; aliases { $($alias:literal => $avar:ident),* $(,)? }) => {
        impl $ty {
            /// The canonical manifest token for this value.
            pub fn as_str(&self) -> &'static str {
                match self { $(<$ty>::$variant => $lit,)+ }
            }
        }
        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
        }
        impl FromStr for $ty {
            type Err = ScaffoldError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s.trim().to_ascii_lowercase().as_str() {
                    $($lit => Ok(<$ty>::$variant),)+
                    $($alias => Ok(<$ty>::$avar),)*
                    other => Err(ScaffoldError::BadEnum {
                        field: stringify!($ty),
                        value: other.to_string(),
                        expected: concat!($($lit, " "),+),
                    }),
                }
            }
        }
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

str_enum!(Structure, Mono => "mono", Poly => "poly"; aliases {});
str_enum!(Artefact, Modules => "modules", Workspaces => "workspaces"; aliases { "states" => Workspaces });
str_enum!(VcsProvider, Github => "github", Gitlab => "gitlab"; aliases {});
str_enum!(CiAuth, Oidc => "oidc", Bot => "bot"; aliases {});
str_enum!(Versioning, Conventional => "conventional", Manual => "manual"; aliases {});

/// The `ci { … }` block (§1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiConfig {
    pub terraform_versions: Vec<String>,
    pub tofu: bool,
    pub auth: CiAuth,
    pub lint: bool,
    pub security_scan: bool,
    pub terraform_docs: bool,
    pub versioning: Versioning,
}

impl Default for CiConfig {
    fn default() -> Self {
        Self {
            terraform_versions: vec!["1.7".into(), "1.9".into()],
            tofu: true,
            auth: CiAuth::Oidc,
            lint: true,
            security_scan: true,
            terraform_docs: true,
            versioning: Versioning::Conventional,
        }
    }
}

/// The `discovery { … }` block — MONO only (§1). Each `include` match is an
/// independently versioned artefact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl Discovery {
    /// The default discovery globs for a mono repo of the given artefact type.
    pub fn default_for(artefact: Artefact) -> Self {
        match artefact {
            Artefact::Modules => Self {
                include: vec!["modules/*".into()],
                exclude: vec!["modules/_archived/*".into()],
            },
            Artefact::Workspaces => Self {
                include: vec!["environments/*".into()],
                exclude: vec![],
            },
        }
    }
}

/// The fully-resolved repo manifest (§1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub structure: Structure,
    pub artefact: Artefact,
    pub org: String,
    /// Optional non-default registry URL; `None` means "use the discovery default".
    pub api_url: Option<String>,
    pub vcs: VcsProvider,
    /// Present iff `structure == Mono`.
    pub discovery: Option<Discovery>,
    pub ci: CiConfig,
}

/// Provenance version stamped into the manifest header; bump when the rendered
/// layout changes so `upgrade` can migrate old files.
pub const MANIFEST_TEMPLATE_VERSION: u32 = 1;

impl Manifest {
    /// The path a manifest lives at, relative to the repo root.
    pub const FILENAME: &'static str = "terramantle.hcl";

    /// Render the manifest as a commented HCL document (the human-facing source
    /// of truth). Deterministic: the same manifest always renders byte-identical.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# managed by terramantle · template=manifest · v={MANIFEST_TEMPLATE_VERSION}\n"
        ));
        out.push_str(
            "# Repo manifest written by `terramantle init`, read by init/upgrade/publish.\n",
        );
        out.push_str("# See docs/cli/SCAFFOLD-PUBLISH-AUTH.md §1.\n");
        out.push_str("terramantle {\n");
        out.push_str(&format!("  structure = {:?}\n", self.structure.as_str()));
        out.push_str(&format!("  artefact  = {:?}\n", self.artefact.as_str()));
        out.push_str(&format!("  org       = {:?}\n", self.org));
        match &self.api_url {
            Some(url) => out.push_str(&format!("  api_url   = {url:?}\n")),
            None => out.push_str(
                "  # api_url defaults to the discovery value; set only to pin a non-prod registry.\n  # api_url = \"https://registry.terramantle.dev\"\n",
            ),
        }
        out.push_str("}\n\n");

        out.push_str("vcs {\n");
        out.push_str(&format!("  provider = {:?}\n", self.vcs.as_str()));
        out.push_str("}\n");

        if let Some(d) = &self.discovery {
            out.push('\n');
            out.push_str(
                "# MONO only — each match is an independently versioned artefact (tag: <name>/vX.Y.Z).\n",
            );
            out.push_str("discovery {\n");
            out.push_str(&format!("  include = {}\n", render_string_list(&d.include)));
            out.push_str(&format!("  exclude = {}\n", render_string_list(&d.exclude)));
            out.push_str("}\n");
        }

        let ci = &self.ci;
        out.push('\n');
        out.push_str("ci {\n");
        out.push_str(&format!(
            "  terraform_versions = {}\n",
            render_string_list(&ci.terraform_versions)
        ));
        out.push_str(&format!("  tofu               = {}\n", ci.tofu));
        out.push_str(&format!("  auth               = {:?}\n", ci.auth.as_str()));
        out.push_str(&format!("  lint               = {}\n", ci.lint));
        out.push_str(&format!("  security_scan      = {}\n", ci.security_scan));
        out.push_str(&format!("  terraform_docs     = {}\n", ci.terraform_docs));
        out.push_str(&format!(
            "  versioning         = {:?}\n",
            ci.versioning.as_str()
        ));
        out.push_str("}\n");
        out
    }

    /// Parse a manifest from HCL text (a hand-editable `terramantle.hcl`).
    pub fn parse(text: &str) -> Result<Self, ScaffoldError> {
        let raw: RawManifest = hcl::from_str(text).map_err(ScaffoldError::Hcl)?;
        raw.into_manifest()
    }
}

fn render_string_list(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| format!("{s:?}")).collect();
    format!("[{}]", inner.join(", "))
}

// ---- HCL deserialization shadow types -------------------------------------
//
// hcl-rs maps each unlabeled block to a struct field. We deserialize into these
// permissive shadows, then normalise/validate into the strict `Manifest`.

#[derive(Debug, Deserialize)]
struct RawManifest {
    terramantle: RawCore,
    vcs: RawVcs,
    #[serde(default)]
    discovery: Option<RawDiscovery>,
    ci: RawCi,
}

#[derive(Debug, Deserialize)]
struct RawCore {
    structure: Structure,
    artefact: Artefact,
    org: String,
    #[serde(default)]
    api_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawVcs {
    provider: VcsProvider,
}

#[derive(Debug, Deserialize)]
struct RawDiscovery {
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawCi {
    terraform_versions: Vec<String>,
    tofu: bool,
    auth: CiAuth,
    lint: bool,
    security_scan: bool,
    terraform_docs: bool,
    versioning: Versioning,
}

impl RawManifest {
    fn into_manifest(self) -> Result<Manifest, ScaffoldError> {
        let structure = self.terramantle.structure;
        // Mono repos require a discovery block; poly repos must not carry one.
        let discovery = match (structure, self.discovery) {
            (Structure::Mono, Some(d)) => Some(Discovery {
                include: d.include,
                exclude: d.exclude,
            }),
            (Structure::Mono, None) => Some(Discovery::default_for(self.terramantle.artefact)),
            (Structure::Poly, _) => None,
        };
        Ok(Manifest {
            structure,
            artefact: self.terramantle.artefact,
            org: self.terramantle.org,
            api_url: self.terramantle.api_url.filter(|s| !s.is_empty()),
            vcs: self.vcs.provider,
            discovery,
            ci: CiConfig {
                terraform_versions: self.ci.terraform_versions,
                tofu: self.ci.tofu,
                auth: self.ci.auth,
                lint: self.ci.lint,
                security_scan: self.ci.security_scan,
                terraform_docs: self.ci.terraform_docs,
                versioning: self.ci.versioning,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono_modules() -> Manifest {
        Manifest {
            structure: Structure::Mono,
            artefact: Artefact::Modules,
            org: "acme".into(),
            api_url: None,
            vcs: VcsProvider::Github,
            discovery: Some(Discovery::default_for(Artefact::Modules)),
            ci: CiConfig::default(),
        }
    }

    #[test]
    fn render_parse_roundtrips_mono() {
        let m = mono_modules();
        let parsed = Manifest::parse(&m.render()).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn render_parse_roundtrips_poly_workspaces() {
        let m = Manifest {
            structure: Structure::Poly,
            artefact: Artefact::Workspaces,
            org: "beta".into(),
            api_url: Some("https://staging.example.com".into()),
            vcs: VcsProvider::Gitlab,
            discovery: None,
            ci: CiConfig {
                lint: false,
                versioning: Versioning::Manual,
                terraform_versions: vec!["1.9".into()],
                ..CiConfig::default()
            },
        };
        let parsed = Manifest::parse(&m.render()).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn render_is_deterministic() {
        assert_eq!(mono_modules().render(), mono_modules().render());
    }

    #[test]
    fn states_alias_normalises_to_workspaces() {
        assert_eq!("states".parse::<Artefact>().unwrap(), Artefact::Workspaces);
        assert_eq!(
            "WORKSPACES".parse::<Artefact>().unwrap(),
            Artefact::Workspaces
        );
    }

    #[test]
    fn bad_enum_is_rejected() {
        let err = "svn".parse::<VcsProvider>().unwrap_err();
        assert!(matches!(err, ScaffoldError::BadEnum { .. }));
    }

    #[test]
    fn poly_manifest_drops_discovery_even_if_present() {
        // A hand-edited poly manifest with a stray discovery block ignores it.
        let text = r#"
            terramantle {
              structure = "poly"
              artefact  = "modules"
              org       = "x"
            }
            vcs { provider = "github" }
            discovery {
              include = ["modules/*"]
              exclude = []
            }
            ci {
              terraform_versions = ["1.9"]
              tofu = true
              auth = "oidc"
              lint = true
              security_scan = true
              terraform_docs = true
              sign = "cosign"
              versioning = "conventional"
            }
        "#;
        let m = Manifest::parse(text).unwrap();
        assert_eq!(m.structure, Structure::Poly);
        assert_eq!(m.discovery, None);
    }
}
