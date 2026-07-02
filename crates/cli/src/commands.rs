//! Command dispatch. Context/config/completion/version are wired for real in
//! this slice; every other command is a stub that prints
//! "<command>: not yet implemented" to stderr and exits 0 (SPEC §10 slice 1).

use std::io;

use clap::CommandFactory;
use clap_complete::generate;
use owo_colors::OwoColorize;

use tm_config::{resolve, Context, EnvOverrides, FlagOverrides, ResolvedConfig};

use crate::cli::{Cli, Command, ConfigCommand, ContextCommand, GlobalArgs, LockCommand};
use crate::output::{self, TableView};

/// Result carrying the process exit code to use.
pub type CmdResult = Result<i32, Box<dyn std::error::Error>>;

/// Turn the global flags into config-layer flag overrides.
fn flag_overrides(g: &GlobalArgs) -> FlagOverrides {
    FlagOverrides {
        api_url: g.api_url.clone(),
        org: g.org.clone(),
        workspace: g.workspace.clone(),
        context: g.context.clone(),
        output: g.output,
    }
}

/// Resolve the effective config (§4.1). `server_org` (layer 4) is `None` in this
/// slice — no network.
fn resolved(cli: &Cli) -> Result<ResolvedConfig, Box<dyn std::error::Error>> {
    let file = tm_config::ConfigFile::load()?;
    let env = EnvOverrides::from_env()?;
    let flags = flag_overrides(&cli.global);
    Ok(resolve(&file, &env, &flags, None)?)
}

pub fn dispatch(cli: &Cli) -> CmdResult {
    match &cli.command {
        Command::Context { command } => context(command, &cli.global),
        Command::Config { command } => config(command, cli),
        Command::Completion { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            generate(*shell, &mut cmd, name, &mut io::stdout());
            Ok(0)
        }
        Command::Version => {
            println!("terramantle {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Command::Providers { command } => crate::discovery::providers(command, cli),
        Command::Modules { command } => crate::discovery::modules(command, cli),
        Command::Lock { command } => lock(command, cli),
        Command::State { .. } => not_implemented("state"),
        Command::Auth { command } => crate::auth::dispatch(command, cli),
    }
}

/// Stub handler for commands landing in a later slice (§10).
fn not_implemented(name: &str) -> CmdResult {
    eprintln!("{name}: not yet implemented");
    Ok(0)
}

fn lock(command: &LockCommand, cli: &Cli) -> CmdResult {
    match command {
        LockCommand::Push {
            path,
            fail_on_atrisk,
            dry_run,
            repo_url,
            posture_timeout,
            require_posture,
        } => crate::lock::push(
            cli,
            &crate::lock::PushArgs {
                path,
                fail_on_atrisk: *fail_on_atrisk,
                dry_run: *dry_run,
                repo_url: repo_url.as_deref(),
                posture_timeout: *posture_timeout,
                require_posture: *require_posture,
            },
        ),
    }
}

fn context(command: &ContextCommand, global: &GlobalArgs) -> CmdResult {
    let path = tm_config::ConfigFile::default_path()?;
    let mut file = tm_config::ConfigFile::load_from(&path)?;

    match command {
        ContextCommand::Ls => {
            let override_name = global.context.as_deref();
            let current = file.active_context_name(override_name);
            let mut view = TableView::new(["current", "name", "org", "workspace"]);
            for (name, ctx) in &file.contexts {
                let marker = if Some(name.as_str()) == current {
                    "*"
                } else {
                    ""
                };
                view.row([
                    marker.to_string(),
                    name.clone(),
                    ctx.org.clone(),
                    ctx.workspace.clone().unwrap_or_default(),
                ]);
            }
            println!("{}", view.render());
            Ok(0)
        }
        ContextCommand::Current => match file.current_context.as_deref() {
            Some(name) => {
                println!("{name}");
                Ok(0)
            }
            None => {
                eprintln!("no current context set");
                Ok(1)
            }
        },
        ContextCommand::Use { name } => {
            if !file.contexts.contains_key(name) {
                eprintln!("unknown context '{name}' (create it with `terramantle context set {name} --org <org>`)");
                return Ok(1);
            }
            file.current_context = Some(name.clone());
            file.save_to(&path)?;
            eprintln!("switched to context '{name}'");
            Ok(0)
        }
        ContextCommand::Set {
            name,
            org,
            workspace,
        } => {
            let entry = file
                .contexts
                .entry(name.clone())
                .or_insert_with(Context::default);
            if let Some(org) = org {
                entry.org = org.clone();
            }
            if let Some(ws) = workspace {
                entry.workspace = Some(ws.clone());
            }
            if entry.org.is_empty() {
                eprintln!("context '{name}' has no org; pass --org <org>");
                return Ok(2);
            }
            file.save_to(&path)?;
            eprintln!("updated context '{name}'");
            Ok(0)
        }
    }
}

fn config(command: &ConfigCommand, cli: &Cli) -> CmdResult {
    match command {
        ConfigCommand::View => {
            let cfg = resolved(cli)?;
            // ResolvedConfig holds no secrets by design (tokens live in the
            // keyring), so nothing to redact here yet; the render path is the
            // redaction seam for later slices.
            if !output::print_structured(&cfg, cfg.output)? {
                render_config_table(&cfg, output::color_enabled(cli.global.no_color));
            }
            Ok(0)
        }
    }
}

fn render_config_table(cfg: &ResolvedConfig, color: bool) {
    let mut view = TableView::new(["key", "value"]);
    view.row(["api_url".to_string(), cfg.api_url.clone()]);
    view.row([
        "oidc_issuer".to_string(),
        cfg.oidc_issuer.clone().unwrap_or_else(dash),
    ]);
    view.row(["org".to_string(), cfg.org.clone().unwrap_or_else(dash)]);
    view.row([
        "workspace".to_string(),
        cfg.workspace.clone().unwrap_or_else(dash),
    ]);
    view.row(["output".to_string(), cfg.output.to_string()]);
    view.row([
        "context".to_string(),
        cfg.context.clone().unwrap_or_else(dash),
    ]);
    let rendered = view.render();
    if color {
        println!("{}", rendered.bold());
    } else {
        println!("{rendered}");
    }
}

fn dash() -> String {
    "—".to_string()
}
