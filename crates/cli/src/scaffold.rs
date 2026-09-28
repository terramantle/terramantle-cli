//! `init` / `upgrade` command handlers (SCAFFOLD-PUBLISH-AUTH.md §2–§3).
//!
//! This is the thin shell over [`tm_scaffold`]: it resolves the manifest inputs
//! (flags → inference → wizard prompt → defaults), then delegates the actual
//! rendering and the terraform-plan-style diff/apply to the crate. All the
//! decision logic lives in `tm-scaffold` where it is unit-tested; here we only do
//! IO, prompting, and narration.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use tm_config::{resolve, ConfigFile, EnvOverrides, FlagOverrides, DEFAULT_API_URL};
use tm_scaffold::render::FileClass;
use tm_scaffold::{
    compute, desired_files, merge_gitignore, vcs, CiConfig, Discovery, Manifest, ScaffoldError,
    Structure,
};

use crate::cli::{Cli, InitArgs, OnlyFilter, UpgradeArgs};
use crate::commands::CmdResult;
use crate::confirm;
use crate::output::Style;

/// Read the on-disk content of a repo-root-relative path (for the diff engine).
fn disk_reader(root: &Path) -> impl Fn(&Path) -> Option<String> + '_ {
    move |rel: &Path| std::fs::read_to_string(root.join(rel)).ok()
}

/// Resolve the registry org from the standard precedence chain (flag/env/context).
fn resolve_org(cli: &Cli) -> Result<(String, String), ScaffoldError> {
    let file = ConfigFile::load().map_err(|_| ScaffoldError::MissingOrg)?;
    let env = EnvOverrides::from_env().map_err(|_| ScaffoldError::MissingOrg)?;
    let flags = FlagOverrides {
        api_url: cli.global.api_url.clone(),
        org: cli.global.org.clone(),
        workspace: cli.global.workspace.clone(),
        context: cli.global.context.clone(),
        output: cli.global.output,
    };
    let cfg = resolve(&file, &env, &flags, None).map_err(|_| ScaffoldError::MissingOrg)?;
    let org = cfg.org.ok_or(ScaffoldError::MissingOrg)?;
    Ok((org, cfg.api_url))
}

/// Prompt for a single line on a TTY; `None` if not interactive or on EOF.
fn prompt_line(question: &str) -> Option<String> {
    if !std::io::stdin().is_terminal() {
        return None;
    }
    let mut err = std::io::stderr();
    let _ = write!(err, "{question} ");
    let _ = err.flush();
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => {
            let t = line.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
    }
}

/// Build the manifest for `init` from flags, inference, and (on a TTY) prompts.
fn build_manifest(cli: &Cli, args: &InitArgs, root: &Path) -> Result<Manifest, ScaffoldError> {
    let (org, api_url) = resolve_org(cli)?;

    let structure = args.structure.unwrap_or(Structure::Poly);

    // Artefact: flag → prompt (TTY) → error.
    let artefact = match args.artefact {
        Some(a) => a,
        None if args.yes => {
            return Err(ScaffoldError::BadEnum {
                field: "artefact",
                value: "<missing>".into(),
                expected: "modules workspaces",
            })
        }
        None => match prompt_line("Artefact type? [modules/workspaces]:") {
            Some(s) => s.parse()?,
            None => {
                return Err(ScaffoldError::BadEnum {
                    field: "artefact",
                    value: "<missing>".into(),
                    expected: "modules workspaces",
                })
            }
        },
    };

    // VCS: flag → infer from origin → prompt (TTY) → error.
    let vcs_provider = match vcs::resolve(root, args.vcs) {
        Ok(v) => v,
        Err(ScaffoldError::VcsUndetermined) if !args.yes => {
            match prompt_line("VCS provider? [github/gitlab]:") {
                Some(s) => s.parse()?,
                None => return Err(ScaffoldError::VcsUndetermined),
            }
        }
        Err(e) => return Err(e),
    };

    let ci = CiConfig {
        tofu: !args.no_tofu,
        lint: !args.no_lint,
        security_scan: !args.no_scan,
        terraform_docs: !args.no_docs,
        sign: args.sign.unwrap_or(CiConfig::default().sign),
        auth: args.ci_auth.unwrap_or(CiConfig::default().auth),
        versioning: args.versioning.unwrap_or(CiConfig::default().versioning),
        terraform_versions: args
            .tf
            .clone()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| CiConfig::default().terraform_versions),
    };

    let discovery = match structure {
        Structure::Mono => Some(Discovery::default_for(artefact)),
        Structure::Poly => None,
    };

    // Only pin api_url in the manifest when it diverges from the discovery default.
    let manifest_api_url = if api_url == DEFAULT_API_URL {
        None
    } else {
        Some(api_url)
    };

    Ok(Manifest {
        structure,
        artefact,
        org,
        api_url: manifest_api_url,
        vcs: vcs_provider,
        discovery,
        ci,
    })
}

