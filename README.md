<div align="center">

# terramantle

**A resource-first CLI for the [Terramantle](https://terramantle.dev) registry** — discover modules & providers, push provider lock files from CI, and operate Terraform/OpenTofu state.

[![CI](https://github.com/terramantle/terramantle-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/terramantle/terramantle-cli/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/terramantle/terramantle-cli?sort=semver)](https://github.com/terramantle/terramantle-cli/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Status: experimental](https://img.shields.io/badge/status-experimental-orange.svg)](#-status--experimental)

</div>

---

> ### 🧪 Status — experimental
>
> **terramantle-cli is a helper tool, built as an experiment.** It's a convenience
> wrapper around Terramantle's public HTTP API — handy for discovery, CI lock-file
> uploads, and state operations — but it is **not** a core/officially-supported
> product and carries **no stability or support guarantees**. Commands, flags, and
> output may change without notice, and it's provided **as-is** (see [License](#license)).
> Everything it does, you can also do with `curl` against the API. Use it, fork it,
> break it — just don't build load-bearing production automation on it expecting
> long-term stability. Issues and PRs are welcome, best-effort.

## What it is

A single static binary, `terramantle`, that wraps the Terramantle registry API in
a `kubectl`/`helm`-style interface — **one tool, two modes** (interactive for
humans, non-interactive for CI), **one auth model**:

- **Discover** — browse modules and providers in use, with Trust Seal verdicts inline.
- **Push lock files** — upload `.terraform.lock.hcl` from a pipeline so Terramantle
  tracks which providers/versions are deployed (and can gate on them later).
- **Operate state** — list workspaces, inspect version history, promote, roll back,
  and force-unlock — with confirmations and permission-aware `--force`.

The feel: resource-first grammar, borderless tables, `-o json|yaml|wide`, coloured
TTY-aware status. Full design notes live in the
[spec](https://github.com/terramantle/terramantle/blob/main/docs/cli/SPEC.md).

## Features

- 🔎 **Registry discovery** — `providers ls/show`, `modules search/show`, Trust verdicts inline.
- 📦 **CI lock-file uploader** — `lock push` with eventually-consistent posture (`--fail-on-atrisk`).
- 🗄️ **State operations** — `state ls/versions/promote/rollback/unlock`, confirmations + `--force`.
- 🔐 **Five auth modes, zero hardcoding** — GitHub/GitLab OIDC, device login, bot client-credentials, raw token; OIDC config is fetched from a discovery endpoint at runtime.
- 🧭 **kubectl-style contexts** — switch org/workspace with `context use`.
- 🖥️ **Cross-platform** — macOS (arm64/x86_64), Linux (x86_64/arm64), Windows; static binaries + Homebrew + shell installer.
- 🧾 **Scriptable** — clean `-o json|yaml`, stable [exit codes](#exit-codes), `NO_COLOR`/non-TTY aware.

## Install

### Homebrew (recommended)

```sh
brew install terramantle/tap/terramantle
# or
brew tap terramantle/tap && brew install terramantle
```

### Shell installer

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/terramantle/terramantle-cli/releases/latest/download/cli-installer.sh | sh
```

### From source

```sh
git clone https://github.com/terramantle/terramantle-cli
cd terramantle-cli
cargo build --release
# binary at target/release/terramantle
```

Requires a stable Rust toolchain ([rustup](https://rustup.rs)).

## Quickstart

```sh
terramantle auth login             # device flow (human); auto-detected in CI
terramantle context set acme --org acme --workspace prod
terramantle context use acme       # kubectl-style: pick the default org/workspace
terramantle providers ls           # providers in use, with Trust verdicts inline
terramantle lock push              # upload ./.terraform.lock.hcl to the current workspace
```

## Commands

```
terramantle
├── providers
│   ├── ls                      # providers in use in the org (usage rollup + TRUST)
│   └── show <ns>/<type>        # versions + trust + used-by workspaces
├── modules
│   ├── search <query>          # registry search
│   └── show <ns>/<name>/<provider>
├── lock
│   └── push [path]             # upload .terraform.lock.hcl (default ./)
├── state
│   ├── ls                      # workspaces in the org
│   ├── versions <workspace>    # version history (serial, actor, pushed_at)
│   ├── promote <workspace> <versionId>       # restore a historical version to latest
│   ├── rollback <workspace> [--to <serial>]  # promote previous (or --to) serial
│   └── unlock <workspace>      # force-unlock
├── auth
│   ├── login                   # device flow (human) / auto in CI
│   ├── logout
│   └── whoami                  # identity, org(s), token type + expiry
├── context                     # kubectl-style org/workspace contexts
│   ├── ls · current · use <name> · set <name> [--org o] [--workspace w]
├── config
│   └── view                    # effective resolved config (secrets redacted)
├── completion <shell>          # bash | zsh | fish
└── version
```

Global flags on every command: `--org`, `--workspace`, `--api-url`, `--context`,
`-o/--output <table|wide|json|yaml>`, `--auth-mode`, `--no-color`, `-v/--verbose`.

### Shell completions

```sh
terramantle completion bash > /etc/bash_completion.d/terramantle
terramantle completion zsh  > "${fpath[1]}/_terramantle"
terramantle completion fish > ~/.config/fish/completions/terramantle.fish
```

## Authentication

Auto-detected from the environment; override with `--auth-mode` /
`TERRAMANTLE_AUTH_MODE`. Before any auth, the CLI fetches
`{api_url}/.well-known/terramantle-cli.json` to discover the OIDC issuer/audience —
nothing is hardcoded in the binary.

| Mode | Trigger | How |
|---|---|---|
| `github` | `GITHUB_ACTIONS=true` | Ambient GitHub OIDC id-token (needs `id-token: write`), sent as `Authorization: Bearer`. No static secret. |
| `gitlab` | `GITLAB_CI=true` | GitLab ID token from `TERRAMANTLE_ID_TOKEN`, same bearer trust path. |
| `device` | interactive TTY (`auth login`) | OIDC device flow; access/refresh stored in the OS keyring. |
| `client` | `TERRAMANTLE_CLIENT_ID`/`_SECRET` present | Client-credentials grant (bot). |
| `token` | `TERRAMANTLE_TOKEN` set | Raw bearer, verbatim; skips all flows (escape hatch / testing). |

For **human** tokens the org defaults from `GET /api/orgs` when you belong to
exactly one org. For **CI OIDC / bot** tokens the org is server-resolved from repo
trust and there is no org endpoint, so `--org` / `TERRAMANTLE_ORG` is **required**
in CI.

> **Note:** `device` login requires the API operator to have provisioned a public
> device-flow client (`OIDC_CLI_CLIENT_ID`). If it isn't configured, `auth login`
> exits with a clear message — use a bot token or CI OIDC in the meantime.

## Environment variables

All `TERRAMANTLE_`-prefixed; URLs have defaults and are overridable.

| Var | Default | Purpose |
|---|---|---|
| `TERRAMANTLE_API_URL` | `https://registry.terramantle.dev` | API base |
| `TERRAMANTLE_OIDC_ISSUER` | discovered | OIDC issuer override |
| `TERRAMANTLE_AUDIENCE` | discovered | token audience override |
| `TERRAMANTLE_ORG` | — | org slug |
| `TERRAMANTLE_WORKSPACE` | — | default workspace |
| `TERRAMANTLE_CONTEXT` | — | context override |
| `TERRAMANTLE_TOKEN` | — | raw bearer; skip all auth flows |
| `TERRAMANTLE_CLIENT_ID` / `_CLIENT_SECRET` | — | client-credentials (bot) |
| `TERRAMANTLE_ID_TOKEN` | — | GitLab CI OIDC id-token |
| `TERRAMANTLE_OUTPUT` | `table` | `table\|wide\|json\|yaml` |
| `TERRAMANTLE_AUTH_MODE` | `auto` | `auto\|github\|gitlab\|device\|client\|token` |
| `NO_COLOR` | — | standard; disables colour |

Colour is emitted only when stdout is a TTY and `NO_COLOR` is unset (or
`--no-color` given). Trust glyphs `✓ ▲ ✕` fall back to `OK / WARN / BLOCK` on
non-unicode / dumb terminals.

## Configuration & precedence

Config lives at the XDG path `~/.config/terramantle/config.toml` (kubectl-style
contexts; never holds secrets — tokens live in the OS keyring). Values resolve
highest-wins:

1. Explicit global flag (`--org`, `--workspace`, `--api-url`, `--context`, `-o/--output`)
2. `TERRAMANTLE_*` environment variable
3. The selected context in the config file
4. A token-derived server default (single-org humans only)
5. Otherwise: a precise error

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | generic / unexpected error |
| 2 | usage error (bad flags/args) |
| 3 | posture gate tripped (`--fail-on-atrisk`, future policy block) |
| 4 | confirmation required but refused / non-interactive without `--yes` |
| 5 | auth error (no/invalid/expired token, missing role) |
| 6 | not found (org / workspace / version) |
| 7 | state conflict — workspace locked / serial conflict (409) |

## Using the CLI in CI

Designed to run non-interactively and auto-detect the ambient OIDC identity. No
static secret is required for GitHub; GitLab needs an `id_tokens` block.

### GitHub Actions

```yaml
name: terraform-lock
on: [push]

jobs:
  push-lock:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      id-token: write          # required: lets terramantle fetch the ambient OIDC token
    steps:
      - uses: actions/checkout@v4

      - name: Install terramantle
        run: |
          curl --proto '=https' --tlsv1.2 -LsSf \
            https://github.com/terramantle/terramantle-cli/releases/latest/download/cli-installer.sh | sh

      # Auth mode auto-detects GITHUB_ACTIONS and uses the ambient OIDC token.
      - name: Push provider lock file
        env:
          TERRAMANTLE_ORG: acme          # required in CI — no org endpoint for OIDC tokens
        run: terramantle lock push --workspace prod --fail-on-atrisk
```

### GitLab CI

```yaml
push-lock:
  image: ubuntu:24.04
  id_tokens:
    TERRAMANTLE_ID_TOKEN:
      aud: https://registry.terramantle.dev   # must match TERRAMANTLE_AUDIENCE
  variables:
    TERRAMANTLE_ORG: acme                       # required in CI
  script:
    - apt-get update && apt-get install -y curl
    - |
      curl --proto '=https' --tlsv1.2 -LsSf \
        https://github.com/terramantle/terramantle-cli/releases/latest/download/cli-installer.sh | sh
    # Auth mode auto-detects GITLAB_CI and uses $TERRAMANTLE_ID_TOKEN as bearer.
    - terramantle lock push --workspace prod --fail-on-atrisk
```

## Development

```sh
cargo test   --all --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt    --all -- --check
```

CI (`.github/workflows/ci.yml`) enforces all four on push/PR. Run
`./scripts/install-hooks.sh` once after cloning to install a pre-commit hook that
runs fmt/clippy/test locally so a push can't fail CI on them. Bypass a single
commit with `git commit --no-verify`.

The workspace is four crates: `cli` (binary), `tm-config` (config/context
resolver), `tm-api` (typed API client), `tm-auth` (auth flows).

## Contributing

This is an experiment, so contributions are welcome but handled best-effort. Please
open an issue to discuss non-trivial changes first. Keep `fmt`/`clippy`/`test`
green (the pre-commit hook helps) and match the surrounding style.

## Releases

Releases are cut by [`cargo-dist`](https://github.com/axodotdev/cargo-dist). The
`[workspace.metadata.dist]` block in `Cargo.toml` declares the shell + Homebrew
installers, the tap (`terramantle/homebrew-tap`), and the target triples (Linux
x86_64/arm64 **gnu**, macOS arm64/x86_64, Windows x86_64).

`.github/workflows/release.yml` is **machine-generated** by `cargo dist generate`
from that config (not hand-authored). On a `v*` tag it builds all targets,
publishes a GitHub Release with checksummed artifacts, and pushes the generated
formula to `terramantle/homebrew-tap` under `Formula/terramantle.rb` — formulae are
never hand-edited. The `publish-homebrew-formula` job needs a `HOMEBREW_TAP_TOKEN`
Actions secret (PAT or GitHub App token) with `contents:write` on the tap; the
Release itself uses the auto-provided `GITHUB_TOKEN`.

## License

[MIT](./LICENSE) © Terramantle. Provided **as-is**, without warranty of any kind —
see the [experimental status](#-status--experimental) note above.
