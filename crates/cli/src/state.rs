//! State commands — `state ls/versions/promote/rollback/unlock` (SPEC §6–§9).
//!
//! Reads (`ls`, `versions`) render borderless tables or `-o json`. Mutations
//! (`promote`, `rollback`, `unlock`) confirm the concrete blast radius (§8) via
//! the shared [`crate::confirm`] helper, then call the endpoint, mapping the
//! worker `{error}` string to the §9 exit code. Every promote/rollback carries a
//! fresh UUID `Idempotency-Key` (rubric 6).
//!
//! The client is built once via [`crate::discovery::resolve_client_and_org`]
//! (which itself uses `tm_auth::client` — never an `HttpClient` clone). Serial→
//! version resolution and the versions-table rendering are pure functions so they
//! are unit-tested network/TTY/sleep-free (rubrics 3, 4).

use serde::Serialize;
use tm_api::{ApiError, Client, LockInfo, PromoteResponse, StateVersion, Workspace};

use crate::cli::{Cli, StateCommand};
use crate::commands::CmdResult;
use crate::confirm::{confirm, EXIT_CONFIRM};
use crate::discovery::{resolve_client_and_org, EXIT_MISSING_ORG};
use crate::output::{self, relative_time, Style, TableView, DASH};

// ── dispatch ────────────────────────────────────────────────────────────────

pub fn state(command: &StateCommand, cli: &Cli) -> CmdResult {
    match command {
        StateCommand::Ls => state_ls(cli),
        StateCommand::Versions { workspace } => state_versions(cli, workspace),
        StateCommand::Promote {
            workspace,
            version_id,
            yes,
        } => state_promote(cli, workspace, version_id, *yes),
        StateCommand::Rollback { workspace, to, yes } => {
            state_rollback(cli, workspace, to.map(|s| s as i64), *yes)
        }
        StateCommand::Unlock { workspace, yes } => state_unlock(cli, workspace, *yes),
    }
}

/// Resolve `(client, org)` or print the error and return the exit code. Shared
/// entry for every state command.
fn client_and_org(cli: &Cli) -> Result<(Client, String), i32> {
    resolve_client_and_org(cli).map_err(|e| {
        eprintln!("error: {e}");
        EXIT_MISSING_ORG
    })
}

/// Map an API error to its §9 exit code, printing the preserved message first.
fn api_fail(e: &ApiError) -> i32 {
    eprintln!("error: {e}");
    e.exit_code()
}

// ── state ls ────────────────────────────────────────────────────────────────

fn state_ls(cli: &Cli) -> CmdResult {
    let (client, org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let workspaces = match client.state_list(&org) {
        Ok(w) => w,
        Err(e) => return Ok(api_fail(&e)),
    };

    let format = cli.global.output.unwrap_or_default();
    if output::print_structured(&workspaces, format)? {
        return Ok(0);
    }
    print!("{}", render_ls(&workspaces));
    Ok(0)
}

/// Render the `state ls` table (pure). Columns: WORKSPACE, SERIAL (latest),
/// RESOURCES, PUSHED (relative). Empty cells show `—`.
pub fn render_ls(workspaces: &[Workspace]) -> String {
    let mut view = TableView::new(["workspace", "serial", "resources", "pushed"]);
    for w in workspaces {
        view.row([
            w.name.clone(),
            w.latest_serial.map(|s| s.to_string()).unwrap_or_else(dash),
            w.resource_count.map(|c| c.to_string()).unwrap_or_else(dash),
            w.pushed_at.map(relative_time).unwrap_or_else(dash),
        ]);
    }
    format!("{}\n", view.render())
}

// ── state versions ──────────────────────────────────────────────────────────

/// One rendered `state versions` row (also the `-o json` shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionRow {
    pub serial: i64,
    /// `true` for the highest serial (marked `*` / "current" in the table).
    pub current: bool,
    /// `actor_type:actor_name`, e.g. `vcs:ci-deploy`.
    pub actor: String,
    pub pushed_at: i64,
    pub resources: u64,
    /// `promoted from <serial>` when this version is a restore-copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The version id (for `-o json`; the promote/rollback caller needs it).
    pub id: String,
}

