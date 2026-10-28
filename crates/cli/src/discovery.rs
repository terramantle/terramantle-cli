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
use tm_api::generated::models::{
    ModuleSearchResponse, ModuleVersionDetail, ProviderArtefactDetail, ProviderArtefactSummary,
    ProviderUpstream, ProviderVersionRow,
};
use tm_api::{Client, ModuleSummary, OrgMembership, VersionScanRollup};
use tm_config::OutputFormat;

use crate::auth::{self};
use crate::cli::{Cli, ModulesCommand, ProvidersCommand};
use crate::commands::CmdResult;
use crate::output::{self, Style, TableView, TrustVerdict, DASH};

/// The `--all` follow-the-cursor cap for module search, to bound a runaway
/// `next_offset` chain (§7). A stderr note is printed when we stop here.
const SEARCH_ALL_CAP: usize = 500;

// ── pure TRUST join (rubric 3, no network) ─────────────────────────────────────

/// One row of the `providers ls` table (also the `-o json` shape).
///
/// One row per PROVIDER, not per version. The removed `providers-usage`
/// endpoint returned a per-version usage rollup; the unified catalog that
/// replaced it is keyed on the artefact, with per-version detail behind
/// `providers show`. That is a visible change to this table, and an unavoidable
/// one — the per-version list is not served any more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderRow {
    pub namespace: String,
    #[serde(rename = "type")]
    pub type_: String,
    /// The org's version constraint, or `—` when it serves every version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraint: Option<String>,
    /// Newest version this org actually holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    /// Newest release upstream; wide + json only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// Workspaces whose lock file pins this provider — its blast radius.
    pub workspaces: u64,
    /// At least one workspace is behind its `latest_matching`.
    pub outdated: bool,
    /// Provider-level verdict, from the `latest_version` severity counts.
    pub trust: TrustVerdict,
    /// Upstream registry (wide + json).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
}

/// Derive the provider-level [`TrustVerdict`] from a catalog row.
///
/// The counts are `latest_version`'s only — never an older version's, even
/// where the older one has findings and the latest does not. So a zero means
/// "the latest version is clean OR has not been scanned", and this payload
/// cannot tell those apart: `scanned` per version is on `providers show`.
///
/// `Unscanned` is therefore reported when the provider holds no version at all;
/// anything else with zero counts reads as `Trusted`, which is the same
/// judgement the server's own UI makes from this row.
pub fn verdict_for_summary(p: &ProviderArtefactSummary) -> TrustVerdict {
    let critical = p.critical_count.unwrap_or(0);
    let high = p.high_count.unwrap_or(0);
    let vulns = p.vuln_count.unwrap_or(0);
    if critical > 0 || high > 0 || vulns > 0 {
        TrustVerdict::AtRisk
    } else if p.latest_version.is_none() {
        TrustVerdict::Unscanned
    } else {
        TrustVerdict::Trusted
    }
}

/// Derive a [`TrustVerdict`] from a workspace provider's own scan rollup.
///
/// This is the posture path's verdict source, and it needs no org-wide overview
/// call: `listWorkspaceProviders` already carries the rollup for the version the
/// workspace actually locks.
///
///   * rollup present + critical/high > 0 → `AtRisk`
///   * rollup present + all zero          → `Trusted` (scanned and clean)
///   * rollup absent                      → `Unscanned` (never scanned)
///
/// The absent-vs-empty distinction is the whole point: an empty rollup is a
/// positive clean result, while `None` means we simply do not know.
pub fn verdict_from_scan(scan: Option<&VersionScanRollup>) -> TrustVerdict {
    match scan {
        Some(r) if r.is_at_risk() => TrustVerdict::AtRisk,
        Some(_) => TrustVerdict::Trusted,
        None => TrustVerdict::Unscanned,
    }
}

