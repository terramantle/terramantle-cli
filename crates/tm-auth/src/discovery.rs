//! Bootstrap discovery (SPEC §5): `GET {api_url}/.well-known/terramantle-cli.json`
//! learns the OIDC config so nothing is hardcoded in the binary.
//!
//! Everything provider-shaped comes from there: the issuer, the issuer's RFC 8414
//! `.well-known/openid-configuration` URL (which supplies the token/device
//! endpoints), the audience, and the scopes to request. The CLI never assumes a
//! vendor-specific path or parameter — any OIDC-compliant IdP works.
//!
//! `TERRAMANTLE_OIDC_ISSUER` / `TERRAMANTLE_AUDIENCE` still override the
//! discovered values. Fetched once per URL and cached in-process.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;
use tm_api::HttpClient;

use crate::AuthError;

/// Scopes requested when the discovery doc doesn't specify any: the standard
/// OIDC set plus `offline_access` for a refresh token. Deployments that mint
/// the API audience via a custom scope advertise it in `oidc.scopes`.
pub const DEFAULT_SCOPES: &str = "openid profile email offline_access";

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
    /// Space-separated scopes the CLI should request (device + client-credentials
    /// flows). Absent → [`DEFAULT_SCOPES`].
    #[serde(default)]
    pub scopes: Option<String>,
}

/// The subset of RFC 8414 provider metadata the token flows need, fetched from
/// the issuer's `.well-known/openid-configuration` (the Terramantle discovery
/// doc points at it via `discovery_url`). These are absolute URLs, so the flows
/// never hardcode a provider-specific path — correct for any compliant IdP.
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

    /// The audience CI OIDC tokens are minted for (GitHub `audience=` request
    /// param / GitLab `id_tokens.aud`). Falls back to the main audience when the
    /// deployment doesn't distinguish the two. The override still wins.
    pub fn vcs_audience<'a>(&'a self, override_audience: Option<&'a str>) -> &'a str {
        override_audience
            .or(self.oidc.vcs_audience.as_deref())
            .unwrap_or(&self.oidc.audience)
    }

    /// The scopes to request from the token/device endpoints, defaulting to
    /// [`DEFAULT_SCOPES`] when the discovery doc doesn't specify any.
    pub fn scopes(&self) -> &str {
        self.oidc
            .scopes
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(DEFAULT_SCOPES)
    }
}

/// Fetch-once-per-URL cache. Entries are leaked so callers keep the convenient
/// `&'static` borrow; a process talks to one (rarely two) URLs, so the leak is
/// bounded and intentional.
struct UrlCache<T: 'static>(OnceLock<Mutex<HashMap<String, &'static T>>>);

impl<T> UrlCache<T> {
    const fn new() -> Self {
        Self(OnceLock::new())
    }

    fn get_or_fetch(
        &self,
        url: &str,
        fetch: impl FnOnce() -> Result<T, AuthError>,
    ) -> Result<&'static T, AuthError> {
        let mut map = self
            .0
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .expect("discovery cache poisoned");
        if let Some(v) = map.get(url) {
            return Ok(v);
        }
        let v: &'static T = Box::leak(Box::new(fetch()?));
        map.insert(url.to_string(), v);
        Ok(v)
    }
}

/// Process-wide discovery cache, keyed by `api_url` (§5: "Fetch once, cache in
/// memory for the process").
static CACHE: UrlCache<Discovery> = UrlCache::new();

/// Fetch (or return the cached) discovery document for `api_url`.
pub fn fetch(api_url: &str) -> Result<&'static Discovery, AuthError> {
    CACHE.get_or_fetch(api_url, || {
        HttpClient::new(api_url)
            .get_json("/.well-known/terramantle-cli.json")
            .map_err(AuthError::Discovery)
    })
}

/// Process-wide cache for the issuer's RFC 8414 metadata, keyed by discovery URL.
static OIDC_CACHE: UrlCache<OidcEndpoints> = UrlCache::new();