/// Format one version's ACTOR cell as `actor_type:actor_name` (§6). Missing parts
/// degrade gracefully: `vcs:—`, `—:rhys`, or `—` when both are absent.
fn actor_label(v: &StateVersion) -> String {
    match (v.actor_type.as_deref(), v.actor_name.as_deref()) {
        (None, None) => DASH.to_string(),
        (ty, name) => format!("{}:{}", ty.unwrap_or(DASH), name.unwrap_or(DASH)),
    }
}

/// Build the render rows from raw versions (pure — rubric 3). The highest serial
/// is flagged `current`; NOTE is derived from `promoted_from_serial`.
pub fn build_version_rows(versions: &[StateVersion]) -> Vec<VersionRow> {
    let max_serial = versions.iter().map(|v| v.serial).max();
    let mut rows: Vec<VersionRow> = versions
        .iter()
        .map(|v| VersionRow {
            serial: v.serial,
            current: Some(v.serial) == max_serial,
            actor: actor_label(v),
            pushed_at: v.pushed_at,
            resources: v.resource_count,
            note: v.promoted_from_serial.map(|s| format!("promoted from {s}")),
            id: v.id.clone(),
        })
        .collect();
    // Newest first — highest serial at the top (matches the §6 mockup).
    rows.sort_by_key(|r| std::cmp::Reverse(r.serial));
    rows
}

/// Render the `state versions` table from prepared rows (pure — rubric 3).
/// The current row's SERIAL is marked `*` and NOTE reads "current".
pub fn render_versions(rows: &[VersionRow]) -> String {
    let mut view = TableView::new(["serial", "actor", "pushed", "resources", "note"]);
    for r in rows {
        let serial = if r.current {
            format!("{} *", r.serial)
        } else {
            r.serial.to_string()
        };
        let note = if r.current {
            "current".to_string()
        } else {
            r.note.clone().unwrap_or_else(dash)
        };
        view.row([
            serial,
            r.actor.clone(),
            relative_time(r.pushed_at),
            r.resources.to_string(),
            note,
        ]);
    }
    format!("{}\n", view.render())
}

fn state_versions(cli: &Cli, workspace: &str) -> CmdResult {
    let (client, org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    // §7: default first page; the whole history is small enough to page for a
    // faithful "current" marking, so always follow pagination for the table.
    let versions = match client.state_versions_all(&org, workspace) {
        Ok(v) => v,
        Err(e) => return Ok(api_fail(&e)),
    };
    let rows = build_version_rows(&versions);

    let format = cli.global.output.unwrap_or_default();
    if output::print_structured(&rows, format)? {
        return Ok(0);
    }
    print!("{}", render_versions(&rows));
    Ok(0)
}

// ── promote ─────────────────────────────────────────────────────────────────

/// Run a promote of `version` (already resolved) after confirming its blast
/// radius. Shared by `promote` and `rollback` (same machinery + §9 mapping).
fn do_promote(
    client: &Client,
    org: &str,
    workspace: &str,
    version: &StateVersion,
    prompt: &str,
    assume_yes: bool,
    style: Style,
) -> i32 {
    match confirm(prompt, assume_yes, style) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("aborted");
            return EXIT_CONFIRM;
        }
        Err(code) => return code,
    }

    // Fresh UUID Idempotency-Key per invocation (rubric 6).
    let key = uuid::Uuid::new_v4().to_string();
    match client.state_promote(org, workspace, &version.id, &key) {
        Ok(resp) => {
            print_promote_result(&resp, version.serial);
            0
        }
        Err(e) => promote_error_exit(&e),
    }
}