/// Build one row per provider from the catalog list, sorted by
/// `(namespace, type)`. Pure — the heart of `providers ls` (rubric 3).
pub fn summary_rows(providers: &[ProviderArtefactSummary]) -> Vec<ProviderRow> {
    let mut rows: Vec<ProviderRow> = providers
        .iter()
        .map(|p| ProviderRow {
            namespace: p.namespace.clone(),
            type_: p.type_.clone(),
            constraint: p.version_constraint.clone().filter(|c| !c.is_empty()),
            latest: p.latest_version.clone(),
            latest_version: p.upstream_latest_version.clone(),
            workspaces: p.usage_workspace_count.unwrap_or(0).max(0) as u64,
            outdated: p.outdated.unwrap_or(false),
            trust: verdict_for_summary(p),
            upstream: p.upstream.map(|u| u.to_string()),
        })
        .collect();
    rows.sort_by(|a, b| (&a.namespace, &a.type_).cmp(&(&b.namespace, &b.type_)));
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
pub fn resolve_client_and_org(cli: &Cli) -> Result<(Client, String), Box<dyn std::error::Error>> {
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
        // Multiple memberships: offer an fzf-style picker on a TTY; otherwise
        // (CI/piped) stay deterministic and require --org.
        Ok(memberships) => match prompt_org(&memberships)? {
            Some(org) => {
                eprintln!("using org '{org}'");
                Ok((client, org))
            }
            None => Err(Box::new(AmbiguousOrg)),
        },
        // CI OIDC/bot tokens have no org endpoint (401/403/404) — require --org.
        Err(e) if matches!(e.status(), Some(401) | Some(403) | Some(404)) => {
            Err(Box::new(MissingOrg))
        }
        // Anything else (5xx, transport) is a real API failure — surface it
        // rather than masking it as "no org configured".
        Err(e) => Err(Box::new(e)),
    }
}

/// An authenticated client for the `init` wizard, or `None`.
///
/// Unlike every other caller, `init` must work for an unauthenticated user: the
/// scaffold itself needs no network, and only the org picker and the VCS-trust
/// offer do. So a missing token is not an error here — it just means those two
/// steps fall back to flags and printed instructions.
pub fn client_for_wizard(cli: &Cli) -> Option<Client> {
    let ctx = auth::auth_context(cli).ok()?;
    auth::api_client(&ctx).ok()
}

/// Resolve `(client, org)` or print the error and return the mapped exit code:
/// auth failures → 5, API failures → their §9 code, missing/ambiguous org → 2.
/// Shared by every command that needs an authed client scoped to an org.
pub fn client_and_org(cli: &Cli) -> Result<(Client, String), i32> {
    resolve_client_and_org(cli).map_err(|e| {
        eprintln!("error: {e}");
        if let Some(auth) = e.downcast_ref::<tm_auth::AuthError>() {
            return auth.exit_code();
        }
        if let Some(api) = e.downcast_ref::<tm_api::ApiError>() {
            return api.exit_code();
        }
        EXIT_MISSING_ORG
    })
}

