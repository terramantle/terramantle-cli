//! Reading the state workspace back out of a stack's backend configuration.
//!
//! `state publish` has to know which Terramantle workspace a directory's state
//! lives in. It used to assume the directory basename, which is true only when
//! nothing customised the backend address — and silently wrong otherwise:
//! `state-sample-mono-repo` names its workspaces `state-sample-<stack>`, so the
//! provider lock file was uploaded to `network` while the state sat in
//! `state-sample-network`. Neither call errors; the two just drift apart.
//!
//! The backend block is authoritative, because it is literally the URL the
//! state is pushed to. Parsing it removes the guess.

use std::path::Path;

/// Backend filenames, in precedence order. `_backend.tf` is what `terramate
//  generate` emits, and it wins: in a Terramate repo a hand-written
/// `backend.tf` is the stale one.
const BACKEND_FILES: &[&str] = &["_backend.tf", "backend.tf"];

/// The workspace segment of the `http` backend's `address`, if there is one.
///
/// A text scan rather than an HCL parse: these files are machine-generated and
/// single-purpose, and a malformed one must degrade to "unknown" rather than
/// failing the command that called us.
pub fn workspace_from_backend(dir: &Path) -> Option<String> {
    for name in BACKEND_FILES {
        let Ok(text) = std::fs::read_to_string(dir.join(name)) else {
            continue;
        };
        if let Some(ws) = workspace_from_text(&text) {
            return Some(ws);
        }
    }
    None
}

/// The workspace segment of the first `address = "…"` assignment in `text`.
///
/// Keyed on the bare `address` attribute: `lock_address` and `unlock_address`
/// carry the same workspace with a `.lock` suffix, and matching one of those
/// by accident would publish to a workspace named `network.lock`.
pub fn workspace_from_text(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "address" {
            continue;
        }
        let url = value.trim().trim_matches('"');
        let segment = url.trim_end_matches('/').rsplit('/').next()?;
        // A Terraform variable or interpolation is not a literal workspace.
        if segment.is_empty() || segment.contains("${") {
            return None;
        }
        return Some(segment.to_string());
    }
    None
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

    /// `lock_address` appears first in some hand-written files; matching it
    /// would yield `network.lock`.
    #[test]
    fn never_matches_the_lock_addresses() {
        let text = r#"
    lock_address   = "https://r.example/state/acme/network.lock"
    unlock_address = "https://r.example/state/acme/network.lock"
    address        = "https://r.example/state/acme/network"
"#;
        assert_eq!(workspace_from_text(text).as_deref(), Some("network"));
    }

    #[test]
    fn tolerates_no_spaces_and_a_trailing_slash() {
        assert_eq!(
            workspace_from_text(r#"address="https://r.example/state/acme/wsp/""#).as_deref(),
            Some("wsp")
        );
    }

    /// An unresolved interpolation is not a workspace name; the caller should
    /// fall back rather than publish to a literal `${var.x}`.
    #[test]
    fn an_interpolated_address_yields_nothing() {
        assert_eq!(
            workspace_from_text(r#"  address = "https://r.example/state/acme/${var.name}""#),
            None
        );
    }

    #[test]
    fn commented_out_addresses_are_ignored() {
        let text = "# address = \"https://r.example/state/acme/old\"\n\
                    address = \"https://r.example/state/acme/new\"\n";
        assert_eq!(workspace_from_text(text).as_deref(), Some("new"));
    }

    #[test]
    fn no_backend_block_yields_nothing() {
        assert_eq!(workspace_from_text("terraform {}\n"), None);
    }

    #[test]
    fn generated_backend_wins_over_a_stale_handwritten_one() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("backend.tf"),
            "address = \"https://r/state/o/stale\"",
        )
        .unwrap();
        std::fs::write(tmp.path().join("_backend.tf"), GENERATED).unwrap();
        assert_eq!(
            workspace_from_backend(tmp.path()).as_deref(),
            Some("state-sample-network")
        );
    }

    #[test]
    fn missing_files_are_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(workspace_from_backend(tmp.path()), None);
    }
}
