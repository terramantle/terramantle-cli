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
    }
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