/// Interactive org picker for the multi-membership case — a fuzzy, type-to-filter
/// select (`inquire`), rendered on stderr so stdout stays clean for `-o json`.
///
/// Returns `Ok(None)` when stdin/stderr isn't a TTY (CI/piped) so the caller
/// falls back to the deterministic "pass --org" error, and also when the user
/// cancels (Esc/Ctrl-C) — a cancel is a no-selection, not a hard failure.
fn prompt_org(memberships: &[OrgMembership]) -> Result<Option<String>, Box<dyn std::error::Error>> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Ok(None);
    }
    let options: Vec<String> = memberships.iter().map(|m| m.slug.clone()).collect();
    match inquire::Select::new("Select org", options).prompt() {
        Ok(choice) => Ok(Some(choice)),
        Err(inquire::InquireError::OperationCanceled)
        | Err(inquire::InquireError::OperationInterrupted) => Ok(None),
        Err(e) => Err(Box::new(e)),
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
pub const EXIT_MISSING_ORG: i32 = 2;

// ── dispatch ───────────────────────────────────────────────────────────────────

pub fn providers(command: &ProvidersCommand, cli: &Cli) -> CmdResult {
    match command {
        ProvidersCommand::Ls { at_risk } => providers_ls(cli, *at_risk),
        ProvidersCommand::Show { provider, upstream } => {
            // clap validates the value, so a parse failure here would be a bug
            // in the value_parser list rather than user input.
            let lineage = match upstream.as_deref().map(str::parse::<ProviderUpstream>) {
                None => None,
                Some(Ok(u)) => Some(u),
                Some(Err(_)) => {
                    eprintln!(
                        "error: unknown upstream '{}'",
                        upstream.as_deref().unwrap_or("")
                    );
                    return Ok(2);
                }
            };
            providers_show(cli, provider, lineage)
        }
    }
}

pub fn modules(command: &ModulesCommand, cli: &Cli) -> CmdResult {
    match command {
        ModulesCommand::Search { query, limit, all } => modules_search(cli, query, *limit, *all),
        ModulesCommand::Show { module } => {
            // `<ns>/<name>/<provider>` optionally suffixed `@<version>`. Module
            // detail is per version, so without a suffix the newest is used.
            let (addr, version) = match module.split_once('@') {
                Some((addr, v)) if !v.is_empty() => (addr, Some(v)),
                Some((addr, _)) => (addr, None),
                None => (module.as_str(), None),
            };
            modules_show(cli, addr, version)
        }
        ModulesCommand::Changed { all } => crate::publish::changed(cli, *all),
        ModulesCommand::Publish(args) => crate::publish::publish(cli, args),
    }
}

// ── providers ls ────────────────────────────────────────────────────────────────

fn providers_ls(cli: &Cli, at_risk_only: bool) -> CmdResult {
    let (client, org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };

    // One request where there were two: the unified catalog carries the usage
    // rollup AND the severity counts the removed `providers-overview` provided.
    let providers = match client.providers_list(&org) {
        Ok(p) => p,
        Err(e) => return Ok(api_fail(&e)),
    };

    let mut rows = summary_rows(&providers);
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
    // An org can manage the same `namespace/type` from both the terraform.io and
    // opentofu.org lineages, and `acme-demo` does. Without the UPSTREAM column
    // those are two identical-looking rows, so it is shown whenever any provider
    // is ambiguous — not only in `-o wide`. A narrow table is worth keeping, but
    // not at the price of two rows a reader cannot tell apart.
    let ambiguous = has_duplicate_addresses(rows);
    let show_upstream = wide || ambiguous;

    let mut headers = vec!["namespace", "type"];
    if show_upstream {
        headers.push("upstream");
    }
    headers.extend(["constraint", "latest", "workspaces", "outdated", "trust"]);
    if wide {
        headers.push("latest_version");
    }

    let mut view = TableView::new(headers);
    for r in rows {
        let mut cells = vec![r.namespace.clone(), r.type_.clone()];
        if show_upstream {
            cells.push(r.upstream.clone().unwrap_or_else(|| DASH.to_string()));
        }
        cells.extend([
            r.constraint.clone().unwrap_or_else(|| DASH.to_string()),
            r.latest.clone().unwrap_or_else(|| DASH.to_string()),
            r.workspaces.to_string(),
            output::outdated_glyph(r.outdated, style),
            r.trust.render(style),
        ]);
        if wide {
            cells.push(r.latest_version.clone().unwrap_or_else(|| DASH.to_string()));
        }
        view.row(cells);
    }
    format!("{}\n", view.render())
}

/// Whether any `namespace/type` appears more than once, i.e. is served from more
/// than one upstream lineage.
fn has_duplicate_addresses(rows: &[ProviderRow]) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    rows.iter()
        .any(|r| !seen.insert((r.namespace.as_str(), r.type_.as_str())))
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

/// Resolve `namespace/type` to a single catalog artefact, which needs the
/// `{upstream}` segment the removed detail route did not have.
///
/// One org can legitimately manage both the `terraform.io` and `opentofu.org`
/// lineages of the same `namespace/type`, so this can be genuinely ambiguous.
/// When it is, the ambiguity is reported with both choices rather than guessed
/// at: picking one would show a user the wrong supply chain and look right.
fn resolve_artefact<'a>(
    providers: &'a [ProviderArtefactSummary],
    ns: &str,
    ty: &str,
    wanted: Option<ProviderUpstream>,
) -> Result<&'a ProviderArtefactSummary, String> {
    let matches: Vec<&ProviderArtefactSummary> = providers
        .iter()
        .filter(|p| p.namespace == ns && p.type_ == ty)
        .filter(|p| wanted.is_none_or(|w| p.upstream == Some(w)))
        .collect();

    match matches.as_slice() {
        [] if wanted.is_some() => Err(format!("org does not manage {ns}/{ty} from that upstream")),
        [] => Err(format!("org does not manage {ns}/{ty}")),
        [only] => Ok(only),
        several => {
            let choices: Vec<String> = several
                .iter()
                .filter_map(|p| p.upstream.map(|u| u.to_string()))
                .collect();
            Err(format!(
                "{ns}/{ty} is managed from more than one upstream ({}). \
                 Re-run with --upstream <{}> to say which.",
                choices.join(", "),
                choices.join("|")
            ))
        }
    }
}

