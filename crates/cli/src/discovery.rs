//! Discovery commands — `providers ls/show`, `modules search/show` (SPEC §7).
//!
//! The API client is built once via `tm_auth::client(&ctx)` (never by cloning an
//! `HttpClient` — a clone drops the refresh hook). Org is resolved with the §4
//! precedence, defaulting to the single membership for human tokens.
//!
//! The usage⋈overview → TRUST derivation lives in the pure [`join`] module so it
//! is unit-tested with synthetic inputs and no network (rubric 3).

use std::collections::BTreeMap;

use serde::Serialize;
use tm_api::{
    Client, ModuleDetail, ModuleSearchResponse, ModuleSummary, ProviderOverview, ProviderUsage,
    UsedBy,
};
use tm_config::OutputFormat;

use crate::auth::{self};
use crate::cli::{Cli, ModulesCommand, ProvidersCommand};
use crate::commands::CmdResult;
use crate::output::{self, Style, TableView, TrustVerdict, DASH};

/// The `--all` follow-the-cursor cap for module search, to bound a runaway
/// `next_offset` chain (§7). A stderr note is printed when we stop here.
const SEARCH_ALL_CAP: usize = 500;

// ── pure TRUST join (rubric 3, no network) ─────────────────────────────────────

/// One rendered `providers ls` row: a (provider, version) pair with its
/// provider-level TRUST verdict. This is exactly what `-o json` emits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderRow {
    pub namespace: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub version: String,
    /// Constraint-respecting newest (`latest_matching`); `None` shows `—`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    /// Absolute newest available (overview/usage upstream); wide + json only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// Workspaces on *this* version.
    pub workspaces: u64,
    pub outdated: bool,
    /// Provider-level verdict (§7): identical for every version of a provider.
    pub trust: TrustVerdict,
    /// Upstream registry host (wide + json).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
}

/// Derive the provider-level [`TrustVerdict`] for `(namespace, type)` from the
/// overview index (§7):
///   * overview row present + any of critical/high/vuln > 0 → `AtRisk`
///   * overview row present + all zero                       → `Trusted`
///   * no overview row                                       → `Unscanned`
pub fn verdict_for(
    overview: &BTreeMap<(String, String), &ProviderOverview>,
    ns: &str,
    ty: &str,
) -> TrustVerdict {
    match overview.get(&(ns.to_string(), ty.to_string())) {
        Some(o) if o.critical_count > 0 || o.high_count > 0 || o.vuln_count > 0 => {
            TrustVerdict::AtRisk
        }
        Some(_) => TrustVerdict::Trusted,
        None => TrustVerdict::Unscanned,
    }
}

/// Build the overview lookup index keyed by `(namespace, type)`.
pub fn index_overview(
    overview: &[ProviderOverview],
) -> BTreeMap<(String, String), &ProviderOverview> {
    overview
        .iter()
        .map(|o| ((o.namespace.clone(), o.type_.clone()), o))
        .collect()
}

/// Left-join usage ⋈ overview into one row per (provider, version) in use, sorted
/// by (namespace, type, version). Pure — the heart of `providers ls` (rubric 3).
pub fn join_rows(usage: &[ProviderUsage], overview: &[ProviderOverview]) -> Vec<ProviderRow> {
    let idx = index_overview(overview);
    let mut rows: Vec<ProviderRow> = Vec::new();
    for u in usage {
        let trust = verdict_for(&idx, &u.namespace, &u.type_);
        // Absolute latest, when the overview surfaces it via its upstream link is
        // not available; usage has no absolute latest, so leave None unless a
        // wide-only field is derivable. (latest_version stays None here.)
        for v in &u.versions {
            rows.push(ProviderRow {
                namespace: u.namespace.clone(),
                type_: u.type_.clone(),
                version: v.version.clone(),
                latest: v.latest_matching.clone(),
                latest_version: None,
                workspaces: v.workspace_count,
                outdated: v.outdated,
                trust,
                upstream: Some(u.upstream.clone()).filter(|s| !s.is_empty()),
            });
        }
    }
    rows.sort_by(|a, b| {
        (&a.namespace, &a.type_, &a.version).cmp(&(&b.namespace, &b.type_, &b.version))
    });
    rows
}

