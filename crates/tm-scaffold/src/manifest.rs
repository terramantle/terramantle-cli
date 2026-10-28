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

/// The IaC binary the generated pipeline drives. v2 is OpenTofu-only in
/// practice; the enum exists so a `terraform` value parses to a clear "not
/// supported" rather than a serde error, and so the manifest never lies about
/// which binary the pipeline installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Tofu,
}

str_enum!(Engine, Tofu => "tofu"; aliases {});

/// Which commit `terramate --changed` diffs against. The pipeline's own default
/// (`git merge-base` vs the PR target, `HEAD^` on the default branch) is already
/// correct for merge, squash *and* rebase merges, so this exists as an escape
/// hatch for repos with a non-standard base — not as a correctness requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeBase {
    /// Derive the base in CI (merge-base / `HEAD^`). The right answer almost always.
    Auto,
    /// Always diff the whole repo — every stack is selected on every run.
    All,
}

str_enum!(ChangeBase, Auto => "auto", All => "all"; aliases {});

/// The `ci { … }` block (§1), v2 shape.
///
/// Renamed from v1: `tofu: bool` → [`Engine`]; `terraform_versions` →
/// `tofu_versions`; `terraform_docs` → `docs`. [`Manifest::parse`] rejects the v1
/// names outright rather than silently defaulting them — see
/// [`ScaffoldError::ManifestTooOld`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiConfig {
    pub engine: Engine,
    /// OpenTofu versions the pipeline pins. The first entry is the version the
    /// plan and apply jobs both install — they MUST match, because an apply
    /// consumes a plan file produced by the other job.
    pub tofu_versions: Vec<String>,
    pub auth: CiAuth,
    pub lint: bool,
    pub security_scan: bool,
    pub docs: bool,
    pub versioning: Versioning,
    /// The branch the pipeline treats as the trunk (apply runs here only).
    pub default_branch: String,
    /// GitHub runner label; ignored for GitLab, which uses the image in `.tools`.
    pub runner: String,
    /// Post a sticky plan summary on the PR / MR.
    pub pr_comment: bool,
    pub change_base: ChangeBase,
}

/// The OpenTofu release the generated pipeline installs. Pinned, not `latest`:
/// the plan and apply jobs must agree, and a floating version would silently
/// change that contract mid-release.
pub const DEFAULT_TOFU_VERSION: &str = "1.13.2";

impl Default for CiConfig {
    fn default() -> Self {
        Self {
            engine: Engine::Tofu,
            tofu_versions: vec![DEFAULT_TOFU_VERSION.into()],
            auth: CiAuth::Oidc,
            lint: true,
            security_scan: true,
            docs: true,
            versioning: Versioning::Conventional,
            default_branch: "main".into(),
            runner: "ubuntu-latest".into(),
            pr_comment: true,
            change_base: ChangeBase::Auto,
        }
    }
}

impl CiConfig {
    /// The single OpenTofu version plan and apply both install. Taking the first
    /// entry (rather than a matrix) is deliberate: a saved plan is only valid for
    /// the binary that produced it.
    pub fn tofu_version(&self) -> &str {
        self.tofu_versions
            .first()
            .map(String::as_str)
            .unwrap_or(DEFAULT_TOFU_VERSION)
    }
}

/// The Terramate release the generated pipeline installs. `terramate-action@v3`
/// removed its fallback to "latest" and hard-`exit 1`s without either an explicit
/// `version:` input or a committed `.tool-versions`, so this is not optional.
pub const DEFAULT_TERRAMATE_VERSION: &str = "0.17.3";

/// The `terramate { … }` block — WORKSPACES only.
///
/// When present, `terramate generate` owns each stack's `_backend.tf` and the
/// scaffolder does not write per-stack backends at all. That split matters: the
/// diff engine keys off a fixed path list, so it could never reconcile a
/// per-stack file whose path depends on what is on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terramate {
    pub version: String,
    /// The Terramantle state workspace each stack binds to.
    ///
    /// MUST resolve to the stack's directory basename: `state publish` derives
    /// the workspace name from the directory (`workspace_units` in the CLI), so
    /// any other value uploads the lock file to a different workspace than the
    /// one holding the state.
    pub workspace_name: String,
}