fn providers_show(cli: &Cli, provider: &str, upstream: Option<ProviderUpstream>) -> CmdResult {
    let (ns, ty) = match split_two(provider, "<ns>/<type>") {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(2);
        }
    };

    let (client, org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };

    // The catalog list first, to resolve `{upstream}` for the detail route.
    let providers = match client.providers_list(&org) {
        Ok(p) => p,
        Err(e) => return Ok(api_fail(&e)),
    };
    let summary = match resolve_artefact(&providers, &ns, &ty, upstream) {
        Ok(s) => s,
        Err(msg) => {
            eprintln!("error: {msg}");
            return Ok(2);
        }
    };
    let Some(lineage) = summary.upstream else {
        eprintln!("error: catalog row for {ns}/{ty} has no upstream, so it cannot be addressed");
        return Ok(1);
    };

    let detail = match client.provider_detail(&org, lineage, &ns, &ty) {
        Ok(d) => d,
        Err(e) => return Ok(api_fail(&e)),
    };

    // /v1/providers/{org}/.../versions is authRequired + org-allowlist-filtered
    // (§7 S4). A 401/403 there → exit 5 with a clear message.
    let available = match client.provider_versions(&org, &ns, &ty) {
        // `version` is optional in the protocol schema; a row without one names
        // nothing, so it is dropped rather than rendered as a blank line.
        Ok(v) => v
            .versions
            .into_iter()
            .filter_map(|v| v.version)
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

    let show = build_show(
        &ns,
        &ty,
        Some(lineage.to_string()),
        summary.source_url.clone(),
        verdict_for_summary(summary),
        &detail,
        &available,
    );

    let format = cli.global.output.unwrap_or_default();
    if output::print_structured(&show, format)? {
        return Ok(0);
    }

    let style = Style::detect(cli.global.no_color);
    print!("{}", render_providers_show(&show, style));
    Ok(0)
}

/// Derive a single version's [`TrustVerdict`] from its own scan row.
///
/// This is strictly better than what `providers show` could do before. The old
/// provider-level overview gave every version of a provider the SAME verdict —
/// the doc comment on the removed field said so — which meant a clean 5.4.1 and
/// a critical-carrying 5.0.0 rendered identically. The detail route reports per
/// version, including an explicit `scanned` flag, so "not scanned" is a real
/// answer rather than an absence read as zero findings.
pub fn verdict_for_version(v: &ProviderVersionRow) -> TrustVerdict {
    if v.scanned == Some(false) {
        return TrustVerdict::Unscanned;
    }
    let bad = v.critical_count.unwrap_or(0) > 0
        || v.high_count.unwrap_or(0) > 0
        || v.vuln_count.unwrap_or(0) > 0;
    if bad {
        TrustVerdict::AtRisk
    } else if v.scanned == Some(true) {
        TrustVerdict::Trusted
    } else {
        // No `scanned` flag at all: the server did not say, so neither do we.
        TrustVerdict::Unscanned
    }
}

