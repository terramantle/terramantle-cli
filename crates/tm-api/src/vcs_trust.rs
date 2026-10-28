//! VCS trust endpoints (`/api/orgs/{orgId}/vcs-trust`).
//!
//! A trust entry is what lets the registry accept a GitHub/GitLab OIDC token
//! from a given repo: the stored `subject` is matched against the CI token's
//! `sub` claim. Both calls are org-admin only, so a non-admin sees 403 — the
//! caller is expected to treat that as "cannot do it for you", not a failure.

use crate::error::ApiError;
use crate::generated::models::{ListVcsTrustResponse, VcsTrustEntry};
use crate::ops;
use crate::Client;

/// One trust row, flattened to the fields a caller actually compares on. The
/// generated model has every field optional (the spec marks nothing required),
/// which makes it awkward to match against; rows missing either field cannot
/// be matched meaningfully, so they are dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustEntry {
    pub provider: String,
    pub subject: String,
}

impl Client {
    /// `GET /api/orgs/{orgId}/vcs-trust` — the org's trust entries. Admin only.
    pub fn vcs_trust_list(&self, org: &str) -> Result<Vec<TrustEntry>, ApiError> {
        let path = ops::LIST_VCS_TRUST.path(&[org])?;
        let resp: ListVcsTrustResponse = self.http().get_json(&path)?;
        Ok(resp.entries.into_iter().filter_map(flatten).collect())
    }

    /// `POST /api/orgs/{orgId}/vcs-trust` — register a subject. Admin only.
    ///
    /// The server validates the subject format and rejects an overlap with
    /// another org, so a 400/409 here is a real answer rather than a transport
    /// failure; both surface as [`ApiError`] with the server's message.
    pub fn vcs_trust_create(
        &self,
        org: &str,
        provider: &str,
        subject: &str,
    ) -> Result<(), ApiError> {
        let path = ops::CREATE_VCS_TRUST.path(&[org])?;
        let body = serde_json::json!({ "provider": provider, "subject": subject });
        // The handler answers with the created row; nothing here needs it.
        let _: serde_json::Value = self.http().post_json(&path, &body)?;
        Ok(())
    }
}

fn flatten(e: VcsTrustEntry) -> Option<TrustEntry> {
    Some(TrustEntry {
        provider: e.provider.map(|p| p.to_string())?,
        subject: e.subject?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paths come from the spec, never a `format!`.
    #[test]
    fn paths_come_from_the_spec() {
        assert_eq!(
            ops::LIST_VCS_TRUST.path(&["acme"]).unwrap(),
            "/api/orgs/acme/vcs-trust"
        );
        assert_eq!(
            ops::CREATE_VCS_TRUST.path(&["acme"]).unwrap(),
            "/api/orgs/acme/vcs-trust"
        );
    }

    #[test]
    fn list_unwraps_the_envelope_and_drops_unmatched_rows() {
        let body = r#"{"entries":[
            {"id":"1","provider":"github","subject":"repo:acme/one","created_at":1},
            {"id":"2","subject":"repo:acme/no-provider"}
        ]}"#;
        let resp: ListVcsTrustResponse = serde_json::from_str(body).unwrap();
        let rows: Vec<TrustEntry> = resp.entries.into_iter().filter_map(flatten).collect();
        assert_eq!(
            rows,
            vec![TrustEntry {
                provider: "github".into(),
                subject: "repo:acme/one".into()
            }]
        );
    }

    #[test]
    fn list_tolerates_a_missing_key() {
        let resp: ListVcsTrustResponse = serde_json::from_str("{}").unwrap();
        assert!(resp.entries.is_empty());
    }
}
