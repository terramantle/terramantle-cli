//! Configuration, env layering, kubectl-style contexts, and the precedence
//! resolver for the Terramantle CLI (SPEC §4).
//!
//! Precedence (highest wins), per §4.1:
//!   1. Explicit global flag (`--org`, `--workspace`, `--api-url`, `--output`, `--context`)
//!   2. `TERRAMANTLE_*` env var
//!   3. Selected context in the config file
//!   4. Token-derived server default (stubbed as `None` in this slice)
//!   5. Error with a precise remediation
//!
//! Config file model (§4.2): TOML with `current_context` and
//! `[contexts.<name>]` tables holding `org` + optional `workspace`. Never holds
//! secrets — tokens live in the OS keyring.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub const ENV_PREFIX: &str = "TERRAMANTLE_";
pub const DEFAULT_API_URL: &str = "https://registry.terramantle.dev";
pub const DEFAULT_OUTPUT: OutputFormat = OutputFormat::Table;

/// Output rendering format (`-o`/`TERRAMANTLE_OUTPUT`, §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Table,
    Wide,
    Json,
    Yaml,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            OutputFormat::Table => "table",
            OutputFormat::Wide => "wide",
            OutputFormat::Json => "json",
            OutputFormat::Yaml => "yaml",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for OutputFormat {
    type Err = ConfigError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "table" => Ok(OutputFormat::Table),
            "wide" => Ok(OutputFormat::Wide),
            "json" => Ok(OutputFormat::Json),
            "yaml" | "yml" => Ok(OutputFormat::Yaml),
            other => Err(ConfigError::BadOutput(other.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("no org configured; set --org, TERRAMANTLE_ORG, or select a context")]
    MissingOrg,
    #[error("unknown context '{0}'")]
    UnknownContext(String),
    #[error("invalid output format '{0}' (expected table|wide|json|yaml)")]
    BadOutput(String),
    #[error("could not locate a config directory for this platform")]
    NoConfigDir,
    #[error("failed to read config file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write config file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// A single named context (§4.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub org: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub workspace: Option<String>,
}

/// The on-disk config file model (§4.2). Never holds secrets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub current_context: Option<String>,
    #[serde(default)]
    pub contexts: BTreeMap<String, Context>,
}

impl ConfigFile {
    /// Path to the config file, `~/.config/terramantle/config.toml` (XDG),
    /// honouring `XDG_CONFIG_HOME` via the `directories` crate.
    pub fn default_path() -> Result<PathBuf, ConfigError> {
        let dirs = ProjectDirs::from("dev", "terramantle", "terramantle")
            .ok_or(ConfigError::NoConfigDir)?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    /// Load from an explicit path. A missing file yields the default (empty)
    /// config — the CLI is usable with no config file at all.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|source| ConfigError::Parse {
                path: path.to_path_buf(),
                source,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Load from the default XDG path.
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(&Self::default_path()?)
    }

    /// Persist to an explicit path, creating parent dirs.
    pub fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                path: path.to_path_buf(),
                source,
            })?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Persist to the default XDG path.
    pub fn save(&self) -> Result<(), ConfigError> {
        self.save_to(&Self::default_path()?)
    }

    /// The active context name, honouring an explicit `--context`/env override.
    pub fn active_context_name<'a>(&'a self, override_name: Option<&'a str>) -> Option<&'a str> {
        override_name.or(self.current_context.as_deref())
    }

    /// Resolve the active context, if any. An override naming an unknown context
    /// is an error; a `current_context` pointing at a missing entry is treated as
    /// "no context" (tolerant of a stale file).
    pub fn active_context(
        &self,
        override_name: Option<&str>,
    ) -> Result<Option<(&str, &Context)>, ConfigError> {
        match override_name {
            Some(name) => self
                .contexts
                .get_key_value(name)
                .map(|(k, v)| Some((k.as_str(), v)))
                .ok_or_else(|| ConfigError::UnknownContext(name.to_string())),
            None => Ok(self
                .current_context
                .as_deref()
                .and_then(|n| self.contexts.get_key_value(n))
                .map(|(k, v)| (k.as_str(), v))),
        }
    }
}

