//! Module registry endpoints (§7 `modules search` / `modules show`).

use crate::error::ApiError;
use crate::models::{ModuleDetail, ModuleSearchResponse, ModuleVersionsResponse};
use crate::Client;

impl Client {
    /// `GET /v1/modules/search?q=&limit=&offset=` — paginated registry search.
    /// The response paginates via `meta.next_offset` (no `total` in the wire
    /// shape — see [`ModuleSearchResponse`]).
    pub fn modules_search(
        &self,
        q: &str,
        limit: u64,
        offset: u64,
    ) -> Result<ModuleSearchResponse, ApiError> {
        let query = [
            ("q", q.to_string()),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
        ];
        self.http().get_json_query("/v1/modules/search", &query)
    }

    /// `GET /v1/modules/{ns}/{name}/{provider}` — full module version detail.
    pub fn module_show(
        &self,
        namespace: &str,
        name: &str,
        provider: &str,
    ) -> Result<ModuleDetail, ApiError> {
        let path = format!("/v1/modules/{namespace}/{name}/{provider}");
        self.http().get_json(&path)
    }

    /// `GET /v1/modules/{ns}/{name}/{provider}/versions` — the Terraform-protocol
    /// versions list for a module.
    pub fn module_versions(
        &self,
        namespace: &str,
        name: &str,
        provider: &str,
    ) -> Result<ModuleVersionsResponse, ApiError> {
        let path = format!("/v1/modules/{namespace}/{name}/{provider}/versions");
        self.http().get_json(&path)
    }
}
