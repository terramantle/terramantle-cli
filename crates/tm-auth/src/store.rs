//! OS keyring storage for device-flow tokens (SPEC §5). Service `terramantle`,
//! account = `api_url`. CI never touches the keyring.
//!
//! We store the access token (and, when present, a refresh token) as a small
//! JSON blob under a single keyring entry so a future slice can add refresh
//! rotation without changing the entry layout.

use serde::{Deserialize, Serialize};

use crate::AuthError;

const SERVICE: &str = "terramantle";

/// A stored device-flow token bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToken {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

fn entry(api_url: &str) -> Result<keyring::Entry, AuthError> {
    keyring::Entry::new(SERVICE, api_url).map_err(|e| AuthError::Keyring(e.to_string()))
}

/// Persist a token bundle for `api_url`.
pub fn save(api_url: &str, token: &StoredToken) -> Result<(), AuthError> {
    let blob = serde_json::to_string(token).map_err(|e| AuthError::Keyring(e.to_string()))?;
    entry(api_url)?
        .set_password(&blob)
        .map_err(|e| AuthError::Keyring(e.to_string()))
}

/// Load the token bundle for `api_url`, if any. A missing entry yields `None`.
pub fn load(api_url: &str) -> Result<Option<StoredToken>, AuthError> {
    match entry(api_url)?.get_password() {
        Ok(blob) => {
            let t = serde_json::from_str(&blob).map_err(|e| AuthError::Keyring(e.to_string()))?;
            Ok(Some(t))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AuthError::Keyring(e.to_string())),
    }
}

/// Clear the token bundle for `api_url`. A missing entry is a no-op.
pub fn clear(api_url: &str) -> Result<(), AuthError> {
    match entry(api_url)?.delete_password() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AuthError::Keyring(e.to_string())),
    }
}
