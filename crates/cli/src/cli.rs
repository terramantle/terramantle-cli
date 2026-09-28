//! The clap (derive) command tree — resource-first grammar (SPEC §3).
//!
//! The full tree is defined here so `--help` and shell completions are complete
//! even for commands whose logic is stubbed in this slice.

use clap::{Args, Parser, Subcommand};
use clap_complete::Shell;
use tm_config::OutputFormat;
use tm_scaffold::{Artefact, CiAuth, Signing, Structure, VcsProvider, Versioning};

/// Terramantle CLI — discover the registry, push provider lock files, operate state.
#[derive(Debug, Parser)]
#[command(
    name = "terramantle",
    version,
    about,
    long_about = None,
    propagate_version = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

/// Global flags, available on every subcommand (§4.1 layer 1).
#[derive(Debug, Args, Clone, Default)]
#[command(next_help_heading = "Global options")]
pub struct GlobalArgs {
    /// Organization slug (overrides env/context).
    #[arg(long, global = true)]
    pub org: Option<String>,

    /// Default workspace (overrides env/context).
    #[arg(long, global = true)]
    pub workspace: Option<String>,

    /// API base URL.
    #[arg(long, global = true, value_name = "URL")]
    pub api_url: Option<String>,

    /// Config context to use for this invocation.
    #[arg(long, global = true, value_name = "NAME")]
    pub context: Option<String>,

    /// Output format.
    #[arg(short = 'o', long, global = true, value_name = "FORMAT")]
    pub output: Option<OutputFormat>,

    /// Authentication mode (auto|token|client|github|gitlab|device).
    #[arg(long, global = true, value_name = "MODE")]
    pub auth_mode: Option<String>,

    /// Disable coloured output.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Increase verbosity (repeatable).
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Scaffold a repo (manifest + CI + skeleton) for a chosen structure/artefact.
    Init(InitArgs),
    /// Re-scaffold: diff the desired layout against disk and apply the delta.
    Upgrade(UpgradeArgs),
    /// Providers in use in the org.
    Providers {
        #[command(subcommand)]
        command: ProvidersCommand,
    },
    /// Search and inspect registry modules.
    Modules {
        #[command(subcommand)]
        command: ModulesCommand,
    },
    /// Provider lock-file operations.
    Lock {
        #[command(subcommand)]
        command: LockCommand,
    },
    /// State workspace operations.
    State {
        #[command(subcommand)]
        command: StateCommand,
    },
    /// Authentication.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Manage org/workspace contexts (kubectl-style).
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },
    /// Configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Generate shell completion script.
    Completion {
        /// Shell to generate completions for.
        shell: Shell,
    },
    /// Print version information.
    Version,
}