// ── org resolution (rubric 6) ──────────────────────────────────────────────────

/// Resolve the effective org for a discovery command: the §4 precedence
/// (flag > env > context), and for **human** tokens a single-membership
/// auto-default. On a missing org this returns a clear error → exit 6/2 handled
/// by the caller; here we surface a `ConfigError::MissingOrg`-style message.
///
/// Returns `(client, org)` so the client (built once, with its refresh hook) is
/// reused for every call the command makes.
fn resolve_client_and_org(cli: &Cli) -> Result<(Client, String), Box<dyn std::error::Error>> {
    let ctx = auth::auth_context(cli)?;
    let client = auth::api_client(&ctx)?;

    // Config-side precedence first (flag > env > context).
    if let Some(org) = auth::config_org(cli)? {
        return Ok((client, org));
    }

    // No explicit org: for human tokens, default to the sole membership.
    match client.orgs_list() {
        Ok(memberships) if memberships.len() == 1 => {
            let org = memberships[0].slug.clone();
            eprintln!("using org '{org}' (sole membership)");
            Ok((client, org))
        }
        Ok(memberships) if memberships.is_empty() => Err(Box::new(MissingOrg)),
        Ok(_) => Err(Box::new(AmbiguousOrg)),
        // CI OIDC/bot tokens have no org endpoint (401/403/404) — require --org.
        Err(_) => Err(Box::new(MissingOrg)),
    }
}

#[derive(Debug)]
struct MissingOrg;
impl std::fmt::Display for MissingOrg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no org configured; set --org, TERRAMANTLE_ORG, or select a context")
    }
}
impl std::error::Error for MissingOrg {}

#[derive(Debug)]
struct AmbiguousOrg;
impl std::fmt::Display for AmbiguousOrg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("multiple org memberships; pass --org to choose one")
    }
}
impl std::error::Error for AmbiguousOrg {}

/// The exit code for an org-resolution failure (§9: missing config → usage-ish;
/// we use 6 "not found" only for real 404s, so a missing org is exit 2).
const EXIT_MISSING_ORG: i32 = 2;

// ── dispatch ───────────────────────────────────────────────────────────────────

pub fn providers(command: &ProvidersCommand, cli: &Cli) -> CmdResult {
    match command {
        ProvidersCommand::Ls { at_risk } => providers_ls(cli, *at_risk),
        ProvidersCommand::Show { provider } => providers_show(cli, provider),
    }
}

pub fn modules(command: &ModulesCommand, cli: &Cli) -> CmdResult {
    match command {
        ModulesCommand::Search { query, limit, all } => modules_search(cli, query, *limit, *all),
        ModulesCommand::Show { module } => modules_show(cli, module),
    }
}

// ── providers ls ────────────────────────────────────────────────────────────────

fn providers_ls(cli: &Cli, at_risk_only: bool) -> CmdResult {
    let (client, org) = match resolve_client_and_org(cli) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(EXIT_MISSING_ORG);
        }
    };

    let usage = match client.providers_usage(&org) {
        Ok(u) => u,
        Err(e) => return Ok(api_fail(&e)),
    };
    let overview = match client.providers_overview(&org) {
        Ok(o) => o,
        Err(e) => return Ok(api_fail(&e)),
    };

    let mut rows = join_rows(&usage, &overview);
    if at_risk_only {
        rows.retain(|r| r.trust == TrustVerdict::AtRisk);
    }

    let format = cli.global.output.unwrap_or_default();
    if output::print_structured(&rows, format)? {
        return Ok(0);
    }

    let style = Style::detect(cli.global.no_color);
    let wide = matches!(format, OutputFormat::Wide);
    print!("{}", render_providers_ls(&rows, wide, style));
    Ok(0)
}

