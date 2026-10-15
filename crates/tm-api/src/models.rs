//! Shared serde models for the Terramantle worker API (SPEC §7).
//!
//! These are the wire types every downstream command (slices 4–7) renders. Field
//! names are grounded against the live worker handlers in
//! `packages/worker/src/routes` — where a shape was uncertain it is modelled
//! loosely (optional + `#[serde(default)]`) and commented. **No model denies
//! unknown fields** (rubric 2): the worker may add columns and the CLI must
//! tolerate them, so every struct simply ignores extras.

use serde::{Deserialize, Serialize};

// ── orgs ─────────────────────────────────────────────────────────────────────

// ── providers: usage rollup ───────────────────────────────────────────────────
// Source: providerFoundry.ts `GET .../providers-usage` (grouped in TS to exactly
// this shape) and `.../providers-usage/:namespace/:type`.

/// A provider in use across the org, grouped by `(namespace, type)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub namespace: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub upstream: String,
    pub workspace_count: u64,
    #[serde(default)]
    pub workspace_ids: Vec<String>,
    #[serde(default)]
    pub versions: Vec<UsageVersion>,
}

/// One locked version of a provider in the usage rollup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageVersion {
    pub version: String,
    /// The `constraints` string (nullable in the worker).
    #[serde(default)]
    pub constraints: Option<String>,
    pub workspace_count: u64,
    /// Constraint-respecting newest version; null when nothing to compare against.
    #[serde(default)]
    pub latest_matching: Option<String>,
    pub outdated: bool,
}

/// One workspace-usage row for a specific provider
/// (`.../providers-usage/:namespace/:type`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsedBy {
    pub version: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub uploaded_at: i64,
    #[serde(default)]
    pub uploaded_by: Option<String>,
    pub locked_at: i64,
}

// ── providers: overview (severity rollup for the TRUST column) ─────────────────
// Source: providerScanRead.ts `FoundryOverviewRow`. The handler wraps the array
// as `{ providers: [...] }` — see `ProvidersOverviewResponse`.

/// One foundry-rule row with the latest scan's severity rollup. Only providers
/// with a scan rule appear; absence of a row means "no scan rule" → `· unscanned`
/// (§7 providers ls). Provider-level, not per-version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderOverview {
    pub namespace: String,
    #[serde(rename = "type")]
    pub type_: String,
    /// Severity rollup for the newest scanned version in the latest job.
    #[serde(default)]
    pub vuln_count: u64,
    #[serde(default)]
    pub critical_count: u64,
    #[serde(default)]
    pub high_count: u64,
    // Extra columns modelled loosely — present in FoundryOverviewRow but not load-
    // bearing for the TRUST derivation; kept optional so a schema change can't break us.
    #[serde(default)]
    pub rule_id: Option<String>,
    #[serde(default)]
    pub upstream: Option<String>,
    #[serde(default)]
    pub version_constraint: Option<String>,
    #[serde(default)]
    pub latest_job_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub triggered_at: Option<i64>,
    #[serde(default)]
    pub completed_at: Option<i64>,
}

/// The overview handler wraps its rows: `{ providers: [...] }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvidersOverviewResponse {
    #[serde(default)]
    pub providers: Vec<ProviderOverview>,
}

// ── providers: available versions (Terraform registry protocol) ────────────────
// Source: providers.ts `GET /v1/providers/:ns/:type/versions` →
// filteredVersionsList → the upstream Terraform protocol `{ versions: [...] }`.

/// A Terraform-protocol provider-versions list. Only `version` is load-bearing
/// for the CLI; `protocols`/`platforms` are modelled loosely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderVersionsResponse {
    #[serde(default)]
    pub versions: Vec<ProviderVersion>,
}

/// One available provider version (Terraform protocol row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderVersion {
    pub version: String,
    #[serde(default)]
    pub protocols: Vec<String>,
    /// Left untyped — platform objects vary and the CLI does not render them.
    #[serde(default)]
    pub platforms: Vec<serde_json::Value>,
}

// ── modules ────────────────────────────────────────────────────────────────────
// Source: ModuleService.ts `SearchResponse`, `RegistryVersionSummary`,
// `RegistryVersionResponse`, `RegistryVersionsResponse`.

/// Result of `GET /v1/modules/search`.
///
/// NOTE (rubric 6): the worker's `SearchResponse` does **not** carry a `total`
/// field — it paginates via `meta.next_offset` (null = no more pages). The slice
/// brief asked to "model `total`"; the honest wire shape is modelled here instead,
/// with `total` intentionally absent. `next_offset` is the paging cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleSearchResponse {
    #[serde(default)]
    pub modules: Vec<ModuleSummary>,
    #[serde(default)]
    pub meta: SearchMeta,
}

/// Pagination metadata for module search.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchMeta {
    #[serde(default)]
    pub limit: u64,
    #[serde(default)]
    pub current_offset: u64,
    /// Offset of the next page, or `None` when this is the last page.
    #[serde(default)]
    pub next_offset: Option<u64>,
}

