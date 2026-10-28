//! `init` / `upgrade` command handlers (SCAFFOLD-PUBLISH-AUTH.md §2–§3).
//!
//! This is the thin shell over [`tm_scaffold`]: it resolves the manifest inputs
//! (flags → inference → wizard prompt → defaults), then delegates the actual
//! rendering and the terraform-plan-style diff/apply to the crate. All the
//! decision logic lives in `tm-scaffold` where it is unit-tested; here we only do
//! IO, prompting, and narration.

use std::io::Write;
use std::path::{Path, PathBuf};

use tm_config::{resolve, ConfigFile, EnvOverrides, FlagOverrides, DEFAULT_API_URL};
use tm_scaffold::render::desired_files_with;
use tm_scaffold::render::FileClass;
use tm_scaffold::{compute, merge_gitignore, Manifest, ScaffoldError};

use crate::cli::{Cli, InitArgs, OnlyFilter, UpgradeArgs};
use crate::commands::CmdResult;
use crate::confirm;
use crate::output::Style;
use crate::wizard;
use tm_api::Client;

/// Exit code for `upgrade --check` when the scaffold has drifted.
///
/// 3 is the established "a gate said no" code in this CLI (the at-risk posture
/// gate uses it), as distinct from 1 "something broke" and 2 "you called it
/// wrong". Drift is a gate result, not a failure.
const EXIT_DRIFTED: i32 = 3;

/// Whether the example artefact belongs in the desired set.
///
/// The skeleton is a starting point for an empty repo. In a populated one it is
/// worse than noise: an `example` stack is a *real* stack, so CI would plan and
/// apply it alongside the user's own.
///
/// `init` and `upgrade` must agree on this. When they did not, `upgrade --check`
/// reported drift on a repo `init` had just scaffolded correctly — the desired
/// set differed between the command that wrote the files and the one that
/// verifies them.
///
/// Discovery comes from the release crate so "already has artefacts" means
/// exactly what `publish` and `modules changed` mean by it.
fn skeleton_wanted(root: &Path, manifest: &Manifest) -> bool {
    let populated = tm_release::artefacts(root, manifest)
        .map(|a| {
            a.iter()
                .any(|art| art.rel_dir != "." && root.join(&art.rel_dir).is_dir())
        })
        .unwrap_or(false);
    !populated
}

/// Read the on-disk content of a repo-root-relative path (for the diff engine).
fn disk_reader(root: &Path) -> impl Fn(&Path) -> Option<String> + '_ {
    move |rel: &Path| std::fs::read_to_string(root.join(rel)).ok()
}

/// `terramantle init`.
///
/// The shape mirrors `upgrade` deliberately: resolve inputs, compute a plan,
/// show it, confirm, apply. v1 `init` wrote files with no preview while
/// `upgrade` previewed and prompted, which made the safer-looking command the
/// surprising one.
pub fn init(cli: &Cli, args: &InitArgs) -> CmdResult {
    let root = std::env::current_dir()?;

    // Before anything else: a manifest here means the repo is already scaffolded.
    let manifest_path = root.join(Manifest::FILENAME);
    if manifest_path.exists() {
        return Err(Box::new(ScaffoldError::ManifestExists(manifest_path)));
    }

    // Offers `git init` rather than dead-ending, but still runs first: every
    // later step reads git.
    wizard::ensure_git_repo(&root, args)?;

    // Org resolution has two halves. The config chain (flag > env > context) is
    // offline and always available; the membership list needs a token, so a
    // failure to build a client is not fatal — it just means the wizard cannot
    // offer a picker and will ask for --org instead.
    let (configured_org, api_url) = config_org_and_api(cli);

    // Built at most once, and only when something actually needs it. Scaffolding
    // is an offline operation; only the org picker and the trust offer talk to
    // the registry, so `init --org … --no-trust` must not pay for a network
    // round trip (or stall behind an unreachable one).
    let client: std::cell::OnceCell<Option<Client>> = std::cell::OnceCell::new();
    let mut lazy_client = || {
        client
            .get_or_init(|| crate::discovery::client_for_wizard(cli))
            .as_ref()
    };

    let manifest = wizard::build(cli, args, &root, configured_org, api_url, &mut lazy_client)?;

    let want_skeleton = skeleton_wanted(&root, &manifest);
    if !want_skeleton {
        let _ = writeln!(
            std::io::stderr(),
            "  · existing artefacts found — skipping the example skeleton"
        );
    }

    // The manifest is an input, not an output, so it is not part of the plan;
    // show it alongside so the preview accounts for every file init writes.
    let desired = desired_files_with(&manifest, want_skeleton);
    let plan = compute(&desired, disk_reader(&root));

    let mut err = std::io::stderr();
    let _ = writeln!(
        err,
        "\n{structure} {artefact} repo for org '{org}' ({vcs}{terramate}).",
        structure = manifest.structure,
        artefact = manifest.artefact,
        org = manifest.org,
        vcs = manifest.vcs,
        terramate = if manifest.terramate.is_some() {
            " + terramate"
        } else {
            ""
        },
    );
    print!("  + {}    create\n{}", Manifest::FILENAME, plan.render());

    let style = Style::detect(cli.global.no_color);
    match confirm::confirm("Write these files?", args.yes, style) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("aborted; no changes applied.");
            return Ok(0);
        }
        Err(code) => return Ok(code),
    }

    std::fs::write(&manifest_path, manifest.render()).map_err(|source| ScaffoldError::Write {
        path: manifest_path.clone(),
        source,
    })?;
    let outcome = plan.apply(&root)?;
    apply_gitignore(&root)?;

    let _ = writeln!(err, "  + {}", Manifest::FILENAME);
    narrate_outcome(&mut err, &outcome);

    // Registering trust is what stops the first push 401ing, so it is offered
    // here rather than left to be discovered from a failed pipeline.
    if !args.no_trust {
        if let Some(c) = lazy_client() {
            crate::trust::offer(c, &manifest, &root, args.yes);
        }
    }

    next_steps(&mut err, &manifest);
    Ok(0)
}

