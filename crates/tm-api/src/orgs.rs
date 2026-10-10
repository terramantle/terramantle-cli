//! Org endpoints (§4.1).

use crate::error::ApiError;
use crate::models::{OrgListResponse, OrgMembership};
use crate::Client;

impl Client {
    /// `GET /api/orgs` → the caller's org memberships (human tokens only; CI
    /// OIDC/bot tokens have no org endpoint — see §4.1). Consolidated here so
    /// `tm-auth` depends on `tm-api` rather than re-declaring the type.
    /// Unwraps the `{ orgs }` envelope.
    pub fn orgs_list(&self) -> Result<Vec<OrgMembership>, ApiError> {
        let resp: OrgListResponse = self.http().get_json("/api/orgs")?;
        Ok(resp.orgs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The handler wraps rows as `{ orgs: [...] }` and sends extra columns
    /// (`id`, `name`, `tier`, …) we must tolerate — deserializing this as a bare
    /// sequence is the "invalid type: map, expected a sequence" bug.
    #[test]
    fn org_list_unwraps_envelope_and_ignores_extra_columns() {
        let body = r#"{"orgs":[
            {"id":"t1","name":"Acme","slug":"acme","is_personal":0,"tier":"team","role":"owner","created_at":1},
            {"id":"t2","name":"Demo","slug":"acme-demo","is_personal":0,"tier":"starter","role":"member","created_at":2}
        ]}"#;
        let resp: OrgListResponse = serde_json::from_str(body).unwrap();
        let slugs: Vec<&str> = resp.orgs.iter().map(|o| o.slug.as_str()).collect();
        assert_eq!(slugs, vec!["acme", "acme-demo"]);
        assert_eq!(resp.orgs[0].role, "owner");
    }

    #[test]
    fn org_list_tolerates_missing_key() {
        let resp: OrgListResponse = serde_json::from_str("{}").unwrap();
        assert!(resp.orgs.is_empty());
    }
}