/// Map a promote/rollback API error to its §9 exit code, printing an actionable
/// diagnostic (§8: role/human requirement, lock holder, feature-off).
fn promote_error_exit(e: &ApiError) -> i32 {
    match (e.status(), e.error_code()) {
        // Feature kill-switch arrives as a 404 `feature_disabled` → exit 6.
        (_, Some("feature_disabled")) => {
            eprintln!("error: state promote is not enabled for this org");
            6
        }
        // Server authz: admin + human required. --force never bypasses this.
        (_, Some("forbidden" | "unauthorized" | "invalid_token")) => {
            eprintln!(
                "error: {} (promote requires the admin role and a human token — a bot/OIDC token is rejected)",
                e.message().unwrap_or("forbidden")
            );
            5
        }
        // The copy takes a workspace lock; a conflict names the holder (§9 note).
        (_, Some("locked" | "serial_conflict")) => {
            match e.lock_holder() {
                Some(who) => eprintln!(
                    "error: workspace is locked by {who}: {}",
                    e.message().unwrap_or("locked")
                ),
                None => eprintln!("error: {}", e.message().unwrap_or("state conflict")),
            }
            7
        }
        (_, Some("bad_request")) => {
            eprintln!("error: {}", e.message().unwrap_or("bad request"));
            2
        }
        (_, Some("not_found")) => {
            eprintln!("error: {}", e.message().unwrap_or("not found"));
            6
        }
        _ => api_fail(e),
    }
}

/// Print the promote result: new serial + the source serial it was copied from.
fn print_promote_result(resp: &PromoteResponse, fallback_source: i64) {
    let new_serial = resp.serial.map(|s| s.to_string()).unwrap_or_else(dash);
    let source = resp.source_serial.unwrap_or(fallback_source);
    if resp.idempotent_replay {
        eprintln!("already promoted (idempotent replay)");
    }
    eprintln!("promoted serial {source} → new current serial {new_serial}");
}