/// The only safe default for [`Terramate::workspace_name`] — see its docs.
pub const TERRAMATE_WORKSPACE_EXPR: &str = "terramate.stack.name";

impl Default for Terramate {
    fn default() -> Self {
        Self {
            version: DEFAULT_TERRAMATE_VERSION.into(),
            workspace_name: TERRAMATE_WORKSPACE_EXPR.into(),
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
            // `stacks/*` rather than `environments/*`: it is the Terramate
            // convention, and `discovery.include` must match the stack layout or
            // `state publish` discovers zero workspaces and exits without
            // uploading anything.
            Artefact::Workspaces => Self {
                include: vec!["stacks/*".into()],
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
    /// Present iff the repo orchestrates stacks with Terramate (workspaces only).
    pub terramate: Option<Terramate>,
    pub ci: CiConfig,
}

/// Provenance version stamped into the manifest header.
///
/// v2 renamed the `ci` block's fields and added `terramate`. There is no
/// migration: [`Manifest::parse`] detects a v1 document and errors with
/// [`ScaffoldError::ManifestTooOld`] telling the user to re-run `init`.
pub const MANIFEST_TEMPLATE_VERSION: u32 = 2;

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

        if let Some(t) = &self.terramate {
            out.push('\n');
            out.push_str(
                "# WORKSPACES only. `terramate generate` owns each stack's _backend.tf, so the\n",
            );
            out.push_str(
                "# scaffolder does not write per-stack backends. Run `terramate generate` after\n",
            );
            out.push_str("# editing terramate.tm.hcl and commit the result — CI fails on drift.\n");
            out.push_str("terramate {\n");
            out.push_str(&format!("  version        = {:?}\n", t.version));
            out.push_str(
                "  # MUST resolve to the stack's directory basename: `state publish` derives the\n",
            );
            out.push_str(
                "  # workspace name from the directory, so anything else uploads the lock file to\n",
            );
            out.push_str("  # a different workspace than the one holding the state.\n");
            out.push_str(&format!("  workspace_name = {:?}\n", t.workspace_name));
            out.push_str("}\n");
        }

        let ci = &self.ci;
        out.push('\n');
        out.push_str("ci {\n");
        out.push_str(&format!("  engine         = {:?}\n", ci.engine.as_str()));
        out.push_str(
            "  # Plan and apply install the SAME version — a saved plan is only valid for the\n",
        );
        out.push_str("  # binary that produced it. The first entry is the one CI uses.\n");
        out.push_str(&format!(
            "  tofu_versions  = {}\n",
            render_string_list(&ci.tofu_versions)
        ));
        out.push_str(&format!("  auth           = {:?}\n", ci.auth.as_str()));
        out.push_str(&format!("  lint           = {}\n", ci.lint));
        out.push_str(&format!("  security_scan  = {}\n", ci.security_scan));
        out.push_str(&format!("  docs           = {}\n", ci.docs));
        out.push_str(&format!(
            "  versioning     = {:?}\n",
            ci.versioning.as_str()
        ));
        out.push_str(&format!("  default_branch = {:?}\n", ci.default_branch));
        out.push_str(&format!("  runner         = {:?}\n", ci.runner));
        out.push_str(&format!("  pr_comment     = {}\n", ci.pr_comment));
        out.push_str(&format!(
            "  change_base    = {:?}\n",
            ci.change_base.as_str()
        ));
        out.push_str("}\n");
        out
    }

    /// Parse a manifest from HCL text (a hand-editable `terramantle.hcl`).
    ///
    /// A v1 document is rejected *before* deserialization. Letting it fall
    /// through would surface as `missing field 'engine'`, which tells the user
    /// nothing about what actually happened or what to do about it.
    pub fn parse(text: &str) -> Result<Self, ScaffoldError> {
        if let Some(field) = v1_marker(text) {
            return Err(ScaffoldError::ManifestTooOld { field });
        }
        let raw: RawManifest = hcl::from_str(text).map_err(ScaffoldError::Hcl)?;
        raw.into_manifest()
    }
}

/// Detect a v1 `ci` block by the fields v2 renamed, returning the first one
/// found so the error can name it.
///
/// Deliberately a text scan rather than a parse: a v1 document does not
/// deserialize into either shape, so there is nothing to inspect structurally.
/// Comment lines are skipped — v2's own rendered output mentions `tofu_versions`
/// in prose, and a stray word in a comment must not condemn a valid manifest.
fn v1_marker(text: &str) -> Option<&'static str> {
    const RENAMED: &[&str] = &["terraform_versions", "terraform_docs"];
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        // The token immediately before the first `=`, so both `terraform_docs =
        // true` on its own line and an inline `ci { tofu = true }` resolve to
        // the bare field name, while a substring inside a string value cannot
        // trip it.
        let key = line
            .split('=')
            .next()
            .unwrap_or("")
            .split_whitespace()
            .next_back()
            .unwrap_or("");
        if RENAMED.contains(&key) {
            return RENAMED.iter().find(|r| **r == key).copied();
        }
        // v1's `tofu = true` became `engine = "tofu"`. Keyed on the bare name so
        // v2's `tofu_versions` does not match.
        if key == "tofu" {
            return Some("tofu");
        }
    }
    None
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
    #[serde(default)]
    terramate: Option<RawTerramate>,
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
struct RawTerramate {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    workspace_name: Option<String>,
}

