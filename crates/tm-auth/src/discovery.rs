//! Bootstrap discovery (SPEC §5): `GET {api_url}/.well-known/terramantle-cli.json`
//! learns the OIDC config so nothing is hardcoded in the binary.
//!
//! `TERRAMANTLE_OIDC_ISSUER` / `TERRAMANTLE_AUDIENCE` still override the
//! discovered values. Fetched once and cached in-process.

use std::sync::OnceLock;

use serde::Deserialize;
use tm_api::HttpClient;

use crate::AuthError;

/// The OIDC block of the discovery document.
#[derive(Debug, Clone, Deserialize)]
pub struct OidcConfig {
    pub issuer: String,
    pub discovery_url: String,
    pub audience: String,
    #[serde(default)]
    pub vcs_audience: Option<String>,
    /// Null until the public device-flow client is provisioned in the IdP.
    #[serde(default)]
    pub device_client_id: Option<String>,
}

/// The subset of RFC 8414 provider metadata the token flows need, fetched from
/// the issuer's `.well-known/openid-configuration` (the Terramantle discovery
/// doc points at it via `discovery_url`). These are absolute URLs, so the flows
/// never hardcode a provider-specific path — correct for Authentik, Zitadel, or
/// any compliant IdP.
#[derive(Debug, Clone, Deserialize)]
pub struct OidcEndpoints {
    pub token_endpoint: String,
    #[serde(default)]
    pub device_authorization_endpoint: Option<String>,
}

/// The `.well-known/terramantle-cli.json` document.
#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    pub api_url: String,
    pub oidc: OidcConfig,
}

impl Discovery {
    /// The effective issuer, honouring the `TERRAMANTLE_OIDC_ISSUER` override.
    pub fn issuer<'a>(&'a self, override_issuer: Option<&'a str>) -> &'a str {
        override_issuer.unwrap_or(&self.oidc.issuer)
    }

    /// The effective audience, honouring the `TERRAMANTLE_AUDIENCE` override.
    pub fn audience<'a>(&'a self, override_audience: Option<&'a str>) -> &'a str {
        override_audience.unwrap_or(&self.oidc.audience)
    }
}

/// Process-wide discovery cache, keyed implicitly on the single api_url used per
/// process. Fetched once (§5: "Fetch once, cache in memory for the process").
static CACHE: OnceLock<Discovery> = OnceLock::new();

/// Fetch (or return the cached) discovery document for `api_url`.
pub fn fetch(api_url: &str) -> Result<&'static Discovery, AuthError> {
    if let Some(d) = CACHE.get() {
        return Ok(d);
    }
    let client = HttpClient::new(api_url);
    let doc: Discovery = client
        .get_json("/.well-known/terramantle-cli.json")
        .map_err(AuthError::Discovery)?;
    // Race is benign: whichever thread wins, both hold identical data.
    let _ = CACHE.set(doc);
    Ok(CACHE.get().expect("cache just set"))
}

/// Process-wide cache for the issuer's RFC 8414 metadata (one issuer per process).
static OIDC_CACHE: OnceLock<OidcEndpoints> = OnceLock::new();

/// Fetch (or return the cached) OIDC provider metadata from `discovery_url` (an
/// absolute `.well-known/openid-configuration` URL). Supplies the absolute
/// token/device endpoints the flows post to.
pub fn fetch_oidc_endpoints(discovery_url: &str) -> Result<&'static OidcEndpoints, AuthError> {
    if let Some(e) = OIDC_CACHE.get() {
        return Ok(e);
    }
    let doc: OidcEndpoints = HttpClient::new("")
        .get_json(discovery_url)
        .map_err(AuthError::Discovery)?;
    let _ = OIDC_CACHE.set(doc);
    Ok(OIDC_CACHE.get().expect("cache just set"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_discovery_with_null_device_client() {
        let json = r#"{
            "api_url": "https://registry.terramantle.dev",
            "oidc": {
                "issuer": "https://zitadel.example",
                "discovery_url": "https://zitadel.example/.well-known/openid-configuration",
                "audience": "https://registry.terramantle.dev",
                "vcs_audience": "https://registry.terramantle.dev",
                "device_client_id": null
            }
        }"#;
        let d: Discovery = serde_json::from_str(json).unwrap();
        assert_eq!(d.api_url, "https://registry.terramantle.dev");
        assert_eq!(d.oidc.issuer, "https://zitadel.example");
        assert_eq!(d.oidc.audience, "https://registry.terramantle.dev");
        assert_eq!(d.oidc.device_client_id, None);
    }

    #[test]
    fn deserializes_discovery_with_device_client() {
        let json = r#"{
            "api_url": "https://reg",
            "oidc": {
                "issuer": "https://iss",
                "discovery_url": "https://iss/.well-known/openid-configuration",
                "audience": "https://reg",
                "device_client_id": "cli-public-123"
            }
        }"#;
        let d: Discovery = serde_json::from_str(json).unwrap();
        assert_eq!(d.oidc.device_client_id.as_deref(), Some("cli-public-123"));
        assert_eq!(d.oidc.vcs_audience, None);
    }

    #[test]
    fn deserializes_oidc_endpoints_from_well_known() {
        // Shape of Authentik's .well-known/openid-configuration (extra fields ignored).
        let json = r#"{
            "issuer": "https://sso.terramantle.dev/application/o/terramantle-app/",
            "authorization_endpoint": "https://sso.terramantle.dev/application/o/authorize/",
            "token_endpoint": "https://sso.terramantle.dev/application/o/token/",
            "device_authorization_endpoint": "https://sso.terramantle.dev/application/o/device/"
        }"#;
        let e: OidcEndpoints = serde_json::from_str(json).unwrap();
        assert_eq!(
            e.token_endpoint,
            "https://sso.terramantle.dev/application/o/token/"
        );
        assert_eq!(
            e.device_authorization_endpoint.as_deref(),
            Some("https://sso.terramantle.dev/application/o/device/")
        );
    }

    #[test]
    fn oidc_endpoints_without_device_support() {
        let json = r#"{ "token_endpoint": "https://iss/token" }"#;
        let e: OidcEndpoints = serde_json::from_str(json).unwrap();
        assert_eq!(e.token_endpoint, "https://iss/token");
        assert_eq!(e.device_authorization_endpoint, None);
    }

    #[test]
    fn overrides_win_over_discovered() {
        let json = r#"{
            "api_url": "https://reg",
            "oidc": {
                "issuer": "https://iss",
                "discovery_url": "https://iss/x",
                "audience": "https://reg",
                "device_client_id": null
            }
        }"#;
        let d: Discovery = serde_json::from_str(json).unwrap();
        assert_eq!(d.issuer(Some("https://override")), "https://override");
        assert_eq!(d.issuer(None), "https://iss");
        assert_eq!(d.audience(Some("aud-override")), "aud-override");
        assert_eq!(d.audience(None), "https://reg");
    }
}
