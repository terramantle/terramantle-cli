//! `auth login/logout/whoami` command wiring (SPEC §5).
//!
//! Token type + expiry are decoded from the JWT locally; org(s) come from
//! `GET /api/orgs` only for human tokens. `/api/auth/me` is never called.
//! Tokens are never printed at any verbosity (rubric 7).

use tm_api::{ApiError, Client, OrgMembership};
use tm_auth::jwt::{self, TokenType};
use tm_auth::mode::{self, AuthMode};
use tm_auth::{AuthContext, AuthError};

use crate::cli::{AuthCommand, Cli};
use crate::commands::CmdResult;
use crate::output::{self, TableView};

/// Build the auth context from the resolved config + env/flag overrides,
/// resolving the effective auth mode. Public so discovery commands share one
/// context-construction path.
pub fn auth_context(cli: &Cli) -> Result<AuthContext, Box<dyn std::error::Error>> {
    let api_url = cli
        .global
        .api_url
        .clone()
        .or_else(|| std::env::var("TERRAMANTLE_API_URL").ok())
        .unwrap_or_else(|| tm_config::DEFAULT_API_URL.to_string());

    let override_mode = match &cli.global.auth_mode {
        Some(s) => AuthMode::parse_override(s)?,
        None => match std::env::var("TERRAMANTLE_AUTH_MODE") {
            Ok(s) => AuthMode::parse_override(&s)?,
            Err(_) => None,
        },
    };
    let detected = mode::detect(|k| std::env::var(k).ok(), override_mode);

    Ok(AuthContext {
        api_url,
        issuer_override: std::env::var("TERRAMANTLE_OIDC_ISSUER").ok(),
        audience_override: std::env::var("TERRAMANTLE_AUDIENCE").ok(),
        mode: detected,
    })
}

/// Build the shared `tm_api::Client` for an authed command. Delegates to
/// `tm_auth::client`, which resolves the bearer per §5 and fits the device flow
/// with its refresh-on-401 hook — never clone an `HttpClient`, a clone drops the
/// hook. Auth failures map to exit 5.
pub fn api_client(ctx: &AuthContext) -> Result<Client, Box<dyn std::error::Error>> {
    Ok(tm_auth::client(ctx)?)
}

/// Resolve the org via the §4 precedence (flag > env > context), **without**
/// erroring when absent — returns `None` so the caller can fall back to the
/// single-membership default (humans) or fail with a tailored message.
pub fn config_org(cli: &Cli) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let file = tm_config::ConfigFile::load()?;
    let env = tm_config::EnvOverrides::from_env()?;
    let context_override = cli.global.context.as_deref().or(env.context.as_deref());
    let active = file.active_context(context_override)?;
    let ctx_org = active.map(|(_, c)| c.org.clone());
    Ok(cli.global.org.clone().or(env.org).or(ctx_org))
}

/// Resolve the effective workspace via the §4 precedence
/// (`--workspace` > `TERRAMANTLE_WORKSPACE` > context default), without erroring
/// when absent — returns `None` so the caller can fail with a tailored message.
pub fn config_workspace(cli: &Cli) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let file = tm_config::ConfigFile::load()?;
    let env = tm_config::EnvOverrides::from_env()?;
    let context_override = cli.global.context.as_deref().or(env.context.as_deref());
    let active = file.active_context(context_override)?;
    let ctx_ws = active.and_then(|(_, c)| c.workspace.clone());
    Ok(cli.global.workspace.clone().or(env.workspace).or(ctx_ws))
}

/// Human-readable label for the resolved auth mode, for the step cadence line
/// (§6: "Authenticating (github oidc)").
pub fn mode_label(ctx: &AuthContext) -> &'static str {
    match ctx.mode {
        tm_auth::mode::AuthMode::GitHub => "github oidc",
        tm_auth::mode::AuthMode::GitLab => "gitlab oidc",
        tm_auth::mode::AuthMode::Device => "device",
        tm_auth::mode::AuthMode::Raw => "token",
        tm_auth::mode::AuthMode::ClientCredentials => "client",
    }
}

