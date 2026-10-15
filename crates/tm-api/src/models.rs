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

/// The `GET /api/orgs` handler wraps rows as `{ orgs: [...] }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgListResponse {
    #[serde(default)]
    pub orgs: Vec<OrgMembership>,
}

/// One org membership row from `GET /api/orgs` (§4.1 layer 4). Consolidated here
/// from the auth slice so both auth and every other command share one type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgMembership {
    pub slug: String,
    pub role: String,
}

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

/// The `GET /api/v1/{org}/state/` list handler wraps rows as `{ workspaces }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceListResponse {
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
}

/// A workspace summary row (`WorkspaceSummary`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub latest_serial: Option<i64>,
    #[serde(default)]
    pub resource_count: Option<u64>,
    #[serde(default)]
    pub pushed_at: Option<i64>,
    #[serde(default)]
    pub pushed_by: Option<String>,
    #[serde(default)]
    pub scan_status: Option<String>,
    #[serde(default)]
    pub lock_id: Option<String>,
    #[serde(default)]
    pub lock_who: Option<String>,
}

/// The `GET .../versions` handler wraps rows as `{ versions }` (paginated via
/// `?limit=&offset=`; §7 default 50, max 100 — no `total`/cursor in the body, so
/// pagination exhausts by observing a short/empty page).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionListResponse {
    #[serde(default)]
    pub versions: Vec<StateVersion>,
}

/// One state version (`VersionSummary`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateVersion {
    pub id: String,
    pub serial: i64,
    pub resource_count: u64,
    #[serde(default)]
    pub lineage: Option<String>,
    #[serde(default)]
    pub pushed_by: Option<String>,
    pub pushed_at: i64,
    #[serde(default)]
    pub scan_status: Option<String>,
    #[serde(default)]
    pub critical_count: u64,
    #[serde(default)]
    pub high_count: u64,
    /// Source serial when this version is a restore (promote-copy); null = normal apply.
    #[serde(default)]
    pub promoted_from_serial: Option<i64>,
    // Actor metadata captured at push time (display-only).
    #[serde(default)]
    pub actor_type: Option<String>,
    #[serde(default)]
    pub actor_name: Option<String>,
}

// ── state: workspace providers (posture polling for lock push) ─────────────────
// Source: state.ts `GET /:workspace/providers` (~line 919): `{ lock_file, providers }`.

/// The `GET .../providers` response used for lock-push posture polling (§7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceProvidersResponse {
    #[serde(default)]
    pub lock_file: Option<LockFileMeta>,
    #[serde(default)]
    pub providers: Vec<WorkspaceProvider>,
}

/// Lock-file metadata attached to a workspace's providers response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockFileMeta {
    #[serde(default)]
    pub uploaded_at: Option<i64>,
    #[serde(default)]
    pub uploaded_by: Option<String>,
    #[serde(default)]
    pub infrastructure_repo_url: Option<String>,
}

/// Finding counts for one provider version, from the cross-org scan cache.
///
/// Note the field names: the rollup is `{critical, high, medium, low}` — **not**
/// the `*_count` spelling used elsewhere — and it carries no total. An *absent*
/// rollup means "not scanned"; a present rollup of all zeroes means "scanned and
/// clean". Conflating those two is what the posture gate got wrong.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionScanRollup {
    #[serde(default)]
    pub critical: u64,
    #[serde(default)]
    pub high: u64,
    #[serde(default)]
    pub medium: u64,
    #[serde(default)]
    pub low: u64,
}

impl VersionScanRollup {
    /// Whether this rollup trips the posture gate.
    ///
    /// `critical || high` only — matching the documented `--at-risk` filter
    /// (§7). `medium`/`low` are deliberately **not** gating: they are carried for
    /// display, and treating them as at-risk would fail nearly every CI run.
    pub fn is_at_risk(&self) -> bool {
        self.critical > 0 || self.high > 0
    }
}

/// One in-use provider with its latest-matching + freshness fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceProvider {
    pub id: String,
    #[serde(default)]
    pub provider_address: Option<String>,
    pub namespace: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub upstream: Option<String>,
    pub current_version: String,
    #[serde(default)]
    pub constraints: Option<String>,
    #[serde(default)]
    pub latest_version: Option<String>,
    /// Constraint-respecting newest; `None` until the lookup queue lands the data.
    #[serde(default)]
    pub latest_matching: Option<String>,
    #[serde(default)]
    pub latest_version_checked_at: Option<i64>,
    #[serde(default)]
    pub provider_repo_url: Option<String>,
    /// Scan rollup for `current_version` — the version actually locked, so this
    /// is what the posture verdict reads. `None` ⇒ not scanned.
    #[serde(default)]
    pub current_scan: Option<VersionScanRollup>,
    /// Scan rollup for `latest_version`. `None` when there is no `latest_version`
    /// or it has no completed scan.
    #[serde(default)]
    pub latest_scan: Option<VersionScanRollup>,
}