/// Render the `providers ls` table (pure — used by golden tests).
pub fn render_providers_ls(rows: &[ProviderRow], wide: bool, style: Style) -> String {
    let mut headers = vec![
        "namespace",
        "type",
        "version",
        "latest",
        "workspaces",
        "outdated",
        "trust",
    ];
    if wide {
        headers.push("upstream");
        headers.push("latest_version");
    }
    let mut view = TableView::new(headers);
    for r in rows {
        let mut cells = vec![
            r.namespace.clone(),
            r.type_.clone(),
            r.version.clone(),
            r.latest.clone().unwrap_or_else(|| DASH.to_string()),
            r.workspaces.to_string(),
            output::outdated_glyph(r.outdated, style),
            r.trust.render(style),
        ];
        if wide {
            cells.push(r.upstream.clone().unwrap_or_else(|| DASH.to_string()));
            cells.push(r.latest_version.clone().unwrap_or_else(|| DASH.to_string()));
        }
        view.row(cells);
    }
    format!("{}\n", view.render())
}

// ── providers show ──────────────────────────────────────────────────────────────

/// One row of the `providers show` versions table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShowVersionRow {
    pub version: String,
    pub trust: TrustVerdict,
    /// Workspace names on this version (from the used-by rows).
    pub workspaces: Vec<String>,
}

/// The `providers show` json/yaml payload.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderShow {
    pub namespace: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub trust: TrustVerdict,
    pub versions: Vec<ShowVersionRow>,
}

fn providers_show(cli: &Cli, provider: &str) -> CmdResult {
    let (ns, ty) = match split_two(provider, "<ns>/<type>") {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(2);
        }
    };

    let (client, org) = match resolve_client_and_org(cli) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(EXIT_MISSING_ORG);
        }
    };

    let used_by = match client.providers_usage_detail(&org, &ns, &ty) {
        Ok(u) => u,
        Err(e) => return Ok(api_fail(&e)),
    };
    // /v1/providers/.../versions is authRequired + org-allowlist-filtered (§7 S4).
    // A 401/403 there → exit 5 with a clear message.
    let available = match client.provider_versions(&ns, &ty) {
        Ok(v) => v
            .versions
            .into_iter()
            .map(|v| v.version)
            .collect::<Vec<_>>(),
        Err(e) => {
            if matches!(e.status(), Some(401) | Some(403)) {
                eprintln!(
                    "error: not entitled to browse {ns}/{ty} versions (org '{org}'): {}",
                    e.message().unwrap_or("unauthorized")
                );
                return Ok(5);
            }
            return Ok(api_fail(&e));
        }
    };
    let overview = match client.providers_overview(&org) {
        Ok(o) => o,
        Err(e) => return Ok(api_fail(&e)),
    };

    let idx = index_overview(&overview);
    let trust = verdict_for(&idx, &ns, &ty);
    // The overview row carries the upstream/repo host; usage-detail rows do not.
    let upstream = overview
        .iter()
        .find(|o| o.namespace == ns && o.type_ == ty)
        .and_then(|o| o.upstream.clone());
    // No dedicated provider-repo field on the wire yet, so `repo` stays None and
    // the repo line is omitted (§7: "repo if available").
    let repo = None;

    let show = build_show(&ns, &ty, upstream, repo, trust, &used_by, &available);

    let format = cli.global.output.unwrap_or_default();
    if output::print_structured(&show, format)? {
        return Ok(0);
    }

    let style = Style::detect(cli.global.no_color);
    print!("{}", render_providers_show(&show, style));
    Ok(0)
}