fn state_promote(cli: &Cli, workspace: &str, version_id: &str, assume_yes: bool) -> CmdResult {
    let (client, org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let style = Style::detect(cli.global.no_color);

    // Look up the target version by id (via the versions list) so the prompt can
    // echo the real serial + resource_count blast radius (§8).
    let version = match find_version_by_id(&client, &org, workspace, version_id) {
        Ok(Some(v)) => v,
        Ok(None) => {
            eprintln!("error: version {version_id} not found in workspace '{workspace}'");
            return Ok(6);
        }
        Err(e) => return Ok(api_fail(&e)),
    };

    let prompt = promote_prompt(&org, workspace, &version);
    Ok(do_promote(
        &client, &org, workspace, &version, &prompt, assume_yes, style,
    ))
}

/// The promote confirmation prompt echoing the blast radius (§6/§8).
fn promote_prompt(org: &str, workspace: &str, version: &StateVersion) -> String {
    format!(
        "About to PROMOTE version {vid} (serial {serial}) to latest in workspace '{workspace}' (org {org}).\n\
         This creates a new current state from a historical version. Resources: {resources}.",
        vid = version.id,
        serial = version.serial,
        resources = version.resource_count,
    )
}

/// Find a version by its **id**, paging the history (never single-page). Reuses
/// the slice-3 all-pages fetch so the whole history is searched.
fn find_version_by_id(
    client: &Client,
    org: &str,
    workspace: &str,
    version_id: &str,
) -> Result<Option<StateVersion>, ApiError> {
    Ok(client
        .state_versions_all(org, workspace)?
        .into_iter()
        .find(|v| v.id == version_id))
}

// ── rollback ────────────────────────────────────────────────────────────────

/// Resolve the rollback target serial from `--to` else the immediately-previous
/// (second-highest) serial (pure — rubric 4). `versions` need not be sorted.
/// Returns `None` when there is no previous serial (single/empty history) and no
/// explicit `--to`.
pub fn resolve_rollback_serial(versions: &[StateVersion], to: Option<i64>) -> Option<i64> {
    if let Some(s) = to {
        return Some(s);
    }
    // Second-highest distinct serial = the immediately-previous version.
    let mut serials: Vec<i64> = versions.iter().map(|v| v.serial).collect();
    serials.sort_unstable();
    serials.dedup();
    if serials.len() < 2 {
        return None;
    }
    Some(serials[serials.len() - 2])
}

fn state_rollback(cli: &Cli, workspace: &str, to: Option<i64>, assume_yes: bool) -> CmdResult {
    let (client, org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let style = Style::detect(cli.global.no_color);

    // Read history once to find the current serial (for the confirm text) and,
    // when no --to, the previous serial.
    let history = match client.state_versions_all(&org, workspace) {
        Ok(h) => h,
        Err(e) => return Ok(api_fail(&e)),
    };
    let current_serial = history.iter().map(|v| v.serial).max();

    let target_serial = match resolve_rollback_serial(&history, to) {
        Some(s) => s,
        None => {
            eprintln!(
                "error: no previous serial to roll back to in workspace '{workspace}'; pass --to <serial>"
            );
            return Ok(6);
        }
    };

    // Resolve the target serial → version_id via the slice-3 PAGED helper (never
    // single-page). Not found after exhausting history → exit 6.
    let version = match client.find_version_by_serial(&org, workspace, target_serial) {
        Ok(Some(v)) => v,
        Ok(None) => {
            eprintln!("error: serial {target_serial} not found in workspace '{workspace}'");
            return Ok(6);
        }
        Err(e) => return Ok(api_fail(&e)),
    };

    let prompt = rollback_prompt(&org, workspace, current_serial, &version);
    Ok(do_promote(
        &client, &org, workspace, &version, &prompt, assume_yes, style,
    ))
}

/// The rollback confirmation prompt, naming BOTH the current source serial and
/// the target serial explicitly (§7/§8).
fn rollback_prompt(
    org: &str,
    workspace: &str,
    current_serial: Option<i64>,
    target: &StateVersion,
) -> String {
    let from = current_serial.map(|s| s.to_string()).unwrap_or_else(dash);
    format!(
        "About to ROLL BACK workspace '{workspace}' (org {org}) from serial {from} to serial {target_serial}.\n\
         This promotes historical version {vid} as a new current state. Resources: {resources}.",
        target_serial = target.serial,
        vid = target.id,
        resources = target.resource_count,
    )
}

// ── unlock ──────────────────────────────────────────────────────────────────

fn state_unlock(cli: &Cli, workspace: &str, assume_yes: bool) -> CmdResult {
    let (client, org) = match client_and_org(cli) {
        Ok(v) => v,
        Err(code) => return Ok(code),
    };
    let style = Style::detect(cli.global.no_color);

    // Step 1: read the current lock. No lock held → say so, exit 0 (§7 N4).
    let lock = match client.state_lock_get(&org, workspace) {
        Ok(Some(l)) => l,
        Ok(None) => {
            eprintln!("no lock held on workspace '{workspace}'");
            return Ok(0);
        }
        Err(e) => return Ok(api_fail(&e)),
    };

    let prompt = unlock_prompt(&org, workspace, &lock);
    match confirm(&prompt, assume_yes, style) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("aborted");
            return Ok(EXIT_CONFIRM);
        }
        Err(code) => return Ok(code),
    }

    // Step 2: DELETE echoing the exact lock id (§7 N4: a bare unlock is a 400).
    match client.state_unlock(&org, workspace, &lock.lock_id) {
        Ok(_) => {
            eprintln!(
                "unlocked workspace '{workspace}' (released lock {})",
                lock.lock_id
            );
            Ok(0)
        }
        Err(e) => Ok(unlock_error_exit(&e)),
    }
}

/// The unlock confirmation prompt, showing the holder (§7/§8).
fn unlock_prompt(org: &str, workspace: &str, lock: &LockInfo) -> String {
    let who = lock.who.as_deref().unwrap_or(DASH);
    let op = lock.operation.as_deref().unwrap_or(DASH);
    let created = lock.created_at.map(relative_time).unwrap_or_else(dash);
    format!(
        "About to force-UNLOCK workspace '{workspace}' (org {org}).\n\
         Lock {id} held by {who} · operation {op} · created {created}.",
        id = lock.lock_id,
    )
}

/// Map an unlock API error to its §9 exit code (member + human server-gated).
fn unlock_error_exit(e: &ApiError) -> i32 {
    match e.error_code() {
        Some("forbidden" | "unauthorized" | "invalid_token") => {
            eprintln!(
                "error: {} (force-unlock requires the member role and a human token)",
                e.message().unwrap_or("forbidden")
            );
            5
        }
        _ => api_fail(e),
    }
}

