//! Authentication flows for the Terramantle CLI (SPEC §5).
//!
//! Placeholder for a later slice (§10 slice 2): flow detection (GitHub/GitLab
//! OIDC, device flow, client-credentials, raw token), keyring storage, and
//! `auth login/logout/whoami` land there. Intentionally empty for now.

/// The auth mode to use for a request (§5). Detection + flows are implemented
/// in the auth slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    #[default]
    Auto,
    GitHub,
    GitLab,
    Device,
    Client,
    Token,
}