/// Assemble the [`ProviderShow`] payload from used-by rows + available versions
/// (pure — used by golden tests). Versions in use carry their workspace list;
/// available-but-not-in-use versions render with an empty list. Each version's
/// trust is the same provider-level verdict (§7).
pub fn build_show(
    ns: &str,
    ty: &str,
    upstream: Option<String>,
    repo: Option<String>,
    trust: TrustVerdict,
    used_by: &[UsedBy],
    available: &[String],
) -> ProviderShow {
    // Group workspace names by the version they lock.
    let mut ws_by_version: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for u in used_by {
        ws_by_version
            .entry(u.version.clone())
            .or_default()
            .push(u.workspace_name.clone());
    }
    for names in ws_by_version.values_mut() {
        names.sort();
        names.dedup();
    }

    // Union of available versions and versions actually in use, so a locked
    // version missing from the (allowlist-filtered) available list still shows.
    let mut versions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for v in available {
        versions.entry(v.clone()).or_default();
    }
    for (v, names) in &ws_by_version {
        versions.insert(v.clone(), names.clone());
    }

    let mut rows: Vec<ShowVersionRow> = versions
        .into_iter()
        .map(|(version, workspaces)| ShowVersionRow {
            version,
            trust,
            workspaces,
        })
        .collect();
    // Newest first: reverse lexical is a good-enough proxy for semver here.
    rows.sort_by(|a, b| b.version.cmp(&a.version));

    ProviderShow {
        namespace: ns.to_string(),
        type_: ty.to_string(),
        upstream,
        repo,
        trust,
        versions: rows,
    }
}

/// Render the `providers show` output (header + versions table), pure.
pub fn render_providers_show(show: &ProviderShow, style: Style) -> String {
    let mut out = String::new();
    let addr = format!("{}/{}", show.namespace, show.type_);
    match &show.upstream {
        Some(up) if !up.is_empty() => out.push_str(&format!("{addr}   ({up})\n")),
        _ => out.push_str(&format!("{addr}\n")),
    }
    if let Some(repo) = &show.repo {
        if !repo.is_empty() {
            out.push_str(&format!("repo   {repo}\n"));
        }
    }
    out.push('\n');

    if show.versions.is_empty() {
        out.push_str("Not in any lock file.\n");
        return out;
    }

    let mut view = TableView::new(["version", "trust", "workspaces"]);
    let mut at_risk_ws = 0u64;
    for v in &show.versions {
        let ws = if v.workspaces.is_empty() {
            DASH.to_string()
        } else {
            v.workspaces.join(", ")
        };
        if v.trust == TrustVerdict::AtRisk {
            at_risk_ws += v.workspaces.len() as u64;
        }
        view.row([v.version.clone(), v.trust.render(style), ws]);
    }
    out.push_str(&view.render());
    out.push('\n');

    if at_risk_ws > 0 {
        let mark = if style.glyphs { "▲" } else { "WARN" };
        out.push_str(&format!(
            "\n{mark} {at_risk_ws} workspaces pull an at-risk version.\n"
        ));
    }
    out
}

// ── modules search ──────────────────────────────────────────────────────────────

fn modules_search(cli: &Cli, query: &str, limit: Option<u64>, all: bool) -> CmdResult {
    let (client, _org) = match resolve_client_and_org(cli) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(EXIT_MISSING_ORG);
        }
    };

    let page_limit = limit.unwrap_or(20);
    let mut hits: Vec<ModuleSummary> = Vec::new();
    let mut offset = 0u64;
    loop {
        let resp: ModuleSearchResponse = match client.modules_search(query, page_limit, offset) {
            Ok(r) => r,
            Err(e) => return Ok(api_fail(&e)),
        };
        hits.extend(resp.modules);
        match resp.meta.next_offset {
            Some(next) if all => {
                if hits.len() >= SEARCH_ALL_CAP {
                    eprintln!(
                        "note: stopped at {SEARCH_ALL_CAP} results (more available; narrow the query)"
                    );
                    break;
                }
                offset = next;
            }
            _ => break,
        }
    }

    let format = cli.global.output.unwrap_or_default();
    if output::print_structured(&hits, format)? {
        return Ok(0);
    }

    print!("{}", render_modules_search(&hits));
    Ok(0)
}

