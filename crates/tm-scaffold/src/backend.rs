//! Reading the Terramantle backend address out of a stack's Terraform configuration.
//!
//! Two commands need it. `state publish` has to know which workspace a directory's
//! state lives in (it used to guess the directory basename, which is right only when
//! nothing customised the backend address — `state-sample-mono-repo` names its
//! workspaces `state-sample-<stack>`, so the lock file went to `network` while the
//! state sat in `state-sample-network`, and neither call errored). `plan push` /
//! `plan pull` need the whole address, because the plan surface hangs off it:
//! `<address>/plan`, exactly as terraform derives `lock_address` from `address`.
//!
//! The backend block is authoritative — it is literally the URL the state is pushed
//! to — so parsing it removes the guess. Three sources, in precedence order
//! ([`resolve_backend`]):
//!
//! 1. an explicit address the caller passed (`--address`);
//! 2. `.terraform/terraform.tfstate`, which `terraform init` writes with the
//!    RESOLVED backend config — including anything supplied through
//!    `-backend-config=` flags or files, which the HCL alone cannot show;
//! 3. the `terraform { backend "http" { address = … } }` block in `_backend.tf`,
//!    `backend.tf`, then any `*.tf` in the directory, parsed with `hcl-rs` (already a
//!    dependency for the manifest; a full HCL parser, so comments, nested blocks and
//!    attribute order are not our problem).
//!
//! A non-`http` backend (`cloud`, `remote`, `s3`, …) and an address that is not a
//! literal (`"${var.x}"`) are distinct errors, because the fix is different.

use std::fmt;
use std::path::Path;

/// Backend filenames tried first, in precedence order. `_backend.tf` is what
/// `terramate generate` emits, and it wins: in a Terramate repo a hand-written
/// `backend.tf` is the stale one. Any other `*.tf` is tried after these.
const BACKEND_FILES: &[&str] = &["_backend.tf", "backend.tf"];

/// Where `terraform init` records the resolved backend configuration.
const INIT_STATE: &str = ".terraform/terraform.tfstate";

/// A Terramantle `http` backend address, split into the parts the CLI addresses
/// with: `{api_origin}/state/{org}/{workspace}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendAddress {
    /// Scheme + host (+ port), no trailing slash, e.g. `https://registry.terramantle.dev`.
    pub api_origin: String,
    pub org: String,
    /// The workspace KEY, which may contain slashes (`stacks/data/platform`).
    pub workspace: String,
}

impl BackendAddress {
    /// Parse a full backend `address` URL. `None` when it is not a Terramantle
    /// state address (`…/state/{org}/{key}`), is interpolated, or has no key.
    pub fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        if url.contains("${") {
            return None;
        }
        let (origin, rest) = url.split_once("/state/")?;
        if !(origin.starts_with("https://") || origin.starts_with("http://")) {
            return None;
        }
        let rest = rest.split(['?', '#']).next().unwrap_or(rest);
        let (org, workspace) = rest.split_once('/')?;
        let workspace = workspace.trim_end_matches('/');
        if org.is_empty() || workspace.is_empty() {
            return None;
        }
        Some(Self {
            api_origin: origin.trim_end_matches('/').to_string(),
            org: org.to_string(),
            workspace: workspace.to_string(),
        })
    }

    /// The state address this was parsed from, normalised.
    pub fn state_url(&self) -> String {
        format!("{}/state/{}/{}", self.api_origin, self.org, self.workspace)
    }
}

impl fmt::Display for BackendAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.state_url())
    }
}

/// Why no backend address could be resolved. Each variant is a different fix.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BackendError {
    /// A backend block exists but is not the `http` backend Terramantle serves.
    #[error("the {found} backend is configured, not the Terramantle http backend; pass --address or point the stack at Terramantle")]
    NotHttp { found: String },
    /// The `http` backend's address is not a literal (`${var.x}`) or not a
    /// Terramantle state URL, so the workspace cannot be read from the config.
    #[error("the http backend address is not a literal Terramantle state URL (got {address:?}); run `terraform init` so .terraform/terraform.tfstate holds the resolved address, or pass --address")]
    Unresolved { address: String },
    /// No backend configuration was found at all.
    #[error("no backend configuration found in {dir}: expected .terraform/terraform.tfstate (after `terraform init`) or a terraform {{ backend \"http\" {{ … }} }} block; pass --address to override")]
    NotFound { dir: String },
}