/// Fetch (or return the cached) OIDC provider metadata from `discovery_url` (an
/// absolute `.well-known/openid-configuration` URL). Supplies the absolute
/// token/device endpoints the flows post to.
pub fn fetch_oidc_endpoints(discovery_url: &str) -> Result<&'static OidcEndpoints, AuthError> {
    OIDC_CACHE.get_or_fetch(discovery_url, || {
        HttpClient::new("")
            .get_json(discovery_url)
            .map_err(AuthError::Discovery)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_discovery_with_null_device_client() {
        let json = r#"{
            "api_url": "https://registry.terramantle.dev",
            "oidc": {
                "issuer": "https://idp.example",
                "discovery_url": "https://idp.example/.well-known/openid-configuration",
                "audience": "https://registry.terramantle.dev",
                "vcs_audience": "https://registry.terramantle.dev",
                "device_client_id": null
            }
        }"#;
        let d: Discovery = serde_json::from_str(json).unwrap();
        assert_eq!(d.api_url, "https://registry.terramantle.dev");
        assert_eq!(d.oidc.issuer, "https://idp.example");
        assert_eq!(d.oidc.audience, "https://registry.terramantle.dev");
        assert_eq!(d.oidc.device_client_id, None);
        // No scopes advertised → the standard OIDC default.
        assert_eq!(d.scopes(), DEFAULT_SCOPES);
    }

    #[test]
    fn deserializes_discovery_with_device_client_and_scopes() {
        let json = r#"{
            "api_url": "https://reg",
            "oidc": {
                "issuer": "https://iss",
                "discovery_url": "https://iss/.well-known/openid-configuration",
                "audience": "https://reg",
                "device_client_id": "cli-public-123",
                "scopes": "openid profile email offline_access terramantle"
            }
        }"#;
        let d: Discovery = serde_json::from_str(json).unwrap();
        assert_eq!(d.oidc.device_client_id.as_deref(), Some("cli-public-123"));
        assert_eq!(d.oidc.vcs_audience, None);
        assert_eq!(
            d.scopes(),
            "openid profile email offline_access terramantle"
        );
    }

    #[test]
    fn empty_scopes_fall_back_to_default() {
        let json = r#"{
            "api_url": "https://reg",
            "oidc": {
                "issuer": "https://iss",
                "discovery_url": "https://iss/x",
                "audience": "https://reg",
                "scopes": "  "
            }
        }"#;
        let d: Discovery = serde_json::from_str(json).unwrap();
        assert_eq!(d.scopes(), DEFAULT_SCOPES);
    }

    #[test]
    fn deserializes_oidc_endpoints_from_well_known() {
        // Shape of a typical IdP's .well-known/openid-configuration (extra
        // fields ignored). Only absolute endpoint URLs are consumed.
        let json = r#"{
            "issuer": "https://sso.example/app/",
            "authorization_endpoint": "https://sso.example/authorize/",
            "token_endpoint": "https://sso.example/token/",
            "device_authorization_endpoint": "https://sso.example/device/"
        }"#;
        let e: OidcEndpoints = serde_json::from_str(json).unwrap();
        assert_eq!(e.token_endpoint, "https://sso.example/token/");
        assert_eq!(
            e.device_authorization_endpoint.as_deref(),
            Some("https://sso.example/device/")
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
                "vcs_audience": "https://reg/vcs",
                "device_client_id": null
            }
        }"#;
        let d: Discovery = serde_json::from_str(json).unwrap();
        assert_eq!(d.issuer(Some("https://override")), "https://override");
        assert_eq!(d.issuer(None), "https://iss");
        assert_eq!(d.audience(Some("aud-override")), "aud-override");
        assert_eq!(d.audience(None), "https://reg");
        // vcs_audience: override > vcs_audience > audience.
        assert_eq!(d.vcs_audience(Some("aud-override")), "aud-override");
        assert_eq!(d.vcs_audience(None), "https://reg/vcs");
    }

    #[test]
    fn vcs_audience_falls_back_to_audience() {
        let json = r#"{
            "api_url": "https://reg",
            "oidc": {
                "issuer": "https://iss",
                "discovery_url": "https://iss/x",
                "audience": "https://reg"
            }
        }"#;
        let d: Discovery = serde_json::from_str(json).unwrap();
        assert_eq!(d.vcs_audience(None), "https://reg");
    }
}