fn dash() -> String {
    DASH.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(id: &str, serial: i64, resources: u64) -> StateVersion {
        StateVersion {
            id: id.into(),
            serial,
            resource_count: resources,
            lineage: None,
            pushed_by: None,
            pushed_at: 0,
            scan_status: None,
            critical_count: 0,
            high_count: 0,
            promoted_from_serial: None,
            actor_type: None,
            actor_name: None,
        }
    }

    fn with_actor(mut v: StateVersion, ty: Option<&str>, name: Option<&str>) -> StateVersion {
        v.actor_type = ty.map(str::to_string);
        v.actor_name = name.map(str::to_string);
        v
    }

    // ── versions rendering (rubric 3) ────────────────────────────────────────

    #[test]
    fn highest_serial_is_marked_current() {
        let vs = vec![version("v11", 11, 126), version("v14", 14, 128)];
        let rows = build_version_rows(&vs);
        // Sorted newest-first; the top row is serial 14 and is current.
        assert_eq!(rows[0].serial, 14);
        assert!(rows[0].current);
        assert!(!rows[1].current);
        let out = render_versions(&rows);
        assert!(out.contains("14 *"), "{out}");
        // The current row's NOTE reads "current".
        assert!(out.contains("current"), "{out}");
    }

    #[test]
    fn actor_format_is_type_colon_name() {
        let v = with_actor(version("v1", 1, 10), Some("vcs"), Some("ci-deploy"));
        let rows = build_version_rows(&[v]);
        assert_eq!(rows[0].actor, "vcs:ci-deploy");
    }

    #[test]
    fn actor_degrades_gracefully() {
        assert_eq!(actor_label(&version("v", 1, 0)), "—");
        let bot = with_actor(version("v", 1, 0), Some("bot"), Some("nightly"));
        assert_eq!(actor_label(&bot), "bot:nightly");
        let human = with_actor(version("v", 1, 0), Some("human"), Some("rhys"));
        assert_eq!(actor_label(&human), "human:rhys");
    }

    #[test]
    fn note_comes_from_promoted_from_serial() {
        let mut v = version("v13", 13, 128);
        v.promoted_from_serial = Some(11);
        // Add a higher serial so v13 is not "current" (else NOTE = "current").
        let rows = build_version_rows(&[v, version("v14", 14, 128)]);
        let v13 = rows.iter().find(|r| r.serial == 13).unwrap();
        assert_eq!(v13.note.as_deref(), Some("promoted from 11"));
        let out = render_versions(&rows);
        assert!(out.contains("promoted from 11"), "{out}");
    }

    #[test]
    fn versions_json_is_valid_and_marks_current() {
        let vs = vec![version("v11", 11, 126), version("v14", 14, 128)];
        let rows = build_version_rows(&vs);
        let json = serde_json::to_string(&rows).unwrap();
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(back[0]["serial"], 14);
        assert_eq!(back[0]["current"], true);
    }

    // ── rollback target resolution (rubric 4) ────────────────────────────────

    #[test]
    fn rollback_default_is_previous_serial() {
        let vs = vec![
            version("v14", 14, 0),
            version("v13", 13, 0),
            version("v12", 12, 0),
        ];
        // No --to → immediately-previous (second-highest) = 13.
        assert_eq!(resolve_rollback_serial(&vs, None), Some(13));
    }

    #[test]
    fn rollback_to_overrides_default() {
        let vs = vec![version("v14", 14, 0), version("v13", 13, 0)];
        assert_eq!(resolve_rollback_serial(&vs, Some(11)), Some(11));
    }

    #[test]
    fn rollback_none_when_single_version() {
        let vs = vec![version("v1", 1, 0)];
        assert_eq!(resolve_rollback_serial(&vs, None), None);
        // But an explicit --to still resolves even with a short history.
        assert_eq!(resolve_rollback_serial(&vs, Some(1)), Some(1));
    }

    #[test]
    fn rollback_dedups_serials_for_previous() {
        // Duplicate serials must not collapse the "previous" to the same number.
        let vs = vec![
            version("a", 14, 0),
            version("b", 14, 0),
            version("c", 10, 0),
        ];
        assert_eq!(resolve_rollback_serial(&vs, None), Some(10));
    }

    // ── serial resolution via the injected PAGED helper (rubric 4) ───────────
    // Mirrors tm-api's find_by_serial seam: a multi-page fake, target on a later
    // page, and not-found after exhaustion — no network, no sleep.

    fn paged_find(serial: i64, total: i64, page: i64) -> (Option<i64>, u64) {
        let calls = std::cell::RefCell::new(0u64);
        let mut offset = 0i64;
        let found = loop {
            *calls.borrow_mut() += 1;
            let end = (offset + page).min(total);
            let batch: Vec<i64> = (offset..end).collect();
            let got = batch.len() as i64;
            if let Some(s) = batch.into_iter().find(|s| *s == serial) {
                break Some(s);
            }
            if got < page {
                break None;
            }
            offset += page;
        };
        let n = *calls.borrow();
        (found, n)
    }

    #[test]
    fn serial_resolves_on_a_later_page() {
        // serials 0..120, pages of 50 → serial 115 is on page 3.
        let (found, calls) = paged_find(115, 120, 50);
        assert_eq!(found, Some(115));
        assert_eq!(calls, 3, "must page past the first page to reach it");
    }

    #[test]
    fn serial_not_found_after_exhausting_history() {
        let (found, calls) = paged_find(999, 120, 50);
        assert_eq!(found, None);
        // Paged all the way to the short final page before giving up → exit 6.
        assert_eq!(calls, 3);
    }

    // ── prompt blast radius (§8) ──────────────────────────────────────────────

    #[test]
    fn promote_prompt_echoes_blast_radius() {
        let v = version("ver-11", 11, 126);
        let p = promote_prompt("acme", "prod", &v);
        assert!(p.contains("PROMOTE version ver-11 (serial 11)"), "{p}");
        assert!(p.contains("workspace 'prod' (org acme)"), "{p}");
        assert!(p.contains("Resources: 126"), "{p}");
    }

    #[test]
    fn rollback_prompt_names_both_serials() {
        let target = version("ver-11", 11, 126);
        let p = rollback_prompt("acme", "prod", Some(14), &target);
        assert!(p.contains("from serial 14 to serial 11"), "{p}");
        assert!(p.contains("Resources: 126"), "{p}");
    }

    #[test]
    fn unlock_prompt_shows_holder() {
        let lock = LockInfo {
            lock_id: "lock-abc".into(),
            operation: Some("OperationTypePlan".into()),
            who: Some("rhys".into()),
            info: None,
            created_at: None,
            actor_type: None,
            actor_name: None,
        };
        let p = unlock_prompt("acme", "prod", &lock);
        assert!(
            p.contains("force-UNLOCK workspace 'prod' (org acme)"),
            "{p}"
        );
        assert!(p.contains("Lock lock-abc held by rhys"), "{p}");
        assert!(p.contains("operation OperationTypePlan"), "{p}");
    }

    // ── ls rendering ─────────────────────────────────────────────────────────

    #[test]
    fn ls_renders_serial_and_dashes_empty() {
        let ws = vec![
            Workspace {
                id: "w1".into(),
                name: "prod".into(),
                created_by: None,
                created_at: None,
                latest_serial: Some(14),
                resource_count: Some(128),
                pushed_at: None,
                pushed_by: None,
                scan_status: None,
                lock_id: None,
                lock_who: None,
            },
            Workspace {
                id: "w2".into(),
                name: "empty".into(),
                created_by: None,
                created_at: None,
                latest_serial: None,
                resource_count: None,
                pushed_at: None,
                pushed_by: None,
                scan_status: None,
                lock_id: None,
                lock_who: None,
            },
        ];
        let out = render_ls(&ws);
        assert!(out.contains("WORKSPACE"));
        assert!(out.contains("prod"));
        assert!(out.contains("14"));
        assert!(out.contains("128"));
        // The empty workspace's serial/resources render as em dash.
        assert!(out.contains('—'), "{out}");
    }
}