/// Log a diagnostic line at `-v` (mode + issuer only — never the token).
fn narrate_mode(cli: &Cli, ctx: &AuthContext) {
    if cli.global.verbose > 0 {
        eprintln!("auth mode: {:?}", ctx.mode);
        if let Some(iss) = &ctx.issuer_override {
            eprintln!("oidc issuer (override): {iss}");
        }
    }
}

pub fn dispatch(command: &AuthCommand, cli: &Cli) -> CmdResult {
    let ctx = auth_context(cli)?;
    narrate_mode(cli, &ctx);
    match command {
        AuthCommand::Login => login(&ctx),
        AuthCommand::Logout => logout(&ctx),
        AuthCommand::Whoami => whoami(&ctx, cli),
        AuthCommand::Token => token(&ctx),
        AuthCommand::Env {
            format,
            write,
            path,
            terraform,
        } => env_cmd(&ctx, cli, *format, *write, path.as_deref(), *terraform),
    }
}

// ── auth token / auth env (SCAFFOLD-PUBLISH-AUTH.md §4.2) ────────────────────────

/// `auth token`: resolve the bearer (auto-refreshing if near expiry, handled by
/// the resolver) and print **only** the token to stdout, so `$(terramantle auth
/// token)` is clean. Narration/errors go to stderr; an auth failure exits 5.
fn token(ctx: &AuthContext) -> CmdResult {
    match tm_auth::resolve_token(ctx) {
        Ok(t) => {
            println!("{t}");
            Ok(0)
        }
        Err(e) => Ok(auth_exit(&e)),
    }
}

/// `auth env`: emit `export TERRAMANTLE_TOKEN=…` (+ `_API_URL`, and `_ORG` when
/// resolved) so `eval "$(terramantle auth env)"` sets the environment. With
/// `--write` the same values are persisted to a mode-600 dotenv instead.
fn env_cmd(
    ctx: &AuthContext,
    cli: &Cli,
    format: EnvFormat,
    write: bool,
    path: Option<&str>,
    terraform: bool,
) -> CmdResult {
    let token = match tm_auth::resolve_token(ctx) {
        Ok(t) => t,
        Err(e) => return Ok(auth_exit(&e)),
    };

    let org = config_org(cli)?;
    let mut vars: Vec<(String, String)> = vec![
        ("TERRAMANTLE_TOKEN".to_string(), token.clone()),
        ("TERRAMANTLE_API_URL".to_string(), ctx.api_url.clone()),
    ];
    if let Some(org) = &org {
        vars.push(("TERRAMANTLE_ORG".to_string(), org.clone()));
    }
    if terraform {
        vars.extend(terraform_vars(&ctx.api_url, org.as_deref(), &token));
    }

    if write {
        // On-disk persistence is always a sourceable POSIX dotenv, regardless of
        // the print `--format` (§4.2: "a mode-600 dotenv the user can source").
        let target = dotenv_path(path)?;
        write_dotenv(&target, &vars)?;
        eprintln!("wrote {} (mode 600)", target.display());
        return Ok(0);
    }

    print!("{}", render_env(&vars, format));
    Ok(0)
}

/// Terraform/OpenTofu credential vars for `auth env --terraform`.
///
/// Emits (all set to the same resolved bearer):
///   - `TF_HTTP_PASSWORD` — for the `http` state backend's basic-auth password;
///   - `TF_TOKEN_<host>` — module-registry host credential (apex, e.g.
///     `registry.terramantle.dev`, which serves `modules.v1`);
///   - `TF_TOKEN_<org>.<host>` — the **provider** registry credential, keyed on
///     the org subdomain (`<slug>.registry.terramantle.dev`), which is the host
///     tofu actually authenticates to for providers. Only when an org resolves.
///
/// Host → env-var encoding follows Terraform's rule, not the server's current
/// consume-snippet (`[^a-z0-9]→_`): a dot becomes a single `_`, a hyphen becomes
/// a double `__`, so hyphenated org slugs resolve correctly (`my-org` →
/// `my__org`). See `tf_token_var`.
fn terraform_vars(api_url: &str, org: Option<&str>, token: &str) -> Vec<(String, String)> {
    let host = host_of(api_url);
    let mut out = vec![
        ("TF_HTTP_PASSWORD".to_string(), token.to_string()),
        (tf_token_var(&host), token.to_string()),
    ];
    if let Some(org) = org {
        out.push((tf_token_var(&format!("{org}.{host}")), token.to_string()));
    }
    out
}