/// Resolve the backend address for `dir`: `override_address` (a `--address`
/// flag) wins; then the init-resolved config; then the HCL.
pub fn resolve_backend(
    dir: &Path,
    override_address: Option<&str>,
) -> Result<BackendAddress, BackendError> {
    if let Some(a) = override_address {
        return BackendAddress::parse(a).ok_or_else(|| BackendError::Unresolved {
            address: a.to_string(),
        });
    }
    if let Some(found) = backend_from_init_state(dir) {
        return found;
    }
    if let Some(found) = backend_from_hcl_dir(dir) {
        return found;
    }
    Err(BackendError::NotFound {
        dir: dir.display().to_string(),
    })
}

/// `.terraform/terraform.tfstate` → `{"backend":{"type":"http","config":{"address":…}}}`.
/// `None` when the file is absent or unreadable (the caller falls through to HCL).
fn backend_from_init_state(dir: &Path) -> Option<Result<BackendAddress, BackendError>> {
    let text = std::fs::read_to_string(dir.join(INIT_STATE)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let backend = v.get("backend")?;
    let ty = backend.get("type")?.as_str()?;
    if ty != "http" {
        return Some(Err(BackendError::NotHttp {
            found: ty.to_string(),
        }));
    }
    let address = backend.get("config")?.get("address")?.as_str()?;
    Some(
        BackendAddress::parse(address).ok_or_else(|| BackendError::Unresolved {
            address: address.to_string(),
        }),
    )
}

/// The backend block from the first `*.tf` in `dir` that has one. Precedence:
/// `_backend.tf`, `backend.tf`, then the rest alphabetically.
fn backend_from_hcl_dir(dir: &Path) -> Option<Result<BackendAddress, BackendError>> {
    let mut names: Vec<String> = BACKEND_FILES.iter().map(|s| s.to_string()).collect();
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut rest: Vec<String> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".tf") && !BACKEND_FILES.contains(&n.as_str()))
            .collect();
        rest.sort();
        names.extend(rest);
    }
    for name in names {
        let Ok(text) = std::fs::read_to_string(dir.join(&name)) else {
            continue;
        };
        if let Some(found) = backend_from_hcl(&text) {
            return Some(found);
        }
    }
    None
}

/// Parse `text` as HCL and read `terraform { backend "<type>" { address } }`.
/// `None` when there is no backend block (or the file is not valid HCL — a
/// malformed file must degrade to "not here", not fail the command).
pub fn backend_from_hcl(text: &str) -> Option<Result<BackendAddress, BackendError>> {
    let body: hcl::Body = hcl::from_str(text).ok()?;
    // The first `backend` block inside any `terraform` block. Terraform itself
    // allows exactly one, so "first" is "the" one.
    let be = body
        .blocks()
        .filter(|b| b.identifier() == "terraform")
        .flat_map(|tf| tf.body().blocks())
        .find(|b| b.identifier() == "backend")?;
    let ty = be
        .labels()
        .first()
        .map(|l| l.as_str().to_string())
        .unwrap_or_default();
    if ty != "http" {
        return Some(Err(BackendError::NotHttp { found: ty }));
    }
    let address = be
        .body()
        .attributes()
        .find(|a| a.key() == "address")
        .map(|a| a.expr().clone());
    Some(match address {
        Some(hcl::Expression::String(s)) => {
            BackendAddress::parse(&s).ok_or(BackendError::Unresolved { address: s })
        }
        Some(other) => Err(BackendError::Unresolved {
            address: other.to_string(),
        }),
        None => Err(BackendError::Unresolved {
            address: String::new(),
        }),
    })
}