// ── state: promote ─────────────────────────────────────────────────────────────
// Source: state.ts promote handler → `{ ok, version_id, serial, source_serial,
// scan_enqueued, ... }`. (The brief said `{version, source_serial}`; the live
// handler returns the richer shape modelled here.)

/// Response from `POST .../versions/{versionId}/promote`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromoteResponse {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub version_id: Option<String>,
    /// New serial created by the promote-copy.
    #[serde(default)]
    pub serial: Option<i64>,
    /// Serial the promoted version was copied from.
    #[serde(default)]
    pub source_serial: Option<i64>,
    #[serde(default)]
    pub scan_enqueued: bool,
    #[serde(default)]
    pub idempotent_replay: bool,
}

// ── state: lock ────────────────────────────────────────────────────────────────
// Source: state.ts `GET/DELETE /:workspace/lock`; StateLock in types.ts. GET wraps
// as `{ lock: null | {...} }`. PII fields (who/ip/geo) are member+-gated server-side
// so they are optional here.

/// The `GET .../lock` response wrapper: `{ lock: null | LockInfo }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockGetResponse {
    #[serde(default)]
    pub lock: Option<LockInfo>,
}

/// Current lock holder (`StateLock`). `lock_id` is the id echoed back on unlock (§7 N4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockInfo {
    pub lock_id: String,
    #[serde(default)]
    pub operation: Option<String>,
    /// The holder; member+-gated so absent for readers.
    #[serde(default)]
    pub who: Option<String>,
    #[serde(default)]
    pub info: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub actor_type: Option<String>,
    #[serde(default)]
    pub actor_name: Option<String>,
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape that mattered: `listWorkspaceProviders` has always sent
    /// `current_scan` / `latest_scan`, but the model did not declare them — and
    /// because no model denies unknown fields, serde dropped them in silence.
    /// That is what left the posture gate with no severity data to act on.
    #[test]
    fn workspace_provider_reads_the_scan_rollups() {
        let body = r#"{
            "id":"wp1","provider_address":"registry.terraform.io/hashicorp/aws",
            "namespace":"hashicorp","type":"aws","upstream":"registry.terraform.io",
            "current_version":"5.2.0","constraints":"~> 5.0",
            "latest_version":"5.4.1","latest_matching":"5.4.1",
            "current_scan":{"critical":2,"high":3,"medium":7,"low":11},
            "latest_scan":{"critical":0,"high":0,"medium":0,"low":0}
        }"#;
        let p: WorkspaceProvider = serde_json::from_str(body).unwrap();
        let current = p.current_scan.expect("current_scan present");
        assert_eq!((current.critical, current.high), (2, 3));
        assert!(current.is_at_risk());
        // Present-but-empty is a positive clean result, NOT "unknown".
        let latest = p.latest_scan.expect("latest_scan present");
        assert!(!latest.is_at_risk());
    }

    /// `null` and an omitted key both mean "not scanned", and must stay
    /// distinguishable from an all-zero rollup.
    #[test]
    fn absent_and_null_rollups_are_both_none() {
        let omitted = r#"{"id":"w","namespace":"n","type":"t","current_version":"1.0.0"}"#;
        let p: WorkspaceProvider = serde_json::from_str(omitted).unwrap();
        assert!(p.current_scan.is_none() && p.latest_scan.is_none());

        let explicit_null = r#"{"id":"w","namespace":"n","type":"t","current_version":"1.0.0",
            "current_scan":null,"latest_scan":null}"#;
        let p: WorkspaceProvider = serde_json::from_str(explicit_null).unwrap();
        assert!(p.current_scan.is_none() && p.latest_scan.is_none());

        // …whereas an empty object deserializes to a real, clean rollup.
        let empty_obj = r#"{"id":"w","namespace":"n","type":"t","current_version":"1.0.0",
            "current_scan":{}}"#;
        let p: WorkspaceProvider = serde_json::from_str(empty_obj).unwrap();
        assert_eq!(p.current_scan, Some(VersionScanRollup::default()));
    }

    /// Only critical/high gate, matching the documented `--at-risk` filter. A
    /// provider carrying only mediums and lows must not fail a CI push.
    #[test]
    fn only_critical_and_high_are_at_risk() {
        let noisy = VersionScanRollup {
            critical: 0,
            high: 0,
            medium: 40,
            low: 99,
        };
        assert!(!noisy.is_at_risk(), "medium/low must not gate");

        for (crit, high) in [(1, 0), (0, 1), (1, 1)] {
            let r = VersionScanRollup {
                critical: crit,
                high,
                medium: 0,
                low: 0,
            };
            assert!(r.is_at_risk(), "critical={crit} high={high} must gate");
        }
        assert!(!VersionScanRollup::default().is_at_risk());
    }

    /// The scanner may add severities; extra keys must not fail the parse.
    #[test]
    fn rollup_tolerates_unknown_severities() {
        let r: VersionScanRollup =
            serde_json::from_str(r#"{"critical":1,"high":0,"unknown":5,"negligible":2}"#).unwrap();
        assert_eq!(r.critical, 1);
        assert!(r.is_at_risk());
    }
}