#[derive(Debug, Subcommand)]
pub enum ProvidersCommand {
    /// List providers in use in the org (usage rollup).
    Ls {
        /// Only show at-risk providers.
        #[arg(long)]
        at_risk: bool,
    },
    /// Show versions, trust, and used-by workspaces for a provider.
    Show {
        /// `<ns>/<type>`, e.g. hashicorp/aws.
        provider: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ModulesCommand {
    /// Search the registry.
    Search {
        /// Search query.
        query: String,
        /// Results per page (default 20).
        #[arg(long, value_name = "N")]
        limit: Option<u64>,
        /// Follow pagination to exhaustion (capped at 500).
        #[arg(long)]
        all: bool,
    },
    /// Show a module.
    Show {
        /// `<ns>/<name>/<provider>`.
        module: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum LockCommand {
    /// Upload .terraform.lock.hcl (default ./).
    Push {
        /// Directory (or file) containing .terraform.lock.hcl.
        #[arg(default_value = ".")]
        path: String,
        /// Exit 3 if any pushed provider is at-risk.
        #[arg(long)]
        fail_on_atrisk: bool,
        /// Parse and show posture without uploading.
        #[arg(long)]
        dry_run: bool,
        /// Attribute the push to a git repo URL (X-Git-Repo-URL header).
        #[arg(long, value_name = "URL")]
        repo_url: Option<String>,
        /// Seconds to poll for posture before giving up (default 15).
        #[arg(long, value_name = "SECS", default_value_t = 15)]
        posture_timeout: u64,
        /// Treat unknown posture (poll timeout) as a failure (exit 3).
        #[arg(long)]
        require_posture: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum StateCommand {
    /// List workspaces in the org.
    Ls,
    /// Version history for a workspace.
    Versions {
        /// Workspace name.
        workspace: String,
    },
    /// Restore a historical version to latest.
    Promote {
        /// Workspace name.
        workspace: String,
        /// Version id to promote.
        version_id: String,
        /// Skip the confirmation prompt.
        #[arg(long, visible_alias = "force")]
        yes: bool,
    },
    /// Promote the previous (or --to) serial.
    Rollback {
        /// Workspace name.
        workspace: String,
        /// Target serial (defaults to the previous serial).
        #[arg(long, value_name = "SERIAL")]
        to: Option<u64>,
        /// Skip the confirmation prompt.
        #[arg(long, visible_alias = "force")]
        yes: bool,
    },
    /// Force-unlock a workspace.
    Unlock {
        /// Workspace name.
        workspace: String,
        /// Skip the confirmation prompt.
        #[arg(long, visible_alias = "force")]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Log in (device flow, or auto in CI).
    Login,
    /// Log out and clear stored tokens.
    Logout,
    /// Show the current identity.
    Whoami,
}

#[derive(Debug, Subcommand)]
pub enum ContextCommand {
    /// List contexts (`*` marks the current one).
    Ls,
    /// Show the current context name.
    Current,
    /// Switch the current context.
    Use {
        /// Context name.
        name: String,
    },
    /// Create or update a context.
    Set {
        /// Context name.
        name: String,
        /// Organization slug.
        #[arg(long)]
        org: Option<String>,
        /// Default workspace.
        #[arg(long)]
        workspace: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show the effective resolved config (secrets redacted).
    View,
}

/// `terramantle init` — scaffold a repo (SCAFFOLD-PUBLISH-AUTH.md §2). Each prompt
/// has a flag override; `--yes`/non-TTY runs headless from flags + detected
/// defaults. Enum values parse via each type's `FromStr` (e.g. `--artefact states`
/// is accepted as an alias for `workspaces`).
#[derive(Debug, Args)]
pub struct InitArgs {
    /// Repo layout: `mono` (many artefacts) or `poly` (one). Default: poly.
    #[arg(long)]
    pub structure: Option<Structure>,
    /// Artefact type: `modules` or `workspaces` (alias: `states`). Prompted if omitted.
    #[arg(long)]
    pub artefact: Option<Artefact>,
    /// VCS provider. Inferred from the `origin` remote when omitted.
    #[arg(long)]
    pub vcs: Option<VcsProvider>,
    /// Terraform version matrix (comma-separated), e.g. `--tf 1.7,1.9`.
    #[arg(long, value_delimiter = ',')]
    pub tf: Option<Vec<String>>,
    /// Also run pipelines against OpenTofu (default on; `--no-tofu` to drop).
    #[arg(long)]
    pub no_tofu: bool,
    /// Drop the tflint step.
    #[arg(long)]
    pub no_lint: bool,
    /// Drop the terramantle security-scan gate.
    #[arg(long)]
    pub no_scan: bool,
    /// Drop terraform-docs README regeneration.
    #[arg(long)]
    pub no_docs: bool,
    /// Module signing strategy: `cosign` (default) | `gpg` | `none`.
    #[arg(long)]
    pub sign: Option<Signing>,
    /// CI auth grant: `oidc` (default, keyless) | `bot` (client-credentials).
    #[arg(long = "ci-auth")]
    pub ci_auth: Option<CiAuth>,
    /// Versioning: `conventional` (default) | `manual`.
    #[arg(long)]
    pub versioning: Option<Versioning>,
    /// Assume defaults and never prompt (headless / CI).
    #[arg(long)]
    pub yes: bool,
}

/// `terramantle upgrade` — idempotent re-scaffold (SCAFFOLD-PUBLISH-AUTH.md §3).
#[derive(Debug, Args)]
pub struct UpgradeArgs {
    /// Print the plan and exit 0 without applying.
    #[arg(long)]
    pub diff: bool,
    /// Apply without the confirmation prompt.
    #[arg(long)]
    pub yes: bool,
    /// Overwrite user-edited (drifted) files instead of writing `.terramantle-new`.
    #[arg(long)]
    pub force: bool,
    /// Restrict the re-scaffold to one output category.
    #[arg(long, value_enum)]
    pub only: Option<OnlyFilter>,
}

/// `--only` category filter for `upgrade`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OnlyFilter {
    /// The CI pipeline + CODEOWNERS (the managed, re-rendered files).
    Ci,
    /// The artefact skeleton (scaffold-once files).
    Skeleton,
}