/// Extract the bare hostname from an API base URL (`https://host:port/path` →
/// `host`). Falls back to the input unchanged if there is no scheme separator.
fn host_of(api_url: &str) -> String {
    let after_scheme = api_url.split("://").nth(1).unwrap_or(api_url);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    authority
        .rsplit_once('@')
        .map_or(authority, |(_, h)| h)
        .split(':')
        .next()
        .unwrap_or(authority)
        .to_string()
}

/// `TF_TOKEN_<host>` with Terraform's host encoding: hyphen → `__`, dot → `_`
/// (hyphens first so the dot pass doesn't touch the inserted underscores).
fn tf_token_var(host: &str) -> String {
    format!("TF_TOKEN_{}", host.replace('-', "__").replace('.', "_"))
}

/// The dotenv target path for `--write`: `--path` when given, else
/// `~/.config/terramantle/token.env`.
fn dotenv_path(path: Option<&str>) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    match path {
        Some(p) => Ok(std::path::PathBuf::from(p)),
        None => {
            let home = std::env::var("HOME").map_err(|_| "cannot resolve HOME for --write")?;
            Ok(std::path::Path::new(&home)
                .join(".config")
                .join("terramantle")
                .join("token.env"))
        }
    }
}

/// Write a POSIX `export`-style dotenv at mode 600 (best-effort chmod on unix).
fn write_dotenv(
    path: &std::path::Path,
    vars: &[(String, String)],
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let pairs: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    std::fs::write(path, render_env(&pairs, EnvFormat::Posix))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Shell dialect for `auth env` rendering (§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum EnvFormat {
    Posix,
    Fish,
    Powershell,
    Json,
}

/// Render env vars for the requested shell (pure — unit-tested). Values are quoted
/// per-dialect so `eval`/`source` round-trips even with spaces/quotes.
pub fn render_env<K: AsRef<str>, V: AsRef<str>>(vars: &[(K, V)], format: EnvFormat) -> String {
    match format {
        EnvFormat::Posix => vars
            .iter()
            .map(|(k, v)| format!("export {}={}\n", k.as_ref(), posix_quote(v.as_ref())))
            .collect(),
        EnvFormat::Fish => vars
            .iter()
            .map(|(k, v)| format!("set -gx {} {}\n", k.as_ref(), fish_quote(v.as_ref())))
            .collect(),
        EnvFormat::Powershell => vars
            .iter()
            .map(|(k, v)| format!("$env:{} = {}\n", k.as_ref(), ps_quote(v.as_ref())))
            .collect(),
        EnvFormat::Json => {
            let map: serde_json::Map<String, serde_json::Value> = vars
                .iter()
                .map(|(k, v)| {
                    (
                        k.as_ref().to_string(),
                        serde_json::Value::String(v.as_ref().to_string()),
                    )
                })
                .collect();
            format!(
                "{}\n",
                serde_json::to_string_pretty(&serde_json::Value::Object(map))
                    .unwrap_or_else(|_| "{}".to_string())
            )
        }
    }
}

/// POSIX single-quote: wrap in `'…'`, escaping embedded quotes as `'\''`.
fn posix_quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}

/// fish single-quote: only `\` and `'` are special inside `'…'`.
fn fish_quote(v: &str) -> String {
    format!("'{}'", v.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// PowerShell single-quote: a literal `'` is doubled to `''`.
fn ps_quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "''"))
}

fn login(ctx: &AuthContext) -> CmdResult {
    // In CI we acquire an ambient token and print the active identity rather
    // than writing to the keyring (§5: "no keyring write in CI").
    if matches!(ctx.mode, AuthMode::GitHub | AuthMode::GitLab) {
        match tm_auth::resolve_token(ctx) {
            Ok(token) => {
                print_identity(&token)?;
                Ok(0)
            }
            Err(e) => Ok(auth_exit(&e)),
        }
    } else {
        match tm_auth::login(ctx) {
            Ok(()) => {
                eprintln!("logged in; token stored in the OS keyring");
                Ok(0)
            }
            Err(e) => Ok(auth_exit(&e)),
        }
    }
}