/// The workspace segment of the `http` backend's `address`, if there is one.
///
/// Kept for `state publish` and `doctor`, which only need the key and treat
/// "unknown" as a fallback rather than an error. Reads the init-resolved config
/// first, then the HCL, as [`resolve_backend`] does.
pub fn workspace_from_backend(dir: &Path) -> Option<String> {
    resolve_backend(dir, None).ok().map(|b| b.workspace)
}

/// The workspace segment of the backend `address` in an HCL `text`. `None` for a
/// non-literal or non-Terramantle address, or no backend block.
pub fn workspace_from_text(text: &str) -> Option<String> {
    match backend_from_hcl(text)? {
        Ok(b) => Some(b.workspace),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENERATED: &str = r#"
// TERRAMATE: GENERATED AUTOMATICALLY DO NOT EDIT

terraform {
  backend "http" {
    address        = "https://registry.terramantle.dev/state/acme-demo/state-sample-network"
    lock_address   = "https://registry.terramantle.dev/state/acme-demo/state-sample-network.lock"
    unlock_address = "https://registry.terramantle.dev/state/acme-demo/state-sample-network.lock"
    username       = "terramantle-ci"
  }
}
"#;

    /// The case the directory-basename guess got wrong.
    #[test]
    fn reads_a_prefixed_workspace_name() {
        assert_eq!(
            workspace_from_text(GENERATED).as_deref(),
            Some("state-sample-network")
        );
    }

    /// `lock_address` appears first in some hand-written files; structurally keyed
    /// on `address`, so order cannot matter.
    #[test]
    fn never_matches_the_lock_addresses() {
        let text = r#"
terraform {
  backend "http" {
    lock_address   = "https://r.example/state/acme/network.lock"
    unlock_address = "https://r.example/state/acme/network.lock"
    address        = "https://r.example/state/acme/network"
  }
}
"#;
        assert_eq!(workspace_from_text(text).as_deref(), Some("network"));
    }

    /// HCL permits a one-line block only when its body is a single attribute, so
    /// the nested `backend` block needs its own line; the attribute inside it may
    /// be squashed.
    #[test]
    fn tolerates_no_spaces_and_a_trailing_slash() {
        let text =
            "terraform {\nbackend \"http\" {address=\"https://r.example/state/acme/wsp/\"}\n}\n";
        assert_eq!(workspace_from_text(text).as_deref(), Some("wsp"));
    }

    /// An unresolved interpolation is not a workspace name; the caller should
    /// fall back rather than publish to a literal `${var.x}`.
    #[test]
    fn an_interpolated_address_is_unresolved() {
        let text = "terraform {\n  backend \"http\" {\n    address = \"https://r.example/state/acme/${var.name}\"\n  }\n}\n";
        assert_eq!(workspace_from_text(text), None);
        assert!(matches!(
            backend_from_hcl(text),
            Some(Err(BackendError::Unresolved { .. }))
        ));
    }

    #[test]
    fn a_variable_reference_is_unresolved() {
        let text = "terraform {\n  backend \"http\" {\n    address = var.backend_address\n  }\n}\n";
        assert!(matches!(
            backend_from_hcl(text),
            Some(Err(BackendError::Unresolved { .. }))
        ));
    }

    #[test]
    fn commented_out_addresses_are_ignored() {
        let text = "terraform {\n  backend \"http\" {\n    # address = \"https://r.example/state/acme/old\"\n    address = \"https://r.example/state/acme/new\"\n  }\n}\n";
        assert_eq!(workspace_from_text(text).as_deref(), Some("new"));
    }

    #[test]
    fn no_backend_block_yields_nothing() {
        assert_eq!(workspace_from_text("terraform {}\n"), None);
        assert_eq!(
            workspace_from_text("resource \"null_resource\" \"x\" {}\n"),
            None
        );
    }

    #[test]
    fn a_non_http_backend_is_its_own_error() {
        let text =
            "terraform {\n  backend \"s3\" {\n    bucket = \"b\"\n    key    = \"k\"\n  }\n}\n";
        assert_eq!(
            backend_from_hcl(text),
            Some(Err(BackendError::NotHttp { found: "s3".into() }))
        );
        assert_eq!(workspace_from_text(text), None);
        let cloud = "terraform {\n  cloud {\n    organization = \"x\"\n  }\n}\n";
        assert_eq!(backend_from_hcl(cloud), None);
    }

    /// `terramate.stack.path.relative` is slash-separated, so a stack nested under
    /// `stacks/data/platform` gets a workspace of the same shape.
    #[test]
    fn reads_a_nested_workspace_path_in_full() {
        let text = r#"
terraform {
  backend "http" {
    address        = "https://registry.terramantle.dev/state/acme-demo/stacks/data/platform"
    lock_address   = "https://registry.terramantle.dev/state/acme-demo/stacks/data/platform.lock"
  }
}
"#;
        let b = backend_from_hcl(text).unwrap().unwrap();
        assert_eq!(
            b,
            BackendAddress {
                api_origin: "https://registry.terramantle.dev".into(),
                org: "acme-demo".into(),
                workspace: "stacks/data/platform".into()
            }
        );
        assert_eq!(
            b.state_url(),
            "https://registry.terramantle.dev/state/acme-demo/stacks/data/platform"
        );
    }

    #[test]
    fn parse_rejects_non_state_urls() {
        assert_eq!(BackendAddress::parse("https://r.example/api/v1/x"), None);
        assert_eq!(BackendAddress::parse("https://r.example/state/acme"), None);
        assert_eq!(BackendAddress::parse("ftp://r.example/state/acme/ws"), None);
        assert_eq!(
            BackendAddress::parse("https://r.example/state/acme/ws?x=1#f")
                .unwrap()
                .workspace,
            "ws"
        );
    }

    #[test]
    fn init_state_wins_over_hcl_and_carries_the_resolved_address() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("backend.tf"),
            "terraform {\n  backend \"http\" {\n    address = var.addr\n  }\n}\n",
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join(".terraform")).unwrap();
        std::fs::write(
            tmp.path().join(INIT_STATE),
            r#"{"version":3,"backend":{"type":"http","config":{"address":"https://r.example/state/acme/from-init","lock_address":"https://r.example/state/acme/from-init.lock"}}}"#,
        )
        .unwrap();
        assert_eq!(
            resolve_backend(tmp.path(), None).unwrap().workspace,
            "from-init"
        );
    }

    #[test]
    fn init_state_with_another_backend_is_not_http() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".terraform")).unwrap();
        std::fs::write(
            tmp.path().join(INIT_STATE),
            r#"{"backend":{"type":"s3","config":{"bucket":"b"}}}"#,
        )
        .unwrap();
        assert_eq!(
            resolve_backend(tmp.path(), None),
            Err(BackendError::NotHttp { found: "s3".into() })
        );
    }

    #[test]
    fn generated_backend_wins_over_a_stale_handwritten_one() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("backend.tf"),
            "terraform {\n  backend \"http\" {\n    address = \"https://r/state/o/stale\"\n  }\n}\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("_backend.tf"), GENERATED).unwrap();
        assert_eq!(
            workspace_from_backend(tmp.path()).as_deref(),
            Some("state-sample-network")
        );
    }

    #[test]
    fn any_tf_file_is_searched_after_the_backend_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("main.tf"),
            "terraform {\n  backend \"http\" {\n    address = \"https://r.example/state/o/in-main\"\n  }\n}\n",
        )
        .unwrap();
        assert_eq!(
            workspace_from_backend(tmp.path()).as_deref(),
            Some("in-main")
        );
    }

    #[test]
    fn an_override_address_wins_and_must_parse() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_backend(tmp.path(), Some("https://r.example/state/o/w"))
                .unwrap()
                .workspace,
            "w"
        );
        assert!(matches!(
            resolve_backend(tmp.path(), Some("nonsense")),
            Err(BackendError::Unresolved { .. })
        ));
    }

    #[test]
    fn missing_files_are_not_an_error_for_the_workspace_helper() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(workspace_from_backend(tmp.path()), None);
        assert!(matches!(
            resolve_backend(tmp.path(), None),
            Err(BackendError::NotFound { .. })
        ));
    }
}
