//! Org endpoints (§4.1).

use crate::error::ApiError;
use crate::generated::models::{ListOrgsResponse, OrgSummary};
use crate::ops;
use crate::Client;

impl Client {
    /// `GET /api/orgs` → the caller's org memberships (human tokens only; CI
    /// OIDC/bot tokens have no org endpoint — see §4.1). Consolidated here so
    /// `tm-auth` depends on `tm-api` rather than re-declaring the type.
    /// Unwraps the `{ orgs }` envelope.
    pub fn orgs_list(&self) -> Result<Vec<OrgSummary>, ApiError> {
        let path = ops::LIST_ORGS.path(&[])?;
        let resp: ListOrgsResponse = self.http().get_json(&path)?;
        Ok(resp.orgs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::models::Role;

    /// The handler wraps rows as `{ orgs: [...] }` and sends extra columns
    /// (`id`, `name`, `tier`, …) we must tolerate — deserializing this as a bare
    /// sequence is the "invalid type: map, expected a sequence" bug.
    #[test]
    fn org_list_unwraps_envelope_and_ignores_extra_columns() {
        let body = r#"{"orgs":[
            {"id":"t1","name":"Acme","slug":"acme","is_personal":0,"is_demo":0,"claimed_domain":null,"tier":"team","role":"owner","created_at":1},
            {"id":"t2","name":"Demo","slug":"acme-demo","is_personal":0,"is_demo":1,"claimed_domain":null,"tier":"starter","role":"member","created_at":2}
        ]}"#;
        let resp: ListOrgsResponse = serde_json::from_str(body).unwrap();
        let slugs: Vec<&str> = resp.orgs.iter().map(|o| o.slug.as_str()).collect();
        assert_eq!(slugs, vec!["acme", "acme-demo"]);
        // `role` is the spec's enum now, not free text, so a typo'd role from the
        // server is a parse error here rather than a string nothing matches.
        assert_eq!(resp.orgs[0].role, Role::Owner);
    }

    #[test]
    fn org_list_tolerates_missing_key() {
        let resp: ListOrgsResponse = serde_json::from_str("{}").unwrap();
        assert!(resp.orgs.is_empty());
    }

    /// The path has no parameters, so it is a constant — but it still comes from
    /// the spec rather than a literal.
    #[test]
    fn path_comes_from_the_spec() {
        assert_eq!(ops::LIST_ORGS.path(&[]).unwrap(), "/api/orgs");
    }
}