fn logout(ctx: &AuthContext) -> CmdResult {
    match tm_auth::logout(&ctx.api_url) {
        Ok(()) => {
            eprintln!("logged out; stored token cleared");
            Ok(0)
        }
        Err(e) => Ok(auth_exit(&e)),
    }
}

/// `auth whoami`: decode the JWT locally, then list orgs for human tokens only.
fn whoami(ctx: &AuthContext, cli: &Cli) -> CmdResult {
    let token = match tm_auth::resolve_token(ctx) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(auth_exit(&e));
        }
    };

    let claims = match jwt::decode_claims(&token) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(1);
        }
    };
    let ttype = claims.token_type();

    // Human tokens: list orgs from GET /api/orgs. CI OIDC/bot: no org endpoint.
    let orgs = if ttype == TokenType::Human {
        match fetch_orgs(&ctx.api_url, &token) {
            Ok(o) => Some(o),
            Err(e) => {
                if let Some(code) = auth_status_exit(&e) {
                    eprintln!("error: {e}");
                    return Ok(code);
                }
                eprintln!("warning: could not list orgs: {e}");
                None
            }
        }
    } else {
        None
    };

    render_whoami(&claims, ttype, orgs.as_deref(), cli)
}

/// `GET /api/orgs` → memberships (human tokens only). Never calls `/api/auth/me`.
/// Delegates to the shared `tm_api::Client` so the model lives in one place.
fn fetch_orgs(api_url: &str, token: &str) -> Result<Vec<OrgMembership>, ApiError> {
    Client::new(api_url, token).orgs_list()
}

/// Map a 401/403 from an authed call to exit 5 (§9); other statuses fall
/// through so the caller can decide.
fn auth_status_exit(e: &ApiError) -> Option<i32> {
    match e.status() {
        Some(401) | Some(403) => Some(tm_auth::EXIT_AUTH),
        _ => None,
    }
}

fn auth_exit(e: &AuthError) -> i32 {
    eprintln!("error: {e}");
    e.exit_code()
}

/// Print the active identity (subject/issuer) for a token acquired in CI. Never
/// prints the token itself.
fn print_identity(token: &str) -> Result<(), Box<dyn std::error::Error>> {
    let claims = jwt::decode_claims(token)?;
    let sub = claims.sub.clone().unwrap_or_else(|| "—".into());
    let iss = claims.iss.clone().unwrap_or_else(|| "—".into());
    eprintln!("active identity: {sub} (issuer {iss})");
    Ok(())
}

#[derive(serde::Serialize)]
struct WhoamiJson<'a> {
    subject: Option<&'a str>,
    issuer: Option<&'a str>,
    audience: Option<String>,
    expiry: Option<i64>,
    token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    orgs: Option<&'a [OrgMembership]>,
}

fn render_whoami(
    claims: &jwt::Claims,
    ttype: TokenType,
    orgs: Option<&[OrgMembership]>,
    cli: &Cli,
) -> CmdResult {
    let format = cli.global.output.unwrap_or_default();
    let payload = WhoamiJson {
        subject: claims.sub.as_deref(),
        issuer: claims.iss.as_deref(),
        audience: claims.aud.as_ref().map(|a| a.display()),
        expiry: claims.exp,
        token_type: ttype.to_string(),
        orgs,
    };
    if output::print_structured(&payload, format)? {
        return Ok(0);
    }

    let mut view = TableView::new(["field", "value"]);
    view.row(["subject".to_string(), opt(claims.sub.as_deref())]);
    view.row(["issuer".to_string(), opt(claims.iss.as_deref())]);
    view.row([
        "audience".to_string(),
        claims
            .aud
            .as_ref()
            .map(|a| a.display())
            .unwrap_or_else(dash),
    ]);
    view.row(["expiry".to_string(), expiry_display(claims.exp)]);
    view.row(["type".to_string(), ttype.to_string()]);
    println!("{}", view.render());

    match orgs {
        Some(list) => {
            let mut orgview = TableView::new(["org", "role"]);
            for m in list {
                orgview.row([m.slug.clone(), m.role.clone()]);
            }
            println!("{}", orgview.render());
        }
        None => {
            if ttype != TokenType::Human {
                eprintln!("org resolved server-side — pass --org to target one");
            }
        }
    }
    Ok(0)
}

