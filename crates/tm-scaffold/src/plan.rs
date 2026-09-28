//! The `upgrade` diff engine (SCAFFOLD-PUBLISH-AUTH.md §3) — a terraform-plan-style
//! reconciliation of the desired file set against what is on disk.
//!
//! The core, [`compute`], is pure: it takes the rendered desired files, the lock,
//! and an injected "read this path" closure, and returns a [`Plan`] of `+ ~ ! -`
//! actions. [`Plan::apply`] performs the IO. Splitting them keeps every branch of
//! the drift model unit-testable without a real filesystem.
//!
//! Per managed file we compare three hashes — desired (freshly rendered), disk,
//! and the lock's "last generated":
//!
//! | on disk | disk == desired | disk == last-generated | action           |
//! |---------|-----------------|------------------------|------------------|
//! | absent  | —               | —                      | **Add** `+`      |
//! | present | yes             | —                      | **Unchanged**    |
//! | present | no              | yes (user untouched)   | **UpdateClean** `~` |
//! | present | no              | no (user edited)       | **UpdateDrifted** `!` |
//!
//! Scaffold-once files are only ever **Add** (absent) or **Unchanged** (present) —
//! the user owns them after creation. A managed file recorded in the lock but no
//! longer desired is **Prune** `-`.

use std::path::{Path, PathBuf};

use crate::error::ScaffoldError;
use crate::lockfile::{key_of, sha256_hex, LockFile};
use crate::render::{FileClass, GeneratedFile};

/// The reconciliation action for one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Desired file absent on disk → create.
    Add,
    /// Present, untouched since last generation → safe rewrite.
    UpdateClean,
    /// Present but user-edited → write `<path>.terramantle-new`, do not clobber.
    UpdateDrifted,
    /// Present and already matches desired → skip.
    Unchanged,
    /// Managed, previously generated, no longer desired → remove.
    Prune,
}

impl Action {
    /// The terraform-plan glyph for this action.
    pub fn glyph(self) -> char {
        match self {
            Action::Add => '+',
            Action::UpdateClean => '~',
            Action::UpdateDrifted => '!',
            Action::Unchanged => '=',
            Action::Prune => '-',
        }
    }

    /// Whether this action mutates the working tree when applied (ignoring the
    /// side-car `.terramantle-new` that `UpdateDrifted` always writes).
    pub fn is_noop(self) -> bool {
        matches!(self, Action::Unchanged)
    }
}

/// One planned change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEntry {
    pub path: PathBuf,
    pub action: Action,
    pub template: String,
    pub class: FileClass,
    /// The bytes to write (None for `Unchanged`/`Prune`).
    pub desired: Option<String>,
}

/// A full reconciliation plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub entries: Vec<PlanEntry>,
}

/// The outcome of applying a plan (for user narration).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub added: Vec<PathBuf>,
    pub updated: Vec<PathBuf>,
    /// `(original, side-car)` pairs written for drifted files.
    pub drifted: Vec<(PathBuf, PathBuf)>,
    pub pruned: Vec<PathBuf>,
}

