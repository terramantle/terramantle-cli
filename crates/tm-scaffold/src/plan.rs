//! The `upgrade` diff engine (SCAFFOLD-PUBLISH-AUTH.md §3) — a terraform-plan-style
//! reconciliation of the desired file set against what is on disk.
//!
//! The core, [`compute`], is pure: it takes the rendered desired files and an
//! injected "read this path" closure, and returns a [`Plan`] of `+ ~ -` actions.
//! [`Plan::apply`] performs the IO. Splitting them keeps every branch unit-testable
//! without a real filesystem.
//!
//! There is **no lock file**. Ownership is carried by the provenance header every
//! managed file starts with (`# managed by terramantle · template=…`), so `upgrade`
//! is a stateless re-render:
//!
//! | file class | on disk | vs desired | action        |
//! |------------|---------|------------|---------------|
//! | managed    | absent  | —          | **Add** `+`   |
//! | managed    | present | equal      | **Unchanged** |
//! | managed    | present | differs    | **Update** `~` (overwrite) |
//! | once       | absent  | —          | **Add** `+`   |
//! | once       | present | —          | **Unchanged** (never touched) |
//!
//! A file carrying the provenance marker that is no longer desired (e.g. the other
//! VCS's pipeline after a `vcs` switch) is **Prune** `-`. Scaffold-once files carry
//! no header, so they are invisible to prune — the user owns them.

use std::path::{Path, PathBuf};

use crate::error::ScaffoldError;
use crate::render::{candidate_managed_paths, FileClass, GeneratedFile, PROVENANCE_MARKER};

/// Normalise a path to a forward-slash key so comparisons are stable across
/// platforms.
fn key_of(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The reconciliation action for one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Desired file absent on disk → create.
    Add,
    /// Managed file present but differing from desired → overwrite.
    Update,
    /// Present and already matches desired (or a once-file that exists) → skip.
    Unchanged,
    /// Carries our provenance marker but is no longer desired → remove.
    Prune,
}

impl Action {
    /// The terraform-plan glyph for this action.
    pub fn glyph(self) -> char {
        match self {
            Action::Add => '+',
            Action::Update => '~',
            Action::Unchanged => '=',
            Action::Prune => '-',
        }
    }

    /// Whether this action leaves the working tree untouched.
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
    pub pruned: Vec<PathBuf>,
}

