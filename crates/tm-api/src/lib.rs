//! Typed API client for the Terramantle registry/worker (SPEC §7).
//!
//! [`Client`] is the one entry point downstream commands (slices 4–7) use: it
//! wraps an [`HttpClient`] and exposes one method per §7 endpoint, grouped by
//! resource ([`orgs`], [`providers`], [`modules`], [`state`]), each returning a
//! typed serde model from [`models`]. Failures are [`ApiError`], which maps the
//! worker's `{error}` string to a §9 exit code via [`ApiError::exit_code`].
//!
//! ## Auth
//! Every authed method needs a resolved bearer token + api_url. The caller
//! resolves the token with `tm_auth::resolve_token` and builds the client via
//! [`Client::new`] (or [`Client::with_refresh`] for the device flow). `tm-api`
//! stays free of `tm-auth`/keyring dependencies; refresh is injected as a hook
//! (see [`client`]).
//!
//! No model denies unknown fields and no token is ever logged (rubrics 2, 7).

pub mod client;
pub mod error;
pub mod models;

mod modules;
mod orgs;
mod providers;
mod state;
pub mod types;

pub use client::{HttpClient, RefreshHook};
pub use error::{ApiError, ErrorBody};
pub use models::*;
pub use state::{VERSIONS_PAGE_DEFAULT, VERSIONS_PAGE_MAX};

/// The typed Terramantle API client. Construct with a resolved bearer token +
/// api_url; call one method per §7 endpoint.
pub struct Client {
    http: HttpClient,
}

impl Client {
    /// Build a client for `api_url` authenticated with `token`.
    pub fn new(api_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            http: HttpClient::new(api_url).with_bearer(token),
        }
    }

    /// Build a client that refreshes on 401 via `hook` (device/keyring tokens).
    /// Non-device tokens use [`Client::new`] and simply fail on 401 (→ exit 5).
    pub fn with_refresh(
        api_url: impl Into<String>,
        token: impl Into<String>,
        hook: RefreshHook,
    ) -> Self {
        Self {
            http: HttpClient::new(api_url)
                .with_bearer(token)
                .with_refresh(hook),
        }
    }

    /// Build a client over an already-configured [`HttpClient`] (e.g. one that
    /// already carries a bearer / refresh hook). Escape hatch for callers that
    /// own the transport.
    pub fn from_http(http: HttpClient) -> Self {
        Self { http }
    }

    /// Borrow the underlying transport (used by the per-resource method modules).
    pub(crate) fn http(&self) -> &HttpClient {
        &self.http
    }
}