fn expiry_display(exp: Option<i64>) -> String {
    match exp {
        Some(ts) => ts.to_string(),
        None => dash(),
    }
}

fn opt(v: Option<&str>) -> String {
    v.map(str::to_string).unwrap_or_else(dash)
}

fn dash() -> String {
    "—".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vec<(&'static str, &'static str)> {
        vec![
            ("TERRAMANTLE_TOKEN", "abc.def"),
            ("TERRAMANTLE_API_URL", "https://reg.example"),
            ("TERRAMANTLE_ORG", "acme"),
        ]
    }

    #[test]
    fn posix_render_is_evalable() {
        let out = render_env(&vars(), EnvFormat::Posix);
        assert!(
            out.contains("export TERRAMANTLE_TOKEN='abc.def'\n"),
            "{out}"
        );
        assert!(
            out.contains("export TERRAMANTLE_API_URL='https://reg.example'\n"),
            "{out}"
        );
        assert!(out.contains("export TERRAMANTLE_ORG='acme'\n"), "{out}");
    }

    #[test]
    fn posix_escapes_single_quotes() {
        let out = render_env(&[("K", "a'b")], EnvFormat::Posix);
        assert_eq!(out, "export K='a'\\''b'\n");
    }

    #[test]
    fn fish_render_uses_set_gx() {
        let out = render_env(&[("K", "v")], EnvFormat::Fish);
        assert_eq!(out, "set -gx K 'v'\n");
        let esc = render_env(&[("K", "a'b\\c")], EnvFormat::Fish);
        assert_eq!(esc, "set -gx K 'a\\'b\\\\c'\n");
    }

    #[test]
    fn powershell_render_uses_env_prefix_and_doubles_quotes() {
        let out = render_env(&[("K", "a'b")], EnvFormat::Powershell);
        assert_eq!(out, "$env:K = 'a''b'\n");
    }

    #[test]
    fn host_of_strips_scheme_path_and_port() {
        assert_eq!(
            host_of("https://registry.terramantle.dev"),
            "registry.terramantle.dev"
        );
        assert_eq!(
            host_of("https://registry.terramantle.dev/api/v1"),
            "registry.terramantle.dev"
        );
        assert_eq!(host_of("http://localhost:8787/x"), "localhost");
        assert_eq!(
            host_of("registry.terramantle.dev"),
            "registry.terramantle.dev"
        );
    }

    #[test]
    fn tf_token_var_encodes_dots_and_hyphens() {
        // dots → single underscore
        assert_eq!(
            tf_token_var("registry.terramantle.dev"),
            "TF_TOKEN_registry_terramantle_dev"
        );
        // org subdomain
        assert_eq!(
            tf_token_var("acme.registry.terramantle.dev"),
            "TF_TOKEN_acme_registry_terramantle_dev"
        );
        // hyphen → double underscore (Terraform rule; the server snippet gets this wrong)
        assert_eq!(
            tf_token_var("my-org.registry.terramantle.dev"),
            "TF_TOKEN_my__org_registry_terramantle_dev"
        );
    }

    #[test]
    fn terraform_vars_apex_and_org_subdomain() {
        let v = terraform_vars("https://registry.terramantle.dev", Some("acme"), "tok");
        let keys: Vec<&str> = v.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "TF_HTTP_PASSWORD",
                "TF_TOKEN_registry_terramantle_dev",
                "TF_TOKEN_acme_registry_terramantle_dev",
            ]
        );
        assert!(v.iter().all(|(_, val)| val == "tok"));
    }

    #[test]
    fn terraform_vars_without_org_omits_provider_subdomain() {
        let v = terraform_vars("https://registry.terramantle.dev", None, "tok");
        let keys: Vec<&str> = v.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            vec!["TF_HTTP_PASSWORD", "TF_TOKEN_registry_terramantle_dev"]
        );
    }

    #[test]
    fn json_render_is_object() {
        let out = render_env(&vars(), EnvFormat::Json);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["TERRAMANTLE_TOKEN"], "abc.def");
        assert_eq!(v["TERRAMANTLE_ORG"], "acme");
    }
}