/// Values sourced from `TERRAMANTLE_*` environment variables. Split out so the
/// resolver is pure and unit-testable with no process env.
#[derive(Debug, Clone, Default)]
pub struct EnvOverrides {
    pub api_url: Option<String>,
    pub oidc_issuer: Option<String>,
    pub org: Option<String>,
    pub workspace: Option<String>,
    pub context: Option<String>,
    pub output: Option<OutputFormat>,
}

impl EnvOverrides {
    /// Read `TERRAMANTLE_*` vars from the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Read from an arbitrary lookup fn (unit tests inject a map here).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let var = |name: &str| get(&format!("{ENV_PREFIX}{name}"));
        let output = match var("OUTPUT") {
            Some(v) => Some(v.parse()?),
            None => None,
        };
        Ok(Self {
            api_url: var("API_URL"),
            oidc_issuer: var("OIDC_ISSUER"),
            org: var("ORG"),
            workspace: var("WORKSPACE"),
            context: var("CONTEXT"),
            output,
        })
    }
}

/// Global CLI flag overrides (§4.1 layer 1, highest precedence).
#[derive(Debug, Clone, Default)]
pub struct FlagOverrides {
    pub api_url: Option<String>,
    pub org: Option<String>,
    pub workspace: Option<String>,
    pub context: Option<String>,
    pub output: Option<OutputFormat>,
}

/// The fully-resolved, effective configuration for one CLI invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedConfig {
    pub api_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oidc_issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub output: OutputFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// Resolve the effective config from the precedence chain (§4.1).
