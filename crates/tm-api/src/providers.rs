//! Provider endpoints (§7 `providers ls` / `providers show`).

use crate::error::ApiError;
use crate::models::{
    ProviderOverview, ProviderUsage, ProviderVersionsResponse, ProvidersOverviewResponse, UsedBy,
};
use crate::Client;

impl Client {
    /// `GET /api/orgs/{org}/provider-foundry/providers-usage` — usage rollup of
    /// the org's in-use providers grouped by `(namespace, type)`.
    pub fn providers_usage(&self, org: &str) -> Result<Vec<ProviderUsage>, ApiError> {
        let path = format!("/api/orgs/{org}/provider-foundry/providers-usage");
        self.http().get_json(&path)
    }

    /// `GET .../providers-usage/{ns}/{type}` — one row per workspace-usage of a
    /// specific provider.
    pub fn providers_usage_detail(
        &self,
        org: &str,
        namespace: &str,
        type_: &str,
    ) -> Result<Vec<UsedBy>, ApiError> {
        let path = format!("/api/orgs/{org}/provider-foundry/providers-usage/{namespace}/{type_}");
        self.http().get_json(&path)
    }

    /// `GET .../provider-foundry/providers-overview` — provider-level severity
    /// rollup for the TRUST column. The handler wraps rows as `{ providers }`;
    /// this unwraps to the bare list.
    pub fn providers_overview(&self, org: &str) -> Result<Vec<ProviderOverview>, ApiError> {
        let path = format!("/api/orgs/{org}/provider-foundry/providers-overview");
        let resp: ProvidersOverviewResponse = self.http().get_json(&path)?;
        Ok(resp.providers)
    }

    /// `GET /v1/providers/{ns}/{type}/versions` — available versions. `authRequired`
    /// and org-allowlist-filtered server-side (§7 S4), so the client must be bearer-
    /// authed; the org is resolved from the token, not the path.
    pub fn provider_versions(
        &self,
        namespace: &str,
        type_: &str,
    ) -> Result<ProviderVersionsResponse, ApiError> {
        let path = format!("/v1/providers/{namespace}/{type_}/versions");
        self.http().get_json(&path)
    }
}