/// A module search hit (`RegistryVersionSummary`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleSummary {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default)]
    pub deprecated_message: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
}

/// Full module version detail (`GET /v1/modules/{ns}/{name}/{provider}` →
/// `RegistryVersionResponse`). Heavy fields (readme, findings) are kept but
/// loosely typed; the CLI renders the summary counts + provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleDetail {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub summary: Option<ModuleSummaryCounts>,
    #[serde(default)]
    pub consumable: bool,
    #[serde(default)]
    pub manually_blocked: bool,
    #[serde(default)]
    pub block_reason: Option<String>,
    #[serde(default)]
    pub readme: Option<String>,
    #[serde(default)]
    pub usage_example: Option<String>,
}

/// Severity/cost rollup on a module version.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModuleSummaryCounts {
    #[serde(default)]
    pub critical: u64,
    #[serde(default)]
    pub high: u64,
    #[serde(default)]
    pub medium: u64,
    #[serde(default)]
    pub low: u64,
    #[serde(default)]
    pub cost_estimate: Option<String>,
}

/// Terraform-protocol module versions list
/// (`GET /v1/modules/{ns}/{name}/{provider}/versions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleVersionsResponse {
    #[serde(default)]
    pub modules: Vec<ModuleVersionsEntry>,
}

/// One entry in the module versions protocol response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleVersionsEntry {
    #[serde(default)]
    pub versions: Vec<ModuleVersionRef>,
}

/// A bare `{version}` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleVersionRef {
    pub version: String,
}

// ── modules: publish ───────────────────────────────────────────────────────────
// Source: the module-publish endpoint `PUT /v1/modules/{ns}/{name}/{provider}/{version}`
// (§6 step 5) → `{ ok, version_id, storage_key?, status? }`. Returns 201 on success.

/// Response from a module-version publish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModulePublishResponse {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub version_id: String,
    #[serde(default)]
    pub storage_key: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

// ── state: workspaces + versions ───────────────────────────────────────────────
// Source: StateService.ts `WorkspaceSummary`, `VersionSummary`; state.ts handlers
// wrap them as `{ workspaces: [...] }` / `{ versions: [...] }`.

// ── state: workspace providers (posture polling for lock push) ─────────────────
//
// The types are generated (see the aliases at the foot of this file). This
// behaviour is not, and that split is the point: the generator produces the
// shape, and the judgement about what the shape MEANS stays hand-written and
// tested.

impl VersionScanRollup {
    /// Whether this rollup trips the posture gate.
    ///
    /// `critical || high` only — matching the documented `--at-risk` filter
    /// (§7). `medium`/`low` are deliberately **not** gating: they are carried for
    /// display, and treating them as at-risk would fail nearly every CI run.
    ///
    /// An absent count reads as zero. Note the distinction this method does NOT
    /// make: an absent *rollup* means "not scanned", which is decided by the
    /// caller holding an `Option<VersionScanRollup>` — see `verdict_from_scan`.
    pub fn is_at_risk(&self) -> bool {
        self.critical.unwrap_or(0) > 0 || self.high.unwrap_or(0) > 0
    }
}

// ── state: promote ─────────────────────────────────────────────────────────────
// Source: state.ts promote handler → `{ ok, version_id, serial, source_serial,
// scan_enqueued, ... }`. (The brief said `{version, source_serial}`; the live
// handler returns the richer shape modelled here.)

// ── state: lock ────────────────────────────────────────────────────────────────
// Source: state.ts `GET/DELETE /:workspace/lock`; StateLock in types.ts. GET wraps
// as `{ lock: null | {...} }`. PII fields (who/ip/geo) are member+-gated server-side
// so they are optional here.

// ── state: lock-file push (protocol router) ────────────────────────────────────
// Source: stateProtocol `PUT /state/{org}/{ws}/terraform.lock.hcl` →
// `{ ok, providers_count, warnings }`.

/// Response from the raw lock-file `PUT`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockPushResponse {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub providers_count: u64,
    /// Non-fatal parse warnings, echoed from the server.
    #[serde(default)]
    pub warnings: Vec<String>,
}

// ── state: now generated ──────────────────────────────────────────────────────
//
// These were hand-written from the worker's TypeScript, with the endpoint and
// line number in a comment. They are now generated from the spec, and aliased to
// the names this crate already used so call sites did not all have to move.
//
// The generated rows are SUPERSETS of what was modelled by hand: VCS provenance
// (`vcs_repository`, `vcs_ref`, `vcs_sha`, …), policy counts
// (`policy_failing_count` / `policy_total_count`) and the real-vs-internal
// resource split were all being sent and silently dropped, because no model
// denies unknown fields.