/// Every field that has a safe default is `Option`, so a hand-trimmed manifest
/// still parses. The ones without defaults (`engine`, `tofu_versions`, `auth`,
/// the booleans, `versioning`) are the v1→v2 renames: making those optional
/// would let a v1 document parse as a v2 one with silently different meaning.
#[derive(Debug, Deserialize)]
struct RawCi {
    engine: Engine,
    tofu_versions: Vec<String>,
    auth: CiAuth,
    lint: bool,
    security_scan: bool,
    docs: bool,
    versioning: Versioning,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    runner: Option<String>,
    #[serde(default)]
    pr_comment: Option<bool>,
    #[serde(default)]
    change_base: Option<ChangeBase>,
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
        // Terramate orchestrates stacks, which only a workspaces repo has. A
        // stray block on a modules repo is dropped, mirroring how a stray
        // `discovery` block is dropped on a poly repo.
        let artefact = self.terramantle.artefact;
        let defaults = Terramate::default();
        let terramate = match (artefact, self.terramate) {
            (Artefact::Workspaces, Some(t)) => Some(Terramate {
                version: t
                    .version
                    .filter(|s| !s.is_empty())
                    .unwrap_or(defaults.version),
                workspace_name: t
                    .workspace_name
                    .filter(|s| !s.is_empty())
                    .unwrap_or(defaults.workspace_name),
            }),
            _ => None,
        };
        let ci_defaults = CiConfig::default();
        Ok(Manifest {
            structure,
            artefact,
            org: self.terramantle.org,
            api_url: self.terramantle.api_url.filter(|s| !s.is_empty()),
            vcs: self.vcs.provider,
            discovery,
            terramate,
            ci: CiConfig {
                engine: self.ci.engine,
                tofu_versions: self.ci.tofu_versions,
                auth: self.ci.auth,
                lint: self.ci.lint,
                security_scan: self.ci.security_scan,
                docs: self.ci.docs,
                versioning: self.ci.versioning,
                default_branch: self
                    .ci
                    .default_branch
                    .filter(|s| !s.is_empty())
                    .unwrap_or(ci_defaults.default_branch),
                runner: self
                    .ci
                    .runner
                    .filter(|s| !s.is_empty())
                    .unwrap_or(ci_defaults.runner),
                pr_comment: self.ci.pr_comment.unwrap_or(ci_defaults.pr_comment),
                change_base: self.ci.change_base.unwrap_or(ci_defaults.change_base),
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
            terramate: None,
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
            terramate: None,
            ci: CiConfig {
                lint: false,
                versioning: Versioning::Manual,
                tofu_versions: vec!["1.9".into()],
                ..CiConfig::default()
            },
        };
        let parsed = Manifest::parse(&m.render()).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn render_parse_roundtrips_terramate_workspaces() {
        let m = Manifest {
            structure: Structure::Mono,
            artefact: Artefact::Workspaces,
            org: "acme-demo".into(),
            api_url: None,
            vcs: VcsProvider::Github,
            discovery: Some(Discovery::default_for(Artefact::Workspaces)),
            terramate: Some(Terramate::default()),
            ci: CiConfig {
                change_base: ChangeBase::All,
                pr_comment: false,
                default_branch: "trunk".into(),
                ..CiConfig::default()
            },
        };
        let parsed = Manifest::parse(&m.render()).unwrap();
        assert_eq!(parsed, m);
    }

    /// Workspaces discovery must point at the Terramate stack layout: a
    /// mismatch here makes `state publish` discover zero workspaces.
    #[test]
    fn workspaces_discovery_defaults_to_stacks() {
        assert_eq!(
            Discovery::default_for(Artefact::Workspaces).include,
            vec!["stacks/*".to_string()]
        );
    }

    /// The workspace expression must resolve to the stack's directory basename,
    /// because `state publish` derives the workspace name from the directory.
    #[test]
    fn terramate_workspace_name_defaults_to_the_stack_basename() {
        assert_eq!(Terramate::default().workspace_name, "terramate.stack.name");
    }

    #[test]
    fn terramate_block_is_dropped_on_a_modules_repo() {
        let mut m = mono_modules();
        m.terramate = Some(Terramate::default());
        // Rendered for a modules repo, the block round-trips back to None.
        let parsed = Manifest::parse(&m.render()).unwrap();
        assert_eq!(parsed.terramate, None);
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
              engine = "tofu"
              tofu_versions = ["1.13.2"]
              auth = "oidc"
              lint = true
              security_scan = true
              docs = true
              sign = "cosign"
              versioning = "conventional"
            }
        "#;
        let m = Manifest::parse(text).unwrap();
        assert_eq!(m.structure, Structure::Poly);
        assert_eq!(m.discovery, None);
    }

    /// A v1 manifest must be named and refused, not reported as a serde error
    /// about a field the user never heard of.
    #[test]
    fn v1_manifest_is_refused_by_name() {
        let v1 = r#"
            terramantle { structure = "poly" artefact = "modules" org = "x" }
            vcs { provider = "github" }
            ci {
              terraform_versions = ["1.7", "1.9"]
              tofu = true
              auth = "oidc"
              lint = true
              security_scan = true
              terraform_docs = true
              versioning = "conventional"
            }
        "#;
        let err = Manifest::parse(v1).unwrap_err();
        match err {
            ScaffoldError::ManifestTooOld { field } => assert_eq!(field, "terraform_versions"),
            other => panic!("expected ManifestTooOld, got {other:?}"),
        }
        assert!(Manifest::parse(v1)
            .unwrap_err()
            .to_string()
            .contains("terramantle init"));
    }

    /// `tofu = true` (v1) must be caught, but v2's own `tofu_versions` must not
    /// be mistaken for it.
    #[test]
    fn v1_detection_keys_on_the_whole_field_name() {
        let v1_bool = "terramantle { structure = \"poly\" artefact = \"modules\" org = \"x\" }\n\
                       vcs { provider = \"github\" }\nci { tofu = true }\n";
        assert!(matches!(
            Manifest::parse(v1_bool).unwrap_err(),
            ScaffoldError::ManifestTooOld { field: "tofu" }
        ));
        // A valid v2 manifest renders the word `tofu_versions` and mentions the
        // renames in comments; neither may trip the check.
        let v2 = mono_modules().render();
        assert!(v2.contains("tofu_versions"));
        assert!(Manifest::parse(&v2).is_ok());
    }
}