/// Compute the plan. `read` returns the current on-disk content of a
/// repo-root-relative path, or `None` if it does not exist.
pub fn compute(
    desired: &[GeneratedFile],
    lock: &LockFile,
    read: impl Fn(&Path) -> Option<String>,
) -> Plan {
    let mut entries = Vec::new();
    let desired_keys: std::collections::BTreeSet<String> =
        desired.iter().map(|f| key_of(&f.path)).collect();

    for f in desired {
        let desired_hash = sha256_hex(f.content.as_bytes());
        let action = match read(&f.path) {
            None => Action::Add,
            Some(disk) => {
                if f.class == FileClass::Once {
                    // User owns once-files after creation: never rewrite.
                    Action::Unchanged
                } else {
                    let disk_hash = sha256_hex(disk.as_bytes());
                    if disk_hash == desired_hash {
                        Action::Unchanged
                    } else {
                        match lock.get(&f.path) {
                            Some(e) if e.sha256 == disk_hash => Action::UpdateClean,
                            _ => Action::UpdateDrifted,
                        }
                    }
                }
            }
        };
        entries.push(PlanEntry {
            path: f.path.clone(),
            action,
            template: f.template.clone(),
            class: f.class,
            desired: Some(f.content.clone()),
        });
    }

    // Prune: managed files the lock remembers but the manifest no longer wants.
    for path in lock.managed_paths() {
        if desired_keys.contains(&key_of(&path)) {
            continue;
        }
        if read(&path).is_some() {
            entries.push(PlanEntry {
                path,
                action: Action::Prune,
                template: String::new(),
                class: FileClass::Managed,
                desired: None,
            });
        }
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Plan { entries }
}

impl Plan {
    /// Entries that would change the tree (everything except `Unchanged`).
    pub fn changes(&self) -> impl Iterator<Item = &PlanEntry> {
        self.entries.iter().filter(|e| !e.action.is_noop())
    }

    /// True when applying the plan would do nothing.
    pub fn is_empty_of_changes(&self) -> bool {
        self.changes().next().is_none()
    }

    /// Count of each action, for the summary line.
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mut add = 0;
        let mut update = 0;
        let mut drift = 0;
        let mut prune = 0;
        for e in &self.entries {
            match e.action {
                Action::Add => add += 1,
                Action::UpdateClean => update += 1,
                Action::UpdateDrifted => drift += 1,
                Action::Prune => prune += 1,
                Action::Unchanged => {}
            }
        }
        (add, update, drift, prune)
    }

    /// Render a terraform-plan-style summary (one line per change + a total).
    pub fn render(&self) -> String {
        let mut out = String::new();
        for e in self.changes() {
            let note = match e.action {
                Action::Add => "create",
                Action::UpdateClean => "update",
                Action::UpdateDrifted => {
                    "drifted — write .terramantle-new (use --force to overwrite)"
                }
                Action::Prune => "remove (no longer in manifest)",
                Action::Unchanged => "",
            };
            out.push_str(&format!(
                "  {} {}    {}\n",
                e.action.glyph(),
                e.path.display(),
                note
            ));
        }
        let (a, u, d, p) = self.counts();
        if a + u + d + p == 0 {
            out.push_str("No changes. Scaffold is up to date.\n");
        } else {
            out.push_str(&format!(
                "\nPlan: {a} to add, {u} to update, {d} drifted, {p} to remove.\n"
            ));
        }
        out
    }

    /// Apply the plan under `root`, mutating the lock in place. `force` overwrites
    /// drifted files instead of writing a side-car.
    pub fn apply(
        &self,
        root: &Path,
        lock: &mut LockFile,
        force: bool,
    ) -> Result<ApplyOutcome, ScaffoldError> {
        let mut out = ApplyOutcome::default();
        for e in &self.entries {
            let abs = root.join(&e.path);
            match e.action {
                Action::Add | Action::UpdateClean => {
                    let content = e.desired.as_deref().unwrap_or_default();
                    write_file(&abs, content)?;
                    record_from_entry(lock, e);
                    if e.action == Action::Add {
                        out.added.push(e.path.clone());
                    } else {
                        out.updated.push(e.path.clone());
                    }
                }
                Action::UpdateDrifted => {
                    let content = e.desired.as_deref().unwrap_or_default();
                    if force {
                        write_file(&abs, content)?;
                        record_from_entry(lock, e);
                        out.updated.push(e.path.clone());
                    } else {
                        let side = sidecar(&e.path);
                        write_file(&root.join(&side), content)?;
                        // Lock unchanged: the original keeps its old recorded hash
                        // so re-running still detects the drift.
                        out.drifted.push((e.path.clone(), side));
                    }
                }
                Action::Prune => {
                    match std::fs::remove_file(&abs) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(source) => return Err(ScaffoldError::Write { path: abs, source }),
                    }
                    lock.forget(&e.path);
                    out.pruned.push(e.path.clone());
                }
                Action::Unchanged => {
                    // Adopt into the lock if it predates it, so future upgrades
                    // can tell clean from drifted.
                    if lock.get(&e.path).is_none() {
                        record_from_entry(lock, e);
                    }
                }
            }
        }
        Ok(out)
    }
}