/// Assemble the [`ProviderShow`] payload from the artefact detail + the
/// allowlist-filtered available versions (pure — used by golden tests).
///
/// Versions carry the workspaces that lock them, and each version's trust is its
/// OWN verdict rather than one provider-level verdict repeated.
pub fn build_show(
    ns: &str,
    ty: &str,
    upstream: Option<String>,
    repo: Option<String>,
    provider_trust: TrustVerdict,
    detail: &ProviderArtefactDetail,
    available: &[String],
) -> ProviderShow {
    // Rows the detail reports, keyed by version.
    let mut rows: BTreeMap<String, ShowVersionRow> = BTreeMap::new();
    for v in &detail.versions {
        let mut workspaces: Vec<String> = v
            .used_by
            .iter()
            .filter_map(|u| u.workspace_name.clone())
            .collect();
        workspaces.sort();
        workspaces.dedup();
        rows.insert(
            v.version.clone(),
            ShowVersionRow {
                version: v.version.clone(),
                trust: verdict_for_version(v),
                workspaces,
            },
        );
    }

    // A version the registry serves but the detail does not list (the two apply
    // different visibility rules) still shows, with no verdict to report.
    for v in available {
        rows.entry(v.clone()).or_insert_with(|| ShowVersionRow {
            version: v.clone(),
            trust: TrustVerdict::Unscanned,
            workspaces: Vec::new(),
        });
    }

    let mut rows: Vec<ShowVersionRow> = rows.into_values().collect();
    // Newest first, semver-aware (lexical puts 5.9.0 above 5.10.0); unparsable
    // versions sort below parsable ones, lexically among themselves.
    rows.sort_by(|a, b| semver_cmp(&b.version, &a.version));

    ProviderShow {
        namespace: ns.to_string(),
        type_: ty.to_string(),
        upstream,
        repo,
        trust: provider_trust,
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
    let (client, _org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
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
        match resp.meta.as_ref().and_then(|m| m.next_offset) {
            // A non-advancing cursor would loop forever — treat it as the end.
            Some(next) if all && next > offset as i64 => {
                if hits.len() >= SEARCH_ALL_CAP {
                    eprintln!(
                        "note: stopped at {SEARCH_ALL_CAP} results (more available; narrow the query)"
                    );
                    break;
                }
                offset = next.max(0) as u64;
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
        view.row([
            m.namespace.clone().unwrap_or_else(|| DASH.to_string()),
            m.name.clone().unwrap_or_else(|| DASH.to_string()),
            m.version.clone().unwrap_or_else(|| DASH.to_string()),
        ]);
    }
    format!("{}\n", view.render())
}

// ── modules show ────────────────────────────────────────────────────────────────

/// The `modules show` json/yaml payload.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleShow {
    #[serde(flatten)]
    pub detail: ModuleVersionDetail,
    pub versions: Vec<String>,
}

fn modules_show(cli: &Cli, module: &str, wanted_version: Option<&str>) -> CmdResult {
    let (ns, name, provider) = match split_three(module) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(2);
        }
    };

    let (client, _org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };

    // Versions FIRST, because detail is per version and the old call requested a
    // versionless path the worker does not route.
    let mut versions = match client.module_versions(&ns, &name, &provider) {
        Ok(v) => v
            .modules
            .into_iter()
            .flat_map(|m| m.versions.into_iter().filter_map(|r| r.version))
            .collect::<Vec<_>>(),
        Err(e) => return Ok(api_fail(&e)),
    };
    versions.sort_by(|a, b| semver_cmp(b, a));
    versions.dedup();

    // Which version to describe: the one asked for, else the newest published.
    let target = match wanted_version {
        Some(v) if !versions.iter().any(|known| known == v) => {
            eprintln!(
                "error: {ns}/{name}/{provider} has no version {v}. Known versions: {}",
                if versions.is_empty() {
                    "none".to_string()
                } else {
                    versions.join(", ")
                }
            );
            return Ok(6);
        }
        Some(v) => v.to_string(),
        None => match versions.first() {
            Some(v) => v.clone(),
            None => {
                eprintln!("error: {ns}/{name}/{provider} has no published versions");
                return Ok(6);
            }
        },
    };

    let detail = match client.module_version(&ns, &name, &provider, &target) {
        Ok(d) => d,
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
    let mut out = format!(
        "{}/{}\n",
        d.namespace.as_deref().unwrap_or(DASH),
        d.name.as_deref().unwrap_or(DASH)
    );
    if let Some(desc) = &d.description {
        if !desc.is_empty() {
            out.push_str(&format!("{desc}\n"));
        }
    }
    out.push_str(&format!(
        "version  {}\n",
        d.version.as_deref().unwrap_or(DASH)
    ));
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
pub fn api_fail(e: &tm_api::ApiError) -> i32 {
    eprintln!("error: {e}");
    e.exit_code()
}

/// Compare two version strings semver-first: both parse → semver order; a
/// parsable version ranks above an unparsable one; neither parses → lexical.
fn semver_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(va), Ok(vb)) => va.cmp(&vb),
        (Ok(_), Err(_)) => std::cmp::Ordering::Greater,
        (Err(_), Ok(_)) => std::cmp::Ordering::Less,
        (Err(_), Err(_)) => a.cmp(b),
    }
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

    // ── verdict_from_scan: the posture path's verdict source ──────────────────

    /// Built from JSON: the generated rollup has optional counts, and going via
    /// serde is the path a real response takes.
    fn rollup(crit: i64, high: i64, medium: i64) -> VersionScanRollup {
        serde_json::from_value(serde_json::json!({
            "critical": crit, "high": high, "medium": medium, "low": 0,
        }))
        .expect("rollup parses")
    }

    /// The regression that mattered. The removed overview endpoint returned 410,
    /// both gate call sites swallowed it with `unwrap_or_default()`, and an empty
    /// overview scored every provider `Unscanned` — so `is_at_risk()` was false
    /// everywhere and `--fail-on-atrisk` never tripped. A provider whose locked
    /// version really does carry a critical must read `AtRisk`.
    #[test]
    fn at_risk_rollup_is_not_mistaken_for_unscanned() {
        assert_eq!(
            verdict_from_scan(Some(&rollup(1, 0, 0))),
            TrustVerdict::AtRisk
        );
        assert_eq!(
            verdict_from_scan(Some(&rollup(0, 1, 0))),
            TrustVerdict::AtRisk
        );
    }

    /// `None` (not scanned) and an all-zero rollup (scanned, clean) are different
    /// answers and must not collapse into one verdict.
    #[test]
    fn absent_rollup_is_unscanned_but_empty_rollup_is_trusted() {
        assert_eq!(verdict_from_scan(None), TrustVerdict::Unscanned);
        assert_eq!(
            verdict_from_scan(Some(&VersionScanRollup::default())),
            TrustVerdict::Trusted
        );
    }

    /// Mediums and lows are reported but do not gate, matching `--at-risk`.
    #[test]
    fn medium_only_rollup_is_trusted() {
        assert_eq!(
            verdict_from_scan(Some(&rollup(0, 0, 25))),
            TrustVerdict::Trusted
        );
    }

    /// A catalog row, built from JSON: `ProviderArtefactSummary` comes from the
    /// spec and gains columns as the server documents them, so a struct literal
    /// would need editing for fields these tests do not read.
    fn summary(ns: &str, ty: &str, crit: i64, high: i64, vuln: i64) -> ProviderArtefactSummary {
        serde_json::from_value(serde_json::json!({
            "id": format!("{ns}/{ty}"),
            "namespace": ns,
            "type": ty,
            "upstream": "terraform.io",
            "serve_mode": "serve",
            "version_constraint": "~> 5.0",
            "latest_version": "5.4.1",
            "upstream_latest_version": "5.5.0",
            "usage_workspace_count": 3,
            "outdated": false,
            "critical_count": crit,
            "high_count": high,
            "vuln_count": vuln,
        }))
        .expect("catalog row parses")
    }

    /// A catalog row holding no versions at all — distinct from one holding a
    /// clean version.
    fn summary_without_versions(ns: &str, ty: &str) -> ProviderArtefactSummary {
        serde_json::from_value(serde_json::json!({
            "id": format!("{ns}/{ty}"),
            "namespace": ns, "type": ty, "upstream": "opentofu.org",
            "serve_mode": "serve", "version_constraint": null,
        }))
        .expect("catalog row parses")
    }

    fn detail(versions: serde_json::Value) -> ProviderArtefactDetail {
        serde_json::from_value(serde_json::json!({
            "namespace": "hashicorp", "type": "aws", "upstream": "terraform.io",
            "versions": versions,
        }))
        .expect("artefact detail parses")
    }

    // ── provider-level verdict from the catalog row ───────────────────────────

    #[test]
    fn summary_with_high_is_at_risk() {
        assert_eq!(
            verdict_for_summary(&summary("hashicorp", "aws", 0, 2, 2)),
            TrustVerdict::AtRisk
        );
    }

    #[test]
    fn summary_with_critical_is_at_risk() {
        assert_eq!(
            verdict_for_summary(&summary("hashicorp", "aws", 1, 0, 1)),
            TrustVerdict::AtRisk
        );
    }

    #[test]
    fn summary_with_vuln_only_is_at_risk() {
        assert_eq!(
            verdict_for_summary(&summary("hashicorp", "aws", 0, 0, 4)),
            TrustVerdict::AtRisk
        );
    }

    #[test]
    fn summary_all_zero_is_trusted() {
        assert_eq!(
            verdict_for_summary(&summary("hashicorp", "random", 0, 0, 0)),
            TrustVerdict::Trusted
        );
    }

    /// A provider with no version at all is `Unscanned`, not `Trusted`: its zero
    /// counts are the absence of a subject, not a clean result.
    #[test]
    fn provider_without_versions_is_unscanned_not_trusted() {
        assert_eq!(
            verdict_for_summary(&summary_without_versions("opentofu", "null")),
            TrustVerdict::Unscanned
        );
    }

    // ── per-version verdict from the detail row ───────────────────────────────

    fn version_row(v: serde_json::Value) -> ProviderVersionRow {
        serde_json::from_value(v).expect("version row parses")
    }

    /// The capability the old provider-level overview could not express: two
    /// versions of one provider with different verdicts.
    #[test]
    fn versions_get_their_own_verdicts() {
        let bad = version_row(serde_json::json!({
            "version": "5.0.0", "scanned": true, "critical_count": 1,
        }));
        let good = version_row(serde_json::json!({
            "version": "5.4.1", "scanned": true, "critical_count": 0, "high_count": 0,
        }));
        assert_eq!(verdict_for_version(&bad), TrustVerdict::AtRisk);
        assert_eq!(verdict_for_version(&good), TrustVerdict::Trusted);
    }

    /// `scanned: false` is a real answer and must never read as "no findings".
    #[test]
    fn unscanned_version_is_not_trusted() {
        let v = version_row(serde_json::json!({ "version": "5.1.0", "scanned": false }));
        assert_eq!(verdict_for_version(&v), TrustVerdict::Unscanned);
        // …and so is a row that does not say either way.
        let silent = version_row(serde_json::json!({ "version": "5.2.0" }));
        assert_eq!(verdict_for_version(&silent), TrustVerdict::Unscanned);
    }

    // ── providers ls rows ─────────────────────────────────────────────────────

    #[test]
    fn one_row_per_provider() {
        let rows = summary_rows(&[summary("hashicorp", "aws", 0, 1, 1)]);
        assert_eq!(rows.len(), 1, "one row per provider, not per version");
        assert_eq!(rows[0].namespace, "hashicorp");
        assert_eq!(rows[0].type_, "aws");
        assert_eq!(rows[0].constraint.as_deref(), Some("~> 5.0"));
        assert_eq!(rows[0].latest.as_deref(), Some("5.4.1"));
        assert_eq!(rows[0].latest_version.as_deref(), Some("5.5.0"));
        assert_eq!(rows[0].workspaces, 3);
        assert_eq!(rows[0].trust, TrustVerdict::AtRisk);
        assert_eq!(rows[0].upstream.as_deref(), Some("terraform.io"));
    }

    #[test]
    fn rows_sorted_across_providers() {
        let rows = summary_rows(&[
            summary("opentofu", "null", 0, 0, 0),
            summary("hashicorp", "aws", 0, 0, 0),
            summary_without_versions("hashicorp", "random"),
        ]);
        let keys: Vec<String> = rows
            .iter()
            .map(|r| format!("{}/{}", r.namespace, r.type_))
            .collect();
        assert_eq!(
            keys,
            vec!["hashicorp/aws", "hashicorp/random", "opentofu/null"]
        );
    }

    /// An empty constraint means "every version" and renders as absent rather
    /// than as an empty cell.
    #[test]
    fn empty_constraint_reads_as_none() {
        let p: ProviderArtefactSummary = serde_json::from_value(serde_json::json!({
            "id": "x", "namespace": "n", "type": "t", "upstream": "terraform.io",
            "serve_mode": "serve", "version_constraint": "",
        }))
        .unwrap();
        assert_eq!(summary_rows(&[p])[0].constraint, None);
    }

    #[test]
    fn provider_rows_json_round_trips() {
        let rows = summary_rows(&[summary("hashicorp", "aws", 0, 0, 0)]);
        let json = serde_json::to_string(&rows).unwrap();
        assert!(json.contains("\"namespace\":\"hashicorp\""), "{json}");
        assert!(json.contains("\"type\":\"aws\""), "{json}");
    }

    // ── upstream resolution ──────────────────────────────────────────────────

    /// One org can manage both lineages of the same provider. Guessing would
    /// show the wrong supply chain and look right, so it refuses and names both.
    #[test]
    fn ambiguous_upstream_is_refused_naming_both() {
        let tf = summary("hashicorp", "aws", 0, 0, 0);
        let tofu: ProviderArtefactSummary = serde_json::from_value(serde_json::json!({
            "id": "2", "namespace": "hashicorp", "type": "aws",
            "upstream": "opentofu.org", "serve_mode": "serve", "version_constraint": null,
        }))
        .unwrap();
        let err = resolve_artefact(&[tf, tofu], "hashicorp", "aws", None)
            .expect_err("two lineages must not resolve");
        assert!(err.contains("terraform.io"), "{err}");
        assert!(err.contains("opentofu.org"), "{err}");
        assert!(err.contains("--upstream"), "{err}");
    }

    /// …and naming the lineage resolves it.
    #[test]
    fn explicit_upstream_breaks_the_tie() {
        let tf = summary("hashicorp", "aws", 0, 0, 0);
        let tofu: ProviderArtefactSummary = serde_json::from_value(serde_json::json!({
            "id": "2", "namespace": "hashicorp", "type": "aws",
            "upstream": "opentofu.org", "serve_mode": "serve", "version_constraint": null,
        }))
        .unwrap();
        let both = [tf, tofu];
        let got = resolve_artefact(
            &both,
            "hashicorp",
            "aws",
            Some(ProviderUpstream::OpentofuOrg),
        )
        .expect("explicit upstream resolves");
        assert_eq!(got.upstream, Some(ProviderUpstream::OpentofuOrg));
    }

    #[test]
    fn unmanaged_provider_is_reported_not_guessed() {
        let err = resolve_artefact(&[], "ghost", "gone", None).expect_err("nothing to resolve");
        assert!(err.contains("ghost/gone"), "{err}");
    }

    #[test]
    fn single_match_resolves_without_a_flag() {
        let only = summary("hashicorp", "aws", 0, 0, 0);
        assert!(resolve_artefact(&[only], "hashicorp", "aws", None).is_ok());
    }

    // ── providers show ───────────────────────────────────────────────────────

    /// A locked version the detail lists, with its workspaces, plus a version the
    /// registry serves that the detail does not list.
    #[test]
    fn show_lists_versions_with_their_workspaces() {
        let d = detail(serde_json::json!([
            {
                "version": "5.2.0", "scanned": true, "critical_count": 1,
                "used_by": [
                    {"workspace_id": "w1", "workspace_name": "prod", "locked_at": 1},
                    {"workspace_id": "w2", "workspace_name": "staging", "locked_at": 2},
                ],
            },
            { "version": "5.4.1", "scanned": true, "used_by": [] },
        ]));
        let show = build_show(
            "hashicorp",
            "aws",
            Some("terraform.io".into()),
            None,
            TrustVerdict::AtRisk,
            &d,
            &["5.5.0".to_string()],
        );
        // Newest first, semver-aware.
        let versions: Vec<&str> = show.versions.iter().map(|v| v.version.as_str()).collect();
        assert_eq!(versions, vec!["5.5.0", "5.4.1", "5.2.0"]);

        let locked = show.versions.iter().find(|v| v.version == "5.2.0").unwrap();
        assert_eq!(locked.workspaces, vec!["prod", "staging"]);
        assert_eq!(locked.trust, TrustVerdict::AtRisk);

        // Per-version verdicts differ within one provider — the thing the old
        // provider-level overview could not express.
        let clean = show.versions.iter().find(|v| v.version == "5.4.1").unwrap();
        assert_eq!(clean.trust, TrustVerdict::Trusted);

        // Served but not in the detail: listed, with nothing claimed about it.
        let extra = show.versions.iter().find(|v| v.version == "5.5.0").unwrap();
        assert_eq!(extra.trust, TrustVerdict::Unscanned);
        assert!(extra.workspaces.is_empty());
    }

    #[test]
    fn provider_show_json_round_trips() {
        let d = detail(serde_json::json!([
            { "version": "5.4.1", "scanned": true, "used_by": [] },
        ]));
        let show = build_show(
            "hashicorp",
            "aws",
            Some("terraform.io".into()),
            Some("https://github.com/hashicorp/terraform-provider-aws".into()),
            TrustVerdict::Trusted,
            &d,
            &[],
        );
        let json = serde_json::to_string(&show).unwrap();
        assert!(json.contains("\"upstream\":\"terraform.io\""), "{json}");
        assert!(json.contains("5.4.1"), "{json}");
    }

    #[test]
    fn show_renders_workspaces_and_the_at_risk_summary() {
        let d = detail(serde_json::json!([{
            "version": "5.2.0", "scanned": true, "high_count": 2,
            "used_by": [
                {"workspace_id": "w1", "workspace_name": "prod", "locked_at": 0},
                {"workspace_id": "w2", "workspace_name": "staging", "locked_at": 0},
            ],
        }]));
        let show = build_show(
            "hashicorp",
            "aws",
            None,
            None,
            TrustVerdict::AtRisk,
            &d,
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
            &detail(serde_json::json!([])),
            &[],
        );
        let out = render_providers_show(&show, Style::plain());
        assert!(out.contains("Not in any lock file."), "{out}");
    }

    // ── ls render (golden-ish, plain) ────────────────────────────────────────

    /// Two lineages of one provider must not render as two identical rows: the
    /// UPSTREAM column appears without `-o wide` when any address is ambiguous.
    #[test]
    fn ls_shows_upstream_when_a_provider_has_two_lineages() {
        let tf = summary("hashicorp", "http", 0, 0, 0);
        let tofu: ProviderArtefactSummary = serde_json::from_value(serde_json::json!({
            "id": "2", "namespace": "hashicorp", "type": "http",
            "upstream": "opentofu.org", "serve_mode": "serve", "version_constraint": null,
        }))
        .unwrap();
        let rows = summary_rows(&[tf, tofu]);
        let out = render_providers_ls(&rows, false, Style::plain());
        assert!(out.contains("UPSTREAM"), "{out}");
        assert!(out.contains("terraform.io"), "{out}");
        assert!(out.contains("opentofu.org"), "{out}");
    }

    /// …and stays out of the way when nothing is ambiguous.
    #[test]
    fn ls_hides_upstream_when_every_provider_is_unique() {
        let rows = summary_rows(&[
            summary("hashicorp", "aws", 0, 0, 0),
            summary("hashicorp", "random", 0, 0, 0),
        ]);
        let out = render_providers_ls(&rows, false, Style::plain());
        assert!(!out.contains("UPSTREAM"), "{out}");
    }

    #[test]
    fn ls_plain_render_has_headers_and_rows() {
        let rows = summary_rows(&[
            summary("hashicorp", "aws", 0, 2, 0),
            summary("hashicorp", "random", 0, 0, 0),
        ]);
        let out = render_providers_ls(&rows, false, Style::plain());
        assert!(out.contains("NAMESPACE"));
        assert!(out.contains("CONSTRAINT"), "the per-version column is gone");
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