/// Render the `modules search` table (pure). DOWNLOADS is omitted — the search
/// wire shape (`RegistryVersionSummary`) carries no download count.
pub fn render_modules_search(hits: &[ModuleSummary]) -> String {
    let mut view = TableView::new(["namespace", "name", "latest"]);
    for m in hits {
        view.row([m.namespace.clone(), m.name.clone(), m.version.clone()]);
    }
    format!("{}\n", view.render())
}

// ── modules show ────────────────────────────────────────────────────────────────

/// The `modules show` json/yaml payload.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleShow {
    #[serde(flatten)]
    pub detail: ModuleDetail,
    pub versions: Vec<String>,
}

fn modules_show(cli: &Cli, module: &str) -> CmdResult {
    let (ns, name, provider) = match split_three(module) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(2);
        }
    };

    let (client, _org) = match resolve_client_and_org(cli) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(EXIT_MISSING_ORG);
        }
    };

    let detail = match client.module_show(&ns, &name, &provider) {
        Ok(d) => d,
        Err(e) => return Ok(api_fail(&e)),
    };
    let versions = match client.module_versions(&ns, &name, &provider) {
        Ok(v) => v
            .modules
            .into_iter()
            .flat_map(|m| m.versions.into_iter().map(|r| r.version))
            .collect::<Vec<_>>(),
        Err(e) => return Ok(api_fail(&e)),
    };

    let show = ModuleShow { detail, versions };

    let format = cli.global.output.unwrap_or_default();
    if output::print_structured(&show, format)? {
        return Ok(0);
    }

    print!("{}", render_module_show(&show));
    Ok(0)
}

/// Render `modules show` (summary + versions table), pure.
pub fn render_module_show(show: &ModuleShow) -> String {
    let d = &show.detail;
    // `provider` isn't a field on ModuleDetail, so the header is ns/name; the
    // summary block below carries description/source/latest.
    let mut out = format!("{}/{}\n", d.namespace, d.name);
    if let Some(desc) = &d.description {
        if !desc.is_empty() {
            out.push_str(&format!("{desc}\n"));
        }
    }
    out.push_str(&format!("latest   {}\n", d.version));
    out.push('\n');

    let mut view = TableView::new(["version"]);
    for v in &show.versions {
        view.row([v.clone()]);
    }
    out.push_str(&view.render());
    out.push('\n');
    out
}

// ── helpers ────────────────────────────────────────────────────────────────────

/// Map an API error to its §9 exit code, printing the preserved message first.
fn api_fail(e: &tm_api::ApiError) -> i32 {
    eprintln!("error: {e}");
    e.exit_code()
}