/// `terramantle init`.
pub fn init(cli: &Cli, args: &InitArgs) -> CmdResult {
    let root = std::env::current_dir()?;
    if !vcs::is_git_repo(&root) {
        return Err(Box::new(ScaffoldError::NotAGitRepo));
    }

    let manifest_path = root.join(Manifest::FILENAME);
    if manifest_path.exists() {
        return Err(Box::new(ScaffoldError::ManifestExists(manifest_path)));
    }

    let manifest = build_manifest(cli, args, &root)?;

    // Write the manifest (user-owned source of truth), then materialise the rest.
    std::fs::write(&manifest_path, manifest.render()).map_err(|source| ScaffoldError::Write {
        path: manifest_path.clone(),
        source,
    })?;

    let desired = desired_files(&manifest);
    let mut lock = tm_scaffold::LockFile::default();
    let plan = compute(&desired, &lock, disk_reader(&root));
    let outcome = plan.apply(&root, &mut lock, false)?;
    apply_gitignore(&root)?;
    lock.save(&root)?;

    let mut err = std::io::stderr();
    let _ = writeln!(
        err,
        "Initialised {structure} {artefact} repo for org '{org}' ({vcs}).",
        structure = manifest.structure,
        artefact = manifest.artefact,
        org = manifest.org,
        vcs = manifest.vcs,
    );
    let _ = writeln!(err, "  + {}", Manifest::FILENAME);
    narrate_outcome(&mut err, &outcome);
    let _ = writeln!(
        err,
        "\nNext: review {}, commit, and push. `terramantle upgrade` re-scaffolds after manifest edits.",
        Manifest::FILENAME
    );
    Ok(0)
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

    let mut desired = desired_files(&manifest);
    if let Some(only) = args.only {
        desired.retain(|f| match only {
            OnlyFilter::Ci => f.class == FileClass::Managed,
            OnlyFilter::Skeleton => f.class == FileClass::Once,
        });
    }

    let mut lock = tm_scaffold::LockFile::load(&root)?;
    let mut plan = compute(&desired, &lock, disk_reader(&root));
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

    let outcome = plan.apply(&root, &mut lock, args.force)?;
    apply_gitignore(&root)?;
    lock.save(&root)?;

    let mut err = std::io::stderr();
    narrate_outcome(&mut err, &outcome);
    if !outcome.drifted.is_empty() {
        let _ = writeln!(
            err,
            "\n{} file(s) were edited since generation and were NOT overwritten.",
            outcome.drifted.len()
        );
        let _ = writeln!(
            err,
            "Review the .terramantle-new side-cars and merge, or re-run with --force:"
        );
        for (orig, side) in &outcome.drifted {
            let _ = writeln!(err, "  ! {} → {}", orig.display(), side.display());
        }
    }
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
    if outcome.added.is_empty()
        && outcome.updated.is_empty()
        && outcome.pruned.is_empty()
        && outcome.drifted.is_empty()
    {
        let _ = writeln!(w, "  (nothing to do)");
    }
}
