//! State endpoints (§7 `state ls/versions/promote/unlock`, `lock push`).

use crate::error::ApiError;
use crate::models::{
    LockGetResponse, LockInfo, LockPushResponse, PromoteResponse, StateVersion,
    VersionListResponse, Workspace, WorkspaceListResponse, WorkspaceProvidersResponse,
};
use crate::Client;

/// Default versions page size (§7 rollback: default 50).
pub const VERSIONS_PAGE_DEFAULT: u64 = 50;
/// Maximum versions page size the server honours (§7 rollback: max 100).
pub const VERSIONS_PAGE_MAX: u64 = 100;

impl Client {
    /// `GET /api/v1/{org}/state/` — list workspaces in the org. Unwraps the
    /// `{ workspaces }` envelope.
    pub fn state_list(&self, org: &str) -> Result<Vec<Workspace>, ApiError> {
        let path = format!("/api/v1/{org}/state/");
        let resp: WorkspaceListResponse = self.http().get_json(&path)?;
        Ok(resp.workspaces)
    }

    /// `GET /api/v1/{org}/state/{ws}/versions?limit=&offset=` — one page of a
    /// workspace's version history. Unwraps the `{ versions }` envelope.
    pub fn state_versions(
        &self,
        org: &str,
        workspace: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<StateVersion>, ApiError> {
        let path = format!("/api/v1/{org}/state/{workspace}/versions");
        let query = [
            ("limit", limit.min(VERSIONS_PAGE_MAX).to_string()),
            ("offset", offset.to_string()),
        ];
        let resp: VersionListResponse = self.http().get_json_query(&path, &query)?;
        Ok(resp.versions)
    }

    /// Page through **all** versions of a workspace, following `?offset=` until a
    /// short/empty page signals exhaustion. Pages at [`VERSIONS_PAGE_DEFAULT`].
    pub fn state_versions_all(
        &self,
        org: &str,
        workspace: &str,
    ) -> Result<Vec<StateVersion>, ApiError> {
        paginate_versions(VERSIONS_PAGE_DEFAULT, |limit, offset| {
            self.state_versions(org, workspace, limit, offset)
        })
    }

    /// Find a specific version by its `serial`, paging via `?offset=` until found
    /// or history is exhausted (§7 rollback S3). Returns `None` when absent.
    pub fn find_version_by_serial(
        &self,
        org: &str,
        workspace: &str,
        serial: i64,
    ) -> Result<Option<StateVersion>, ApiError> {
        find_by_serial(serial, VERSIONS_PAGE_DEFAULT, |limit, offset| {
            self.state_versions(org, workspace, limit, offset)
        })
    }

    /// `POST /api/v1/{org}/state/{ws}/versions/{version_id}/promote` with an
    /// `Idempotency-Key` header (§7: required, reused on `--retry`).
    pub fn state_promote(
        &self,
        org: &str,
        workspace: &str,
        version_id: &str,
        idempotency_key: &str,
    ) -> Result<PromoteResponse, ApiError> {
        let path = format!("/api/v1/{org}/state/{workspace}/versions/{version_id}/promote");
        let empty = serde_json::json!({});
        self.http()
            .post_json_headers(&path, &empty, &[("Idempotency-Key", idempotency_key)])
    }

    /// `GET /api/v1/{org}/state/{ws}/lock` — current lock holder, or `None` when
    /// unlocked. Unwraps the `{ lock }` envelope.
    pub fn state_lock_get(&self, org: &str, workspace: &str) -> Result<Option<LockInfo>, ApiError> {
        let path = format!("/api/v1/{org}/state/{workspace}/lock");
        let resp: LockGetResponse = self.http().get_json(&path)?;
        Ok(resp.lock)
    }

    /// `DELETE /api/v1/{org}/state/{ws}/lock` — force-unlock. The body **must**
    /// echo the exact lock id under key `ID` (§7 N4); a bare unlock is a 400.
    pub fn state_unlock(
        &self,
        org: &str,
        workspace: &str,
        lock_id: &str,
    ) -> Result<serde_json::Value, ApiError> {
        let path = format!("/api/v1/{org}/state/{workspace}/lock");
        let body = serde_json::json!({ "ID": lock_id });
        self.http().delete_json(&path, &body)
    }

    /// `GET /api/v1/{org}/state/{ws}/providers` — in-use providers with
    /// `latest_matching`/scan fields, used to poll lock-push posture (§7 S7).
    pub fn workspace_providers(
        &self,
        org: &str,
        workspace: &str,
    ) -> Result<WorkspaceProvidersResponse, ApiError> {
        let path = format!("/api/v1/{org}/state/{workspace}/providers");
        self.http().get_json(&path)
    }

    /// `PUT /state/{org}/{ws}/terraform.lock.hcl` — raw lock-file upload with an
    /// optional `X-Git-Repo-URL` attribution header (§7 lock push).
    pub fn lock_push(
        &self,
        org: &str,
        workspace: &str,
        body: &[u8],
        repo_url: Option<&str>,
    ) -> Result<LockPushResponse, ApiError> {
        let path = format!("/state/{org}/{workspace}/terraform.lock.hcl");
        let headers: Vec<(&str, &str)> = match repo_url {
            Some(url) => vec![("X-Git-Repo-URL", url)],
            None => vec![],
        };
        self.http().put_bytes(&path, body, &headers)
    }
}

// ── pagination helpers (network-free core, unit-tested by injection) ───────────

/// Collect every version by paging `fetch(limit, offset)` until a page returns
/// fewer than `page` rows (or empty). The injected `fetch` is the seam the tests
/// use to avoid the network.
fn paginate_versions<F>(page: u64, mut fetch: F) -> Result<Vec<StateVersion>, ApiError>
where
    F: FnMut(u64, u64) -> Result<Vec<StateVersion>, ApiError>,
{
    let mut all = Vec::new();
    let mut offset = 0u64;
    loop {
        let batch = fetch(page, offset)?;
        let got = batch.len() as u64;
        all.extend(batch);
        // A short (or empty) page means we've reached the end — the server never
        // returns more than `page` rows, so `got < page` is the exhaustion signal.
        if got < page {
            break;
        }
        offset += page;
    }
    Ok(all)
}

/// Page via `fetch(limit, offset)` until a version with `serial` is found or the
/// history is exhausted. Returns the matching version, or `None` if absent.
fn find_by_serial<F>(serial: i64, page: u64, mut fetch: F) -> Result<Option<StateVersion>, ApiError>
where
    F: FnMut(u64, u64) -> Result<Vec<StateVersion>, ApiError>,
{
    let mut offset = 0u64;
    loop {
        let batch = fetch(page, offset)?;
        let got = batch.len() as u64;
        if let Some(found) = batch.into_iter().find(|v| v.serial == serial) {
            return Ok(Some(found));
        }
        if got < page {
            return Ok(None);
        }
        offset += page;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A multi-page fake source: `total` versions with serials `0..total`, served
    /// in pages of `page`. Records how many pages were requested so we can assert
    /// the pager stops early once it finds the target.
    fn fake_source(
        total: i64,
        page: u64,
        calls: &std::cell::RefCell<u64>,
    ) -> impl Fn(u64, u64) -> Result<Vec<StateVersion>, ApiError> + '_ {
        move |limit, offset| {
            *calls.borrow_mut() += 1;
            assert_eq!(limit, page, "pager must request the fixed page size");
            let start = offset as i64;
            let end = (start + page as i64).min(total);
            let versions = (start..end)
                .map(|serial| StateVersion {
                    id: format!("v{serial}"),
                    serial,
                    resource_count: 0,
                    lineage: None,
                    pushed_by: None,
                    pushed_at: 0,
                    scan_status: None,
                    critical_count: 0,
                    high_count: 0,
                    promoted_from_serial: None,
                    actor_type: None,
                    actor_name: None,
                })
                .collect();
            Ok(versions)
        }
    }

    #[test]
    fn paginate_collects_every_page() {
        let calls = std::cell::RefCell::new(0);
        let all = paginate_versions(50, fake_source(120, 50, &calls)).unwrap();
        assert_eq!(all.len(), 120);
        // 50 + 50 + 20 → three pages (the last is short, ending the loop).
        assert_eq!(*calls.borrow(), 3);
    }

    #[test]
    fn paginate_exhausts_on_exact_multiple() {
        // 100 items at page 50 → two full pages, then a third empty page proves
        // exhaustion (a full second page is not a stop signal on its own).
        let calls = std::cell::RefCell::new(0);
        let all = paginate_versions(50, fake_source(100, 50, &calls)).unwrap();
        assert_eq!(all.len(), 100);
        assert_eq!(*calls.borrow(), 3);
    }

    #[test]
    fn find_serial_on_first_page() {
        let calls = std::cell::RefCell::new(0);
        let found = find_by_serial(7, 50, fake_source(120, 50, &calls))
            .unwrap()
            .expect("serial 7 exists");
        assert_eq!(found.serial, 7);
        assert_eq!(*calls.borrow(), 1, "found on page 1 — no further paging");
    }

    #[test]
    fn find_serial_on_a_later_page() {
        let calls = std::cell::RefCell::new(0);
        let found = find_by_serial(115, 50, fake_source(120, 50, &calls))
            .unwrap()
            .expect("serial 115 exists");
        assert_eq!(found.serial, 115);
        // 0..50, 50..100, 100..120 → three pages to reach serial 115.
        assert_eq!(*calls.borrow(), 3);
    }

    #[test]
    fn find_serial_absent_returns_none_after_exhausting() {
        let calls = std::cell::RefCell::new(0);
        let found = find_by_serial(999, 50, fake_source(120, 50, &calls)).unwrap();
        assert!(found.is_none());
        // Must page all the way to the short final page before giving up.
        assert_eq!(*calls.borrow(), 3);
    }

    #[test]
    fn find_serial_stops_early_before_exhausting() {
        // Target on page 1 of a large history: must NOT read every page.
        let calls = std::cell::RefCell::new(0);
        find_by_serial(3, 50, fake_source(10_000, 50, &calls))
            .unwrap()
            .unwrap();
        assert_eq!(*calls.borrow(), 1);
    }

    #[test]
    fn find_serial_propagates_fetch_error() {
        let err = find_by_serial(1, 50, |_l, _o| {
            Err(ApiError::from_status(500, "u".into(), String::new()))
        });
        assert!(err.is_err());
    }
}