/// Split `a/b`, with a clear error naming the expected shape.
fn split_two(s: &str, shape: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
        return Err(format!("expected {shape}, got '{s}'"));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Split `ns/name/provider`.
fn split_three(s: &str) -> Result<(String, String, String), String> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
        return Err(format!("expected <ns>/<name>/<provider>, got '{s}'"));
    }
    Ok((
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_api::{ProviderOverview, ProviderUsage, UsageVersion, UsedBy};

    fn overview(ns: &str, ty: &str, crit: u64, high: u64, vuln: u64) -> ProviderOverview {
        ProviderOverview {
            namespace: ns.into(),
            type_: ty.into(),
            vuln_count: vuln,
            critical_count: crit,
            high_count: high,
            rule_id: None,
            upstream: Some("registry.terraform.io".into()),
            version_constraint: None,
            latest_job_id: None,
            status: None,
            triggered_at: None,
            completed_at: None,
        }
    }

    fn usage_version(v: &str, ws: u64, latest: &str, outdated: bool) -> UsageVersion {
        UsageVersion {
            version: v.into(),
            constraints: None,
            workspace_count: ws,
            latest_matching: Some(latest.into()),
            outdated,
        }
    }

    fn usage(ns: &str, ty: &str, versions: Vec<UsageVersion>) -> ProviderUsage {
        ProviderUsage {
            namespace: ns.into(),
            type_: ty.into(),
            upstream: "registry.terraform.io".into(),
            workspace_count: versions.iter().map(|v| v.workspace_count).sum(),
            workspace_ids: vec![],
            versions,
        }
    }

    // ── TRUST derivation (rubric 3) ──────────────────────────────────────────

    #[test]
    fn overview_with_high_is_at_risk() {
        let ov = vec![overview("hashicorp", "aws", 0, 2, 0)];
        let idx = index_overview(&ov);
        assert_eq!(verdict_for(&idx, "hashicorp", "aws"), TrustVerdict::AtRisk);
    }

    #[test]
    fn overview_with_critical_is_at_risk() {
        let ov = vec![overview("hashicorp", "aws", 1, 0, 0)];
        let idx = index_overview(&ov);
        assert_eq!(verdict_for(&idx, "hashicorp", "aws"), TrustVerdict::AtRisk);
    }

    #[test]
    fn overview_with_vuln_is_at_risk() {
        let ov = vec![overview("hashicorp", "aws", 0, 0, 3)];
        let idx = index_overview(&ov);
        assert_eq!(verdict_for(&idx, "hashicorp", "aws"), TrustVerdict::AtRisk);
    }

    #[test]
    fn overview_all_zero_is_trusted() {
        let ov = vec![overview("hashicorp", "random", 0, 0, 0)];
        let idx = index_overview(&ov);
        assert_eq!(
            verdict_for(&idx, "hashicorp", "random"),
            TrustVerdict::Trusted
        );
    }

    #[test]
    fn no_overview_row_is_unscanned_not_trusted() {
        let ov: Vec<ProviderOverview> = vec![];
        let idx = index_overview(&ov);
        assert_eq!(
            verdict_for(&idx, "opentofu", "null"),
            TrustVerdict::Unscanned
        );
    }

    // ── join granularity (one row per (provider, version)) ───────────────────

    #[test]
    fn one_row_per_version_in_use() {
        let usage = vec![usage(
            "hashicorp",
            "aws",
            vec![
                usage_version("5.2.0", 4, "5.4.1", true),
                usage_version("5.1.0", 1, "5.4.1", true),
            ],
        )];
        let overview = vec![overview("hashicorp", "aws", 0, 2, 0)];
        let rows = join_rows(&usage, &overview);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.trust == TrustVerdict::AtRisk));
        // sorted by version ascending
        assert_eq!(rows[0].version, "5.1.0");
        assert_eq!(rows[1].version, "5.2.0");
        assert_eq!(rows[1].workspaces, 4);
        assert_eq!(rows[1].latest.as_deref(), Some("5.4.1"));
    }

    #[test]
    fn unscanned_provider_rows_carry_unscanned_verdict() {
        let usage = vec![usage(
            "opentofu",
            "null",
            vec![usage_version("3.2.1", 2, "3.2.2", true)],
        )];
        let rows = join_rows(&usage, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].trust, TrustVerdict::Unscanned);
    }

    #[test]
    fn rows_sorted_across_providers() {
        let usage = vec![
            usage(
                "opentofu",
                "null",
                vec![usage_version("3.2.1", 2, "3.2.2", true)],
            ),
            usage(
                "hashicorp",
                "random",
                vec![usage_version("3.6.0", 7, "3.6.0", false)],
            ),
        ];
        let rows = join_rows(&usage, &[]);
        assert_eq!(rows[0].namespace, "hashicorp");
        assert_eq!(rows[1].namespace, "opentofu");
    }

    // ── json contract round-trip (rubric 2) ──────────────────────────────────

    #[test]
    fn provider_rows_json_round_trips() {
        let rows = join_rows(
            &[usage(
                "hashicorp",
                "aws",
                vec![usage_version("5.2.0", 4, "5.4.1", true)],
            )],
            &[overview("hashicorp", "aws", 0, 2, 0)],
        );
        let json = serde_json::to_string(&rows).unwrap();
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(back[0]["type"], "aws");
        assert_eq!(back[0]["trust"], "at-risk");
        assert_eq!(back[0]["workspaces"], 4);
    }

    #[test]
    fn provider_show_json_round_trips() {
        let used = vec![
            UsedBy {
                version: "5.2.0".into(),
                workspace_id: "w1".into(),
                workspace_name: "prod".into(),
                uploaded_at: 0,
                uploaded_by: None,
                locked_at: 0,
            },
            UsedBy {
                version: "5.2.0".into(),
                workspace_id: "w2".into(),
                workspace_name: "staging".into(),
                uploaded_at: 0,
                uploaded_by: None,
                locked_at: 0,
            },
        ];
        let show = build_show(
            "hashicorp",
            "aws",
            Some("registry.terraform.io".into()),
            None,
            TrustVerdict::AtRisk,
            &used,
            &["5.4.1".into(), "5.2.0".into()],
        );
        let json = serde_json::to_string(&show).unwrap();
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(back["trust"], "at-risk");
        // 5.4.1 first (newest), no workspaces; 5.2.0 has prod, staging
        assert_eq!(back["versions"][0]["version"], "5.4.1");
        assert_eq!(back["versions"][1]["workspaces"][0], "prod");
    }

    // ── show render + used-by grouping ───────────────────────────────────────

    #[test]
    fn show_groups_workspaces_by_version() {
        let used = vec![
            UsedBy {
                version: "5.2.0".into(),
                workspace_id: "w1".into(),
                workspace_name: "prod".into(),
                uploaded_at: 0,
                uploaded_by: None,
                locked_at: 0,
            },
            UsedBy {
                version: "5.2.0".into(),
                workspace_id: "w2".into(),
                workspace_name: "staging".into(),
                uploaded_at: 0,
                uploaded_by: None,
                locked_at: 0,
            },
        ];
        let show = build_show(
            "hashicorp",
            "aws",
            None,
            None,
            TrustVerdict::AtRisk,
            &used,
            &["5.4.1".into(), "5.2.0".into()],
        );
        let out = render_providers_show(&show, Style::plain());
        assert!(out.contains("prod, staging"), "{out}");
        assert!(
            out.contains("2 workspaces pull an at-risk version"),
            "{out}"
        );
    }

    #[test]
    fn show_empty_used_by_says_not_in_lock_file() {
        let show = build_show(
            "hashicorp",
            "aws",
            None,
            None,
            TrustVerdict::Trusted,
            &[],
            &[],
        );
        let out = render_providers_show(&show, Style::plain());
        assert!(out.contains("Not in any lock file."), "{out}");
    }

    // ── ls render (golden-ish, plain) ────────────────────────────────────────

    #[test]
    fn ls_plain_render_has_headers_and_rows() {
        let rows = join_rows(
            &[
                usage(
                    "hashicorp",
                    "aws",
                    vec![usage_version("5.2.0", 4, "5.4.1", true)],
                ),
                usage(
                    "hashicorp",
                    "random",
                    vec![usage_version("3.6.0", 7, "3.6.0", false)],
                ),
            ],
            &[
                overview("hashicorp", "aws", 0, 2, 0),
                overview("hashicorp", "random", 0, 0, 0),
            ],
        );
        let out = render_providers_ls(&rows, false, Style::plain());
        assert!(out.contains("NAMESPACE"));
        assert!(out.contains("WARN at-risk"));
        assert!(out.contains("OK trusted"));
        assert!(!out.contains('\u{1b}'), "plain render must have no ANSI");
    }

    #[test]
    fn split_helpers_reject_bad_shapes() {
        assert!(split_two("hashicorp", "<ns>/<type>").is_err());
        assert!(split_two("a/b/c", "<ns>/<type>").is_err());
        assert!(split_two("hashicorp/aws", "<ns>/<type>").is_ok());
        assert!(split_three("a/b/c").is_ok());
        assert!(split_three("a/b").is_err());
    }
}
