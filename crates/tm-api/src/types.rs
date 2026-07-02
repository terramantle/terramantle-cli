//! Shared API model stubs. Fleshed out in the API-client slice (§10 slice 3).

use serde::{Deserialize, Serialize};

/// Trust Seal verdict for a provider/module version (§6). The wire
/// representation is finalized in the API slice; this is a placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trust {
    Trusted,
    AtRisk,
    Blocked,
}
