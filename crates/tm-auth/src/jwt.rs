//! Local JWT decoding for `auth whoami` (SPEC §5, correction S6): token **type
//! and expiry are decoded from the JWT locally — no server introspection**.
//!
//! We only need the claims, not signature verification (the server verifies on
//! use). We hand-parse the base64url payload to avoid pulling a validation
//! stack and to keep the "no network" test guarantee trivial.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;

use crate::AuthError;

/// The subset of standard claims we surface in `whoami`.
#[derive(Debug, Clone, Deserialize)]
pub struct Claims {
    #[serde(default)]
    pub sub: Option<String>,
    #[serde(default)]
    pub iss: Option<String>,
    /// `aud` may be a string or an array of strings.
    #[serde(default)]
    pub aud: Option<Audience>,
    /// Expiry, seconds since the Unix epoch.
    #[serde(default)]
    pub exp: Option<i64>,
    /// Authorized party — present on OIDC tokens; `oidc` marks CI provenance.
    #[serde(default)]
    pub azp: Option<String>,
    /// Email claim — present on human tokens.
    #[serde(default)]
    pub email: Option<String>,
}

/// `aud` is either a single string or a list.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    /// Render for display (comma-joined).
    pub fn display(&self) -> String {
        match self {
            Audience::One(s) => s.clone(),
            Audience::Many(v) => v.join(", "),
        }
    }
}

/// Inferred token type, from `azp`/issuer/email (§5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    /// Interactive user (device flow) — has an email claim.
    Human,
    /// CI OIDC provenance (`azp == "oidc"`).
    Oidc,
    /// Machine/service token (client credentials).
    Bot,
}

impl std::fmt::Display for TokenType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            TokenType::Human => "human",
            TokenType::Oidc => "oidc",
            TokenType::Bot => "bot",
        })
    }
}

impl Claims {
    /// Infer the token type. `azp == "oidc"` → CI OIDC; an email claim → human;
    /// otherwise a bot/service token.
    pub fn token_type(&self) -> TokenType {
        if self.azp.as_deref() == Some("oidc") {
            return TokenType::Oidc;
        }
        if self.email.is_some() {
            return TokenType::Human;
        }
        TokenType::Bot
    }
}

/// Decode the claims of a JWT locally, **without** verifying the signature.
pub fn decode_claims(token: &str) -> Result<Claims, AuthError> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or(AuthError::MalformedJwt("not a three-part JWT"))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| AuthError::MalformedJwt("payload is not valid base64url"))?;
    serde_json::from_slice(&bytes).map_err(|_| AuthError::MalformedJwt("payload is not valid JSON"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an unsigned test JWT: `base64url(header).base64url(payload).`.
    fn craft(payload: &str) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
        let body = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        format!("{header}.{body}.")
    }

    #[test]
    fn decodes_human_token_claims() {
        let jwt = craft(
            r#"{"sub":"user-1","iss":"https://iss","aud":"https://reg","exp":1893456000,"email":"rhys@x.io"}"#,
        );
        let c = decode_claims(&jwt).unwrap();
        assert_eq!(c.sub.as_deref(), Some("user-1"));
        assert_eq!(c.iss.as_deref(), Some("https://iss"));
        assert_eq!(c.aud.as_ref().unwrap().display(), "https://reg");
        assert_eq!(c.exp, Some(1893456000));
        assert_eq!(c.token_type(), TokenType::Human);
    }

    #[test]
    fn infers_oidc_from_azp() {
        let jwt = craft(r#"{"sub":"repo:acme/x","azp":"oidc"}"#);
        let c = decode_claims(&jwt).unwrap();
        assert_eq!(c.token_type(), TokenType::Oidc);
    }

    #[test]
    fn infers_bot_when_no_email_no_oidc() {
        let jwt = craft(r#"{"sub":"svc-account","azp":"client-123"}"#);
        let c = decode_claims(&jwt).unwrap();
        assert_eq!(c.token_type(), TokenType::Bot);
    }

    #[test]
    fn aud_array_is_joined() {
        let jwt = craft(r#"{"aud":["a","b"]}"#);
        let c = decode_claims(&jwt).unwrap();
        assert_eq!(c.aud.as_ref().unwrap().display(), "a, b");
    }

    #[test]
    fn rejects_non_jwt() {
        assert!(matches!(
            decode_claims("not-a-jwt"),
            Err(AuthError::MalformedJwt(_))
        ));
    }

    #[test]
    fn rejects_bad_base64_payload() {
        assert!(matches!(
            decode_claims("aaa.!!!not-base64!!!.bbb"),
            Err(AuthError::MalformedJwt(_))
        ));
    }
}