///
/// `server_org` models layer 4 (the token-derived server default). This slice
/// has no network, so callers pass `None`; a later auth slice will supply the
/// `whoami` lookup result.
pub fn resolve(
    file: &ConfigFile,
    env: &EnvOverrides,
    flags: &FlagOverrides,
    server_org: Option<String>,
) -> Result<ResolvedConfig, ConfigError> {
    // Context selection is itself layered: flag > env > current_context.
    let context_override = flags.context.as_deref().or(env.context.as_deref());
    let active = file.active_context(context_override)?;
    let context_name = active.map(|(name, _)| name.to_string());
    let ctx = active.map(|(_, c)| c);

    // org: flag > env > context > server default > error.
    let org = flags
        .org
        .clone()
        .or_else(|| env.org.clone())
        .or_else(|| ctx.map(|c| c.org.clone()))
        .or(server_org);
    let org = Some(org.ok_or(ConfigError::MissingOrg)?);

    // workspace: flag > env > context (optional, no error if absent).
    let workspace = flags
        .workspace
        .clone()
        .or_else(|| env.workspace.clone())
        .or_else(|| ctx.and_then(|c| c.workspace.clone()));

    // api_url: flag > env > default.
    let api_url = flags
        .api_url
        .clone()
        .or_else(|| env.api_url.clone())
        .unwrap_or_else(|| DEFAULT_API_URL.to_string());

    // output: flag > env > default.
    let output = flags.output.or(env.output).unwrap_or(DEFAULT_OUTPUT);

    Ok(ResolvedConfig {
        api_url,
        oidc_issuer: env.oidc_issuer.clone(),
        org,
        workspace,
        output,
        context: context_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(org: &str, ws: Option<&str>) -> Context {
        Context {
            org: org.to_string(),
            workspace: ws.map(String::from),
        }
    }

    fn file_with_context() -> ConfigFile {
        let mut contexts = BTreeMap::new();
        contexts.insert("acme-prod".to_string(), ctx("acme", Some("prod")));
        ConfigFile {
            current_context: Some("acme-prod".to_string()),
            contexts,
        }
    }

    #[test]
    fn defaults_apply_when_only_org_present() {
        let file = ConfigFile::default();
        let env = EnvOverrides::default();
        let flags = FlagOverrides {
            org: Some("solo".into()),
            ..Default::default()
        };
        let r = resolve(&file, &env, &flags, None).unwrap();
        assert_eq!(r.api_url, DEFAULT_API_URL);
        assert_eq!(r.output, OutputFormat::Table);
        assert_eq!(r.org.as_deref(), Some("solo"));
        assert_eq!(r.workspace, None);
    }

    #[test]
    fn context_supplies_org_and_workspace() {
        let file = file_with_context();
        let env = EnvOverrides::default();
        let flags = FlagOverrides::default();
        let r = resolve(&file, &env, &flags, None).unwrap();
        assert_eq!(r.org.as_deref(), Some("acme"));
        assert_eq!(r.workspace.as_deref(), Some("prod"));
        assert_eq!(r.context.as_deref(), Some("acme-prod"));
    }

    #[test]
    fn env_beats_context() {
        let file = file_with_context();
        let env = EnvOverrides {
            org: Some("from-env".into()),
            ..Default::default()
        };
        let flags = FlagOverrides::default();
        let r = resolve(&file, &env, &flags, None).unwrap();
        assert_eq!(r.org.as_deref(), Some("from-env"));
        // workspace still falls through to the context.
        assert_eq!(r.workspace.as_deref(), Some("prod"));
    }

    #[test]
    fn flag_beats_env_and_context() {
        let file = file_with_context();
        let env = EnvOverrides {
            org: Some("from-env".into()),
            ..Default::default()
        };
        let flags = FlagOverrides {
            org: Some("from-flag".into()),
            ..Default::default()
        };
        let r = resolve(&file, &env, &flags, None).unwrap();
        assert_eq!(r.org.as_deref(), Some("from-flag"));
    }

    #[test]
    fn missing_org_is_an_error() {
        let file = ConfigFile::default();
        let env = EnvOverrides::default();
        let flags = FlagOverrides::default();
        let err = resolve(&file, &env, &flags, None).unwrap_err();
        assert!(matches!(err, ConfigError::MissingOrg));
    }

    #[test]
    fn server_default_used_only_as_last_resort() {
        let file = ConfigFile::default();
        let env = EnvOverrides::default();
        let flags = FlagOverrides::default();
        let r = resolve(&file, &env, &flags, Some("server-org".into())).unwrap();
        assert_eq!(r.org.as_deref(), Some("server-org"));

        // ...but an explicit flag still wins over the server default.
        let flags = FlagOverrides {
            org: Some("flag-org".into()),
            ..Default::default()
        };
        let r = resolve(&file, &env, &flags, Some("server-org".into())).unwrap();
        assert_eq!(r.org.as_deref(), Some("flag-org"));
    }

    #[test]
    fn context_override_selects_a_different_context() {
        let mut file = file_with_context();
        file.contexts.insert("personal".into(), ctx("rhys", None));
        let env = EnvOverrides::default();
        let flags = FlagOverrides {
            context: Some("personal".into()),
            ..Default::default()
        };
        let r = resolve(&file, &env, &flags, None).unwrap();
        assert_eq!(r.org.as_deref(), Some("rhys"));
        assert_eq!(r.context.as_deref(), Some("personal"));
    }

    #[test]
    fn unknown_context_override_errors() {
        let file = file_with_context();
        let env = EnvOverrides::default();
        let flags = FlagOverrides {
            context: Some("nope".into()),
            ..Default::default()
        };
        let err = resolve(&file, &env, &flags, None).unwrap_err();
        assert!(matches!(err, ConfigError::UnknownContext(_)));
    }

    #[test]
    fn output_precedence_flag_over_env() {
        let file = ConfigFile::default();
        let env = EnvOverrides {
            org: Some("o".into()),
            output: Some(OutputFormat::Yaml),
            ..Default::default()
        };
        let flags = FlagOverrides {
            output: Some(OutputFormat::Json),
            ..Default::default()
        };
        let r = resolve(&file, &env, &flags, None).unwrap();
        assert_eq!(r.output, OutputFormat::Json);
    }

    #[test]
    fn env_lookup_uses_terramantle_prefix() {
        let env = EnvOverrides::from_lookup(|k| match k {
            "TERRAMANTLE_ORG" => Some("x".to_string()),
            "TERRAMANTLE_OUTPUT" => Some("json".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(env.org.as_deref(), Some("x"));
        assert_eq!(env.output, Some(OutputFormat::Json));
    }

    #[test]
    fn config_file_roundtrips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let file = file_with_context();
        file.save_to(&path).unwrap();
        let loaded = ConfigFile::load_from(&path).unwrap();
        assert_eq!(loaded, file);
    }

    #[test]
    fn missing_config_file_loads_as_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        let loaded = ConfigFile::load_from(&path).unwrap();
        assert_eq!(loaded, ConfigFile::default());
    }
}