fn sidecar(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".terramantle-new");
    PathBuf::from(s)
}

fn record_from_entry(lock: &mut LockFile, e: &PlanEntry) {
    let gen = GeneratedFile {
        path: e.path.clone(),
        content: e.desired.clone().unwrap_or_default(),
        template: e.template.clone(),
        template_version: crate::render::TEMPLATE_VERSION,
        class: e.class,
    };
    lock.record(&gen);
}

fn write_file(abs: &Path, content: &str) -> Result<(), ScaffoldError> {
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ScaffoldError::Write {
            path: abs.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(abs, content).map_err(|source| ScaffoldError::Write {
        path: abs.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn managed(path: &str, content: &str) -> GeneratedFile {
        GeneratedFile {
            path: PathBuf::from(path),
            content: content.to_string(),
            template: "t".into(),
            template_version: 1,
            class: FileClass::Managed,
        }
    }
    fn once(path: &str, content: &str) -> GeneratedFile {
        GeneratedFile {
            class: FileClass::Once,
            ..managed(path, content)
        }
    }

    /// An in-memory filesystem for the pure core.
    fn fs(pairs: &[(&str, &str)]) -> impl Fn(&Path) -> Option<String> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |p: &Path| map.get(&key_of(p)).cloned()
    }

    fn action_for<'a>(plan: &'a Plan, path: &str) -> Option<&'a PlanEntry> {
        plan.entries.iter().find(|e| e.path == Path::new(path))
    }

    #[test]
    fn absent_file_is_add() {
        let desired = vec![managed("ci.yml", "v1")];
        let plan = compute(&desired, &LockFile::default(), fs(&[]));
        assert_eq!(action_for(&plan, "ci.yml").unwrap().action, Action::Add);
    }

    #[test]
    fn matching_disk_is_unchanged() {
        let desired = vec![managed("ci.yml", "v1")];
        let plan = compute(&desired, &LockFile::default(), fs(&[("ci.yml", "v1")]));
        assert_eq!(
            action_for(&plan, "ci.yml").unwrap().action,
            Action::Unchanged
        );
        assert!(plan.is_empty_of_changes());
    }

    #[test]
    fn untouched_since_last_gen_is_clean_update() {
        // disk == recorded (old gen), desired differs → safe rewrite.
        let mut lock = LockFile::default();
        lock.record(&managed("ci.yml", "old"));
        let desired = vec![managed("ci.yml", "new")];
        let plan = compute(&desired, &lock, fs(&[("ci.yml", "old")]));
        assert_eq!(
            action_for(&plan, "ci.yml").unwrap().action,
            Action::UpdateClean
        );
    }

    #[test]
    fn user_edited_is_drifted() {
        // disk != recorded and != desired → user edited it.
        let mut lock = LockFile::default();
        lock.record(&managed("ci.yml", "old"));
        let desired = vec![managed("ci.yml", "new")];
        let plan = compute(&desired, &lock, fs(&[("ci.yml", "hand-edited")]));
        assert_eq!(
            action_for(&plan, "ci.yml").unwrap().action,
            Action::UpdateDrifted
        );
    }

    #[test]
    fn no_lock_but_present_and_differs_is_drifted() {
        // Adopting a repo that predates the lock: don't clobber.
        let desired = vec![managed("ci.yml", "new")];
        let plan = compute(
            &desired,
            &LockFile::default(),
            fs(&[("ci.yml", "preexisting")]),
        );
        assert_eq!(
            action_for(&plan, "ci.yml").unwrap().action,
            Action::UpdateDrifted
        );
    }

    #[test]
    fn once_file_present_is_never_touched() {
        let desired = vec![once("main.tf", "new-scaffold")];
        let plan = compute(
            &desired,
            &LockFile::default(),
            fs(&[("main.tf", "user code")]),
        );
        assert_eq!(
            action_for(&plan, "main.tf").unwrap().action,
            Action::Unchanged
        );
    }

    #[test]
    fn once_file_absent_is_add() {
        let desired = vec![once("main.tf", "scaffold")];
        let plan = compute(&desired, &LockFile::default(), fs(&[]));
        assert_eq!(action_for(&plan, "main.tf").unwrap().action, Action::Add);
    }

    #[test]
    fn dropped_managed_file_is_pruned_but_once_is_not() {
        let mut lock = LockFile::default();
        lock.record(&managed("old-ci.yml", "x"));
        lock.record(&once("main.tf", "y"));
        let desired = vec![managed("ci.yml", "v1")];
        let plan = compute(
            &desired,
            &lock,
            fs(&[("old-ci.yml", "x"), ("main.tf", "y")]),
        );
        assert_eq!(
            action_for(&plan, "old-ci.yml").unwrap().action,
            Action::Prune
        );
        // once file recorded in lock is never a prune candidate
        assert!(action_for(&plan, "main.tf").is_none());
    }

    #[test]
    fn apply_writes_adds_and_records_lock() {
        let dir = tempfile::tempdir().unwrap();
        let desired = vec![managed(".github/workflows/terramantle.yml", "pipeline")];
        let plan = compute(&desired, &LockFile::default(), |_| None);
        let mut lock = LockFile::default();
        let out = plan.apply(dir.path(), &mut lock, false).unwrap();
        assert_eq!(out.added.len(), 1);
        let written =
            std::fs::read_to_string(dir.path().join(".github/workflows/terramantle.yml")).unwrap();
        assert_eq!(written, "pipeline");
        assert!(lock
            .get(&PathBuf::from(".github/workflows/terramantle.yml"))
            .is_some());
    }

    #[test]
    fn apply_drifted_writes_sidecar_not_original() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ci.yml"), "hand-edited").unwrap();
        let mut lock = LockFile::default();
        lock.record(&managed("ci.yml", "old"));
        let desired = vec![managed("ci.yml", "new")];
        let plan = compute(&desired, &lock, |p| {
            std::fs::read_to_string(dir.path().join(p)).ok()
        });
        let out = plan.apply(dir.path(), &mut lock, false).unwrap();
        assert_eq!(out.drifted.len(), 1);
        // original untouched, sidecar has the new content
        assert_eq!(
            std::fs::read_to_string(dir.path().join("ci.yml")).unwrap(),
            "hand-edited"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("ci.yml.terramantle-new")).unwrap(),
            "new"
        );
        // lock still records the OLD hash so drift persists until resolved
        assert_eq!(
            lock.get(&PathBuf::from("ci.yml")).unwrap().sha256,
            sha256_hex(b"old")
        );
    }

    #[test]
    fn apply_force_overwrites_drifted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ci.yml"), "hand-edited").unwrap();
        let mut lock = LockFile::default();
        lock.record(&managed("ci.yml", "old"));
        let desired = vec![managed("ci.yml", "new")];
        let plan = compute(&desired, &lock, |p| {
            std::fs::read_to_string(dir.path().join(p)).ok()
        });
        plan.apply(dir.path(), &mut lock, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("ci.yml")).unwrap(),
            "new"
        );
        assert!(!dir.path().join("ci.yml.terramantle-new").exists());
        assert_eq!(
            lock.get(&PathBuf::from("ci.yml")).unwrap().sha256,
            sha256_hex(b"new")
        );
    }

    #[test]
    fn apply_prune_removes_and_forgets() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("old.yml"), "x").unwrap();
        let mut lock = LockFile::default();
        lock.record(&managed("old.yml", "x"));
        let plan = compute(&[], &lock, |p| {
            std::fs::read_to_string(dir.path().join(p)).ok()
        });
        plan.apply(dir.path(), &mut lock, false).unwrap();
        assert!(!dir.path().join("old.yml").exists());
        assert!(lock.get(&PathBuf::from("old.yml")).is_none());
    }

    #[test]
    fn plan_render_mentions_counts() {
        let desired = vec![managed("a.yml", "1"), managed("b.yml", "2")];
        let plan = compute(&desired, &LockFile::default(), fs(&[]));
        let r = plan.render();
        assert!(r.contains("2 to add"));
        assert!(r.contains("+ a.yml"));
    }
}
