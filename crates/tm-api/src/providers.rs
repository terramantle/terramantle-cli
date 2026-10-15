//! Provider endpoints (§7 `providers ls` / `providers show`).
//!
//! Every endpoint this module used to call is gone. `providers-usage`,
//! `providers-usage/{ns}/{type}` and `providers-overview` were removed with the
//! mirror/foundry split and answer `410`; the token-scoped provider version list
//! still works but the spec deprecates it, because for a user in several orgs it
//! is ambiguous and the ambiguity surfaces later as a confusing `tofu`
//! credentials error.
//!
//! The replacements are fewer calls, not more: the unified catalog list carries
//! the usage rollup AND the severity counts that previously took a second
//! request, and the artefact detail carries per-version findings and the used-by
//! rows that previously took a third.

use crate::error::ApiError;
use crate::generated::models::{
    ListOrgProvidersResponse, ProviderArtefactDetail, ProviderArtefactSummary, ProviderUpstream,
    ProviderVersionsResponse,
};
use crate::ops;
use crate::Client;

impl Client {
    /// `GET /api/orgs/{orgId}/providers` — the org's managed provider catalog.
    ///
    /// One row per provider (not per version), carrying serve mode, constraint,
    /// blast radius, download windows, build/patch counts and the severity
    /// rollup for `latest_version`.
    ///
    /// Replaces the `providers-usage` + `providers-overview` pair: one request
    /// where there were two.
    ///
    /// Every enrichment is fault-isolated server-side — a failing usage,
    /// references or downloads query degrades that column to `0`/`null` rather
    /// than failing the list. So a zero here can mean "none" or "the enrichment
    /// was unavailable", and this is not the place to assert an absence from.
    pub fn providers_list(&self, org: &str) -> Result<Vec<ProviderArtefactSummary>, ApiError> {
        let path = ops::LIST_ORG_PROVIDERS.path(&[org])?;
        let resp: ListOrgProvidersResponse = self.http().get_json(&path)?;
        Ok(resp.providers)
    }

    /// `GET /api/orgs/{orgId}/providers/{upstream}/{namespace}/{type}` — one
    /// managed provider with its versions.
    ///
    /// Note the `{upstream}` segment, which the removed route did not have: one
    /// org can manage both the `terraform.io` and `opentofu.org` lineages of the
    /// same `namespace/type`, so all three are needed to name an artefact.
    /// Callers holding only `namespace/type` resolve `upstream` from
    /// [`Client::providers_list`] — see `providers show`.
    pub fn provider_detail(
        &self,
        org: &str,
        upstream: ProviderUpstream,
        namespace: &str,
        type_: &str,
    ) -> Result<ProviderArtefactDetail, ApiError> {
        let upstream = upstream.to_string();
        let path = ops::GET_ORG_PROVIDER.path(&[org, &upstream, namespace, type_])?;
        self.http().get_json(&path)
    }

    /// `GET /v1/providers/{orgSlug}/{namespace}/{type}/versions` — the registry
    /// protocol's version list, narrowed to what this org serves.
    ///
    /// Org-scoped deliberately. The token-scoped form is spec-deprecated and
    /// generated with `#[deprecated]`, so under this repo's `-D warnings` it
    /// cannot be called by accident.
    ///
    /// The server's refusals are ordered, and the order is the contract:
    /// `serve_mode: block` ⇒ `404` (a blocked provider must not advertise that
    /// it exists for this org), no serve rule ⇒ `403` naming the provider.
    pub fn provider_versions(
        &self,
        org: &str,
        namespace: &str,
        type_: &str,
    ) -> Result<ProviderVersionsResponse, ApiError> {
        let path = ops::LIST_ORG_PROVIDER_VERSIONS.path(&[org, namespace, type_])?;
        self.http().get_json(&path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reported bug, as a path assertion. The old call built
    /// `/api/orgs/{org}/provider-foundry/providers-usage`, which answers 410.
    #[test]
    fn list_targets_the_unified_catalog() {
        assert_eq!(
            ops::LIST_ORG_PROVIDERS.path(&["acme-demo"]).unwrap(),
            "/api/orgs/acme-demo/providers"
        );
    }

    /// The detail route takes `{upstream}` BEFORE `{namespace}/{type}`; getting
    /// that order wrong would silently address a different artefact.
    #[test]
    fn detail_path_puts_upstream_first() {
        assert_eq!(
            ops::GET_ORG_PROVIDER
                .path(&["acme", "terraform.io", "hashicorp", "aws"])
                .unwrap(),
            "/api/orgs/acme/providers/terraform.io/hashicorp/aws"
        );
    }

    /// `ProviderUpstream` renders as the wire value, so it can be substituted
    /// into a path directly.
    #[test]
    fn upstream_enum_renders_as_the_path_segment() {
        assert_eq!(ProviderUpstream::TerraformIo.to_string(), "terraform.io");
        assert_eq!(ProviderUpstream::OpentofuOrg.to_string(), "opentofu.org");
        assert_eq!(ProviderUpstream::SelfBuilt.to_string(), "self_built");
    }

    /// The version list is org-scoped: the org slug is the FIRST segment, not
    /// resolved from the token.
    #[test]
    fn version_list_is_org_scoped() {
        assert_eq!(
            ops::LIST_ORG_PROVIDER_VERSIONS
                .path(&["acme", "hashicorp", "aws"])
                .unwrap(),
            "/v1/providers/acme/hashicorp/aws/versions"
        );
    }
}