/// The config-chain half of org resolution: offline, and never fatal here —
/// the wizard decides what to do when there is no org.
fn config_org_and_api(cli: &Cli) -> (Option<String>, String) {
    let flags = FlagOverrides {
        api_url: cli.global.api_url.clone(),
        org: cli.global.org.clone(),
        workspace: cli.global.workspace.clone(),
        context: cli.global.context.clone(),
        output: cli.global.output,
    };
    let file = ConfigFile::load().unwrap_or_default();
    let env = EnvOverrides::from_env().unwrap_or_default();
    match resolve(&file, &env, &flags, None) {
        Ok(cfg) => (cfg.org, cfg.api_url),
        Err(_) => (cli.global.org.clone(), DEFAULT_API_URL.to_string()),
    }
}

/// What the user has to do next that the scaffolder could not do for them.
fn next_steps<W: Write>(w: &mut W, m: &Manifest) {
    let _ = writeln!(w, "\nNext:");
    if m.terramate.is_some() {
        let _ = writeln!(
            w,
            "  1. `terramate generate` to write each stack's _backend.tf, then commit it.\n     \
             CI fails if the committed output does not match terramate.tm.hcl."
        );
    }
    if m.ci.auth == tm_scaffold::CiAuth::Bot {
        let _ = writeln!(
            w,
            "  · auth=bot: create a bot token in the portal (Settings → Tokens), then set\n    \
             its Client ID and Client Secret as the CI secrets TERRAMANTLE_CLIENT_ID\n    \
             and TERRAMANTLE_CLIENT_SECRET."
        );
    }
    let _ = writeln!(
        w,
        "  · review {}, commit, and push. `terramantle upgrade` re-scaffolds after edits,\n    \
         and `terramantle doctor` checks the repo is wired up correctly.",
        Manifest::FILENAME
    );
}

/// `terramantle upgrade`.
pub fn upgrade(cli: &Cli, args: &UpgradeArgs) -> CmdResult {
    let root = std::env::current_dir()?;
    let manifest_path = root.join(Manifest::FILENAME);
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Box::new(ScaffoldError::ManifestMissing(manifest_path)))
        }
        Err(source) => {
            return Err(Box::new(ScaffoldError::Read {
                path: manifest_path,
                source,
            }))
        }
    };
    let manifest = Manifest::parse(&text)?;

    let mut desired = desired_files_with(&manifest, skeleton_wanted(&root, &manifest));
    if let Some(only) = args.only {
        desired.retain(|f| match only {
            OnlyFilter::Ci => f.class == FileClass::Managed,
            OnlyFilter::Skeleton => f.class == FileClass::Once,
        });
    }

    let mut plan = compute(&desired, disk_reader(&root));
    // With --only we scope to a subset, so suppress prune (the excluded managed
    // files would otherwise look "no longer desired").
    if args.only.is_some() {
        plan.entries
            .retain(|e| e.action != tm_scaffold::Action::Prune);
    }

    // The plan is the primary output → stdout; narration/prompts → stderr.
    print!("{}", plan.render());

    if plan.is_empty_of_changes() {
        return Ok(0);
    }
    // `--check` is `--diff` with a verdict: CI needs a non-zero exit to gate on
    // a stale scaffold, and `--diff` has always exited 0 whatever it printed.
    if args.check {
        eprintln!("scaffold is out of date — run `terramantle upgrade` and commit the result.");
        return Ok(EXIT_DRIFTED);
    }
    if args.diff {
        return Ok(0);
    }

    let style = Style::detect(cli.global.no_color);
    match confirm::confirm("Apply these changes?", args.yes, style) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("aborted; no changes applied.");
            return Ok(0);
        }
        Err(code) => return Ok(code),
    }

    let outcome = plan.apply(&root)?;
    apply_gitignore(&root)?;

    let mut err = std::io::stderr();
    narrate_outcome(&mut err, &outcome);
    Ok(0)
}

/// Ensure the repo's `.gitignore` carries the terramantle ignore lines.
fn apply_gitignore(root: &Path) -> Result<(), ScaffoldError> {
    let path = root.join(".gitignore");
    let existing = std::fs::read_to_string(&path).ok();
    if let Some(new) = merge_gitignore(existing.as_deref()) {
        std::fs::write(&path, new).map_err(|source| ScaffoldError::Write { path, source })?;
    }
    Ok(())
}

/// Narrate an apply outcome to `w` (stderr).
fn narrate_outcome<W: Write>(w: &mut W, outcome: &tm_scaffold::ApplyOutcome) {
    let mut lines = |glyph: char, paths: &[PathBuf]| {
        for p in paths {
            let _ = writeln!(w, "  {glyph} {}", p.display());
        }
    };
    lines('+', &outcome.added);
    lines('~', &outcome.updated);
    lines('-', &outcome.pruned);
    if outcome.added.is_empty() && outcome.updated.is_empty() && outcome.pruned.is_empty() {
        let _ = writeln!(w, "  (nothing to do)");
    }
}
