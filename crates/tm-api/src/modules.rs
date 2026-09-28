//! Module registry endpoints (§7 `modules search` / `modules show`).

use crate::error::ApiError;
use crate::models::{
    ModuleDetail, ModulePublishResponse, ModuleSearchResponse, ModuleVersionsResponse,
};
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

    /// `PUT /v1/modules/{namespace}/{name}/{provider}/{version}` — publish a
    /// module version (§6 step 5). `namespace` is the org slug; the raw `tarball`
    /// is the request body. An optional `description` is passed as a url-encoded
    /// `?description=` query param. The server returns 201 on success.
    pub fn module_publish(
        &self,
        namespace: &str,
        name: &str,
        provider: &str,
        version: &str,
        tarball: &[u8],
        description: Option<&str>,
    ) -> Result<ModulePublishResponse, ApiError> {
        let mut path = format!("/v1/modules/{namespace}/{name}/{provider}/{version}");
        if let Some(desc) = description {
            path.push_str(&format!("?description={}", encode_query(desc)));
        }
        self.http()
            .put_bytes(&path, tarball, &[("Content-Type", "application/gzip")])
    }
}

/// Minimal percent-encoding for a query-string value (RFC 3986 unreserved set
/// passes through; everything else is `%XX`). Kept local so `tm-api` pulls in no
/// url crate for the single publish query param.
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
    use super::encode_query;

    #[test]
    fn encode_query_escapes_reserved_and_keeps_unreserved() {
        assert_eq!(encode_query("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(encode_query("A-z_0.9~"), "A-z_0.9~");
    }
}