/// Compute the plan. `read` returns the current on-disk content of a
/// repo-root-relative path, or `None` if it does not exist.
pub fn compute(desired: &[GeneratedFile], read: impl Fn(&Path) -> Option<String>) -> Plan {
    let mut entries = Vec::new();
    let desired_keys: std::collections::BTreeSet<String> =
        desired.iter().map(|f| key_of(&f.path)).collect();

    for f in desired {
        let action = match read(&f.path) {
            None => Action::Add,
            Some(disk) => {
                // Once-files are the user's after creation; a managed file is
                // overwritten whenever it diverges from the freshly rendered bytes.
                if f.class == FileClass::Once || disk == f.content {
                    Action::Unchanged
                } else {
                    Action::Update
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

    // Prune: any file carrying our provenance marker that is no longer desired.
    for path in candidate_managed_paths() {
        if desired_keys.contains(&key_of(&path)) {
            continue;
        }
        if let Some(content) = read(&path) {
            if content.contains(PROVENANCE_MARKER) {
                entries.push(PlanEntry {
                    path,
                    action: Action::Prune,
                    template: String::new(),
                    class: FileClass::Managed,
                    desired: None,
                });
            }
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

    /// Count of (add, update, prune) for the summary line.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut add = 0;
        let mut update = 0;
        let mut prune = 0;
        for e in &self.entries {
            match e.action {
                Action::Add => add += 1,
                Action::Update => update += 1,
                Action::Prune => prune += 1,
                Action::Unchanged => {}
            }
        }
        (add, update, prune)
    }

    /// Render a terraform-plan-style summary (one line per change + a total).
    pub fn render(&self) -> String {
        let mut out = String::new();
        for e in self.changes() {
            let note = match e.action {
                Action::Add => "create",
                Action::Update => "update",
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
        let (a, u, p) = self.counts();
        if a + u + p == 0 {
            out.push_str("No changes. Scaffold is up to date.\n");
        } else {
            out.push_str(&format!(
                "\nPlan: {a} to add, {u} to update, {p} to remove.\n"
            ));
        }
        out
    }

    /// Apply the plan under `root`. Managed files are (over)written; scaffold-once
    /// files are only created when absent; pruned files are removed.
    pub fn apply(&self, root: &Path) -> Result<ApplyOutcome, ScaffoldError> {
        let mut out = ApplyOutcome::default();
        for e in &self.entries {
            let abs = root.join(&e.path);
            match e.action {
                Action::Add => {
                    write_file(&abs, e.desired.as_deref().unwrap_or_default())?;
                    out.added.push(e.path.clone());
                }
                Action::Update => {
                    write_file(&abs, e.desired.as_deref().unwrap_or_default())?;
                    out.updated.push(e.path.clone());
                }
                Action::Prune => {
                    match std::fs::remove_file(&abs) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(source) => return Err(ScaffoldError::Write { path: abs, source }),
                    }
                    out.pruned.push(e.path.clone());
                }
                Action::Unchanged => {}
            }
        }
        Ok(out)
    }
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
        let desired = vec![managed(".github/workflows/terramantle.yml", "v1")];
        let plan = compute(&desired, fs(&[]));
        assert_eq!(
            action_for(&plan, ".github/workflows/terramantle.yml")
                .unwrap()
                .action,
            Action::Add
        );
    }

    #[test]
    fn matching_disk_is_unchanged() {
        let desired = vec![managed(".gitlab-ci.yml", "v1")];
        let plan = compute(&desired, fs(&[(".gitlab-ci.yml", "v1")]));
        assert_eq!(
            action_for(&plan, ".gitlab-ci.yml").unwrap().action,
            Action::Unchanged
        );
        assert!(plan.is_empty_of_changes());
    }

    #[test]
    fn differing_managed_file_is_update() {
        let desired = vec![managed(".gitlab-ci.yml", "new")];
        let plan = compute(&desired, fs(&[(".gitlab-ci.yml", "old or hand-edited")]));
        assert_eq!(
            action_for(&plan, ".gitlab-ci.yml").unwrap().action,
            Action::Update
        );
    }

    #[test]
    fn once_file_present_is_never_touched() {
        let desired = vec![once("main.tf", "new-scaffold")];
        let plan = compute(&desired, fs(&[("main.tf", "user code")]));
        assert_eq!(
            action_for(&plan, "main.tf").unwrap().action,
            Action::Unchanged
        );
    }

    #[test]
    fn once_file_absent_is_add() {
        let desired = vec![once("main.tf", "scaffold")];
        let plan = compute(&desired, fs(&[]));
        assert_eq!(action_for(&plan, "main.tf").unwrap().action, Action::Add);
    }

    #[test]
    fn stale_managed_file_with_marker_is_pruned() {
        // Switched github → gitlab: the old GitHub pipeline still carries our
        // provenance marker and is no longer desired → prune.
        let stale = format!("# {PROVENANCE_MARKER}github/modules · v=1\njobs: {{}}\n");
        let desired = vec![managed(".gitlab-ci.yml", "v1")];
        let plan = compute(
            &desired,
            fs(&[
                (".gitlab-ci.yml", "v1"),
                (".github/workflows/terramantle.yml", &stale),
            ]),
        );
        assert_eq!(
            action_for(&plan, ".github/workflows/terramantle.yml")
                .unwrap()
                .action,
            Action::Prune
        );
    }

    #[test]
    fn foreign_file_without_marker_is_not_pruned() {
        // A user's own .github/workflows file that we never generated (no marker)
        // must not be pruned — but our candidate scan only checks known managed
        // paths, and even there requires the marker.
        let desired = vec![managed(".gitlab-ci.yml", "v1")];
        let plan = compute(
            &desired,
            fs(&[
                (".gitlab-ci.yml", "v1"),
                (
                    ".github/workflows/terramantle.yml",
                    "name: my own pipeline\n",
                ),
            ]),
        );
        assert!(action_for(&plan, ".github/workflows/terramantle.yml").is_none());
    }

    #[test]
    fn apply_writes_add_and_update_and_prune() {
        let dir = tempfile::tempdir().unwrap();
        // seed a stale marked file to prune + a to-be-updated file
        std::fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
        let stale = format!("# {PROVENANCE_MARKER}github/modules · v=1\n");
        std::fs::write(dir.path().join(".github/workflows/terramantle.yml"), &stale).unwrap();
        std::fs::write(dir.path().join(".gitlab-ci.yml"), "old").unwrap();

        let desired = vec![managed(".gitlab-ci.yml", "new")];
        let plan = compute(&desired, |p| {
            std::fs::read_to_string(dir.path().join(p)).ok()
        });
        let out = plan.apply(dir.path()).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join(".gitlab-ci.yml")).unwrap(),
            "new"
        );
        assert!(!dir
            .path()
            .join(".github/workflows/terramantle.yml")
            .exists());
        assert_eq!(out.updated, vec![PathBuf::from(".gitlab-ci.yml")]);
        assert_eq!(
            out.pruned,
            vec![PathBuf::from(".github/workflows/terramantle.yml")]
        );
    }

    #[test]
    fn plan_render_mentions_counts() {
        let desired = vec![
            managed(".github/workflows/terramantle.yml", "1"),
            once("main.tf", "2"),
        ];
        let plan = compute(&desired, fs(&[]));
        let r = plan.render();
        assert!(r.contains("2 to add"));
    }
}
