//! Org endpoints (§4.1).

use crate::error::ApiError;
use crate::models::OrgMembership;
use crate::Client;

impl Client {
    /// `GET /api/orgs` → the caller's org memberships (human tokens only; CI
    /// OIDC/bot tokens have no org endpoint — see §4.1). Consolidated here so
    /// `tm-auth` depends on `tm-api` rather than re-declaring the type.
    pub fn orgs_list(&self) -> Result<Vec<OrgMembership>, ApiError> {
        self.http().get_json("/api/orgs")
    }
}