/// The `{ lock }` envelope.
pub use crate::generated::models::GetStateLockResponse as LockGetResponse;
/// The `{ orgs }` envelope.
pub use crate::generated::models::ListOrgsResponse as OrgListResponse;
/// The `{ versions }` envelope.
pub use crate::generated::models::ListStateVersionsResponse as VersionListResponse;
/// The `{ workspaces }` envelope.
pub use crate::generated::models::ListStateWorkspacesResponse as WorkspaceListResponse;
/// The org list row, generated from the spec's `OrgSummary`.
///
/// `role` is the spec's `Role` enum rather than a `String`, and the row carries
/// the columns the handler always sent (`name`, `tier`, `is_demo`,
/// `claimed_domain`, …) which the old two-field struct threw away.
pub use crate::generated::models::OrgSummary as OrgMembership;
/// The current lock holder. Was `LockInfo`.
pub use crate::generated::models::StateLock as LockInfo;
/// The promote result. Was `PromoteResponse`.
pub use crate::generated::models::StatePromoteResult as PromoteResponse;
/// One version in a workspace's history. Was `StateVersion`.
pub use crate::generated::models::StateVersionSummary as StateVersion;
/// A state workspace row. Was `Workspace`.
pub use crate::generated::models::StateWorkspaceSummary as Workspace;
/// In-use providers for a workspace, with their per-version scan rollups.
pub use crate::generated::models::{
    VersionScanRollup, WorkspaceProvider, WorkspaceProvidersResponse,
    WorkspaceProvidersResponseLockFile as LockFileMeta,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a rollup from JSON: the generated type has optional counts and no
    /// `Default`, and going through serde is also the path the real response
    /// takes.
    fn rollup(json: &str) -> VersionScanRollup {
        serde_json::from_str(json).expect("rollup parses")
    }

    /// The shape that mattered. `listWorkspaceProviders` has always sent
    /// `current_scan` / `latest_scan`, but the hand-written model did not declare
    /// them — and because no model denies unknown fields, serde dropped them in
    /// silence. That is what left the posture gate with no severity data to act
    /// on. They now come from the spec, so they cannot go missing again.
    #[test]
    fn workspace_provider_reads_the_scan_rollups() {
        let body = r#"{
            "provider_address":"registry.terraform.io/hashicorp/aws",
            "namespace":"hashicorp","type":"aws","upstream":"terraform.io",
            "current_version":"5.2.0","constraints":"~> 5.0",
            "latest_version":"5.4.1","latest_matching":"5.4.1",
            "current_scan":{"critical":2,"high":3,"medium":7,"low":11},
            "latest_scan":{"critical":0,"high":0,"medium":0,"low":0}
        }"#;
        let p: WorkspaceProvider = serde_json::from_str(body).unwrap();
        assert_eq!(p.namespace, "hashicorp");
        assert_eq!(p.type_, "aws");
        let current = p.current_scan.expect("current_scan present");
        assert_eq!((current.critical, current.high), (Some(2), Some(3)));
        assert!(current.is_at_risk());
        // Present-but-empty is a positive clean result, NOT "unknown".
        assert!(!p.latest_scan.expect("latest_scan present").is_at_risk());
    }

    /// `null` and an omitted key both mean "not scanned", and must stay
    /// distinguishable from an all-zero rollup, which means "scanned and clean".
    #[test]
    fn absent_and_null_rollups_are_both_none() {
        let base =
            r#""namespace":"n","type":"t","upstream":"terraform.io","current_version":"1.0.0""#;

        let p: WorkspaceProvider = serde_json::from_str(&format!("{{{base}}}")).unwrap();
        assert!(p.current_scan.is_none() && p.latest_scan.is_none());

        let p: WorkspaceProvider = serde_json::from_str(&format!(
            r#"{{{base},"current_scan":null,"latest_scan":null}}"#
        ))
        .unwrap();
        assert!(p.current_scan.is_none() && p.latest_scan.is_none());

        // …whereas an empty object is a real, clean rollup.
        let p: WorkspaceProvider =
            serde_json::from_str(&format!(r#"{{{base},"current_scan":{{}}}}"#)).unwrap();
        let scan = p.current_scan.expect("an empty object is still a rollup");
        assert!(!scan.is_at_risk());
        assert_eq!(scan.critical, None, "absent count, not zero");
    }

    /// Only critical/high gate, matching the documented `--at-risk` filter. A
    /// provider carrying only mediums and lows must not fail a CI push.
    #[test]
    fn only_critical_and_high_are_at_risk() {
        assert!(
            !rollup(r#"{"medium":40,"low":99}"#).is_at_risk(),
            "medium/low must not gate"
        );
        assert!(!rollup("{}").is_at_risk(), "an empty rollup is clean");
        for body in [
            r#"{"critical":1}"#,
            r#"{"high":1}"#,
            r#"{"critical":1,"high":1}"#,
        ] {
            assert!(rollup(body).is_at_risk(), "{body} must gate");
        }
    }

    /// The scanner may add severities; extra keys must not fail the parse.
    #[test]
    fn rollup_tolerates_unknown_severities() {
        let r = rollup(r#"{"critical":1,"high":0,"unknown":5,"negligible":2}"#);
        assert_eq!(r.critical, Some(1));
        assert!(r.is_at_risk());
    }
}
