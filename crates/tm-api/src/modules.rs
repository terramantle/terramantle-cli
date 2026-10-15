//! Module registry endpoints (§7 `modules search` / `modules show`).

use crate::error::ApiError;
use crate::generated::models::{
    ModuleSearchResponse, ModuleVersionDetail, ModuleVersionsResponse, PublishModuleVersionResponse,
};
use crate::ops;
use crate::Client;

impl Client {
    /// `GET /v1/modules/search?q=&limit=&offset=` — paginated registry search.
    /// The response paginates via `meta.next_offset`.
    pub fn modules_search(
        &self,
        q: &str,
        limit: u64,
        offset: u64,
    ) -> Result<ModuleSearchResponse, ApiError> {
        let path = ops::SEARCH_MODULES.path(&[])?;
        let query = [
            ("q", q.to_string()),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
        ];
        self.http().get_json_query(&path, &query)
    }

    /// `GET /v1/modules/{ns}/{name}/{provider}/versions` — the
    /// Terraform-protocol versions list for a module.
    pub fn module_versions(
        &self,
        namespace: &str,
        name: &str,
        provider: &str,
    ) -> Result<ModuleVersionsResponse, ApiError> {
        let path = ops::LIST_MODULE_VERSIONS.path(&[namespace, name, provider])?;
        self.http().get_json(&path)
    }

    /// `GET /v1/modules/{ns}/{name}/{provider}/{version}` — one version's full
    /// detail.
    ///
    /// This replaces a `module_show` that requested
    /// `/v1/modules/{ns}/{name}/{provider}` with no version — a path the worker
    /// does not route. It answered `404` while its siblings answered `401`,
    /// which is how a route that never existed stayed invisible: it looked like
    /// an ordinary not-found.
    ///
    /// Detail is per VERSION, so a caller holding only `ns/name/provider` picks
    /// one first — see `modules show`, which resolves the newest.
    pub fn module_version(
        &self,
        namespace: &str,
        name: &str,
        provider: &str,
        version: &str,
    ) -> Result<ModuleVersionDetail, ApiError> {
        let path = ops::GET_MODULE_VERSION.path(&[namespace, name, provider, version])?;
        self.http().get_json(&path)
    }

    /// `PUT /v1/modules/{namespace}/{name}/{provider}/{version}` — publish a
    /// module version (§6 step 5). `namespace` is the org slug and the raw
    /// `tarball` is the request body.
    ///
    /// The response is a genuine union: the server answers `ModulePublishSettled`
    /// when the scan finished inside the request, and `ModulePublishPending` when
    /// it did not. Callers must handle both — which is new. The old model made
    /// every field optional and could not tell the two apart, so a publish that
    /// was still scanning looked like a settled one with nothing to report.
    pub fn module_publish(
        &self,
        namespace: &str,
        name: &str,
        provider: &str,
        version: &str,
        tarball: &[u8],
        description: Option<&str>,
    ) -> Result<PublishModuleVersionResponse, ApiError> {
        let mut path = ops::PUBLISH_MODULE_VERSION.path(&[namespace, name, provider, version])?;
        if let Some(desc) = description {
            path.push_str(&format!("?description={}", encode_query(desc)));
        }
        self.http()
            .put_bytes(&path, tarball, &[("Content-Type", "application/gzip")])
    }
}

/// Minimal percent-encoding for a query-string VALUE (RFC 3986 unreserved set
/// passes through; everything else is `%XX`).
///
/// Path segments are encoded by `Op::path`; this is only for the one query
/// parameter the publish call appends, and it is kept local so `tm-api` pulls in
/// no url crate for it.
fn encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_query_escapes_reserved_and_keeps_unreserved() {
        assert_eq!(encode_query("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(encode_query("A-z_0.9~"), "A-z_0.9~");
    }

    /// Bug 6, as a path assertion. `module_show` used to request
    /// `/v1/modules/{ns}/{name}/{provider}` — a path with no version, which the
    /// worker does not route. Detail needs the version.
    #[test]
    fn detail_path_includes_the_version() {
        assert_eq!(
            ops::GET_MODULE_VERSION
                .path(&["acme", "vpc", "aws", "1.2.3"])
                .unwrap(),
            "/v1/modules/acme/vpc/aws/1.2.3"
        );
    }

    #[test]
    fn versions_path_is_the_protocol_list() {
        assert_eq!(
            ops::LIST_MODULE_VERSIONS
                .path(&["acme", "vpc", "aws"])
                .unwrap(),
            "/v1/modules/acme/vpc/aws/versions"
        );
    }

    /// Path segments are encoded by `Op::path`, so a module name containing a
    /// slash cannot escape its segment and address a different route.
    #[test]
    fn path_segments_are_encoded() {
        assert_eq!(
            ops::LIST_MODULE_VERSIONS
                .path(&["acme", "../admin", "aws"])
                .unwrap(),
            "/v1/modules/acme/..%2Fadmin/aws/versions"
        );
    }
}
