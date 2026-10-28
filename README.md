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

- 🏗️ **Repo scaffolding** — `init`/`upgrade` scaffold a mono/poly repo for modules or workspaces, with GitHub **or** GitLab CI inferred from the `origin` remote. Terraform-style: `init` creates, `upgrade` diffs the desired layout against disk and applies the delta without clobbering your edits.
- 🔎 **Registry discovery** — `providers ls/show`, `modules search/show`, Trust verdicts inline.
- 🚀 **Publish & tag** — `modules changed/publish` (conventional-commit versioning, deterministic `tar.gz`), `repo tag` (mono `<module>@<version>` release tagging) and `state publish` (per-workspace lock-file upload + posture).
- 📦 **CI lock-file uploader** — `lock push` with eventually-consistent posture (`--fail-on-atrisk`).
- 🗄️ **State operations** — `state ls/versions/promote/rollback/unlock`, confirmations + `--force`.
- 🔐 **Five auth modes, zero hardcoding** — GitHub/GitLab OIDC, device login, portal bot tokens, raw token; every client id and endpoint is fetched from a discovery endpoint at runtime.
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
├── init                        # scaffold a repo (manifest + CI + skeleton)
├── upgrade                     # diff desired scaffold vs disk, apply the delta
├── providers
│   ├── ls                      # providers in use in the org (usage rollup + TRUST)
│   └── show <ns>/<type>        # versions + trust + used-by workspaces
├── modules
│   ├── search <query>          # registry search
│   ├── show <ns>/<name>/<provider>
│   ├── changed [--all]         # changed modules + proposed next semver (CI-consumable)
│   └── publish [--all] […]     # package · terraform-docs · hash · upload · tag
├── lock
│   └── push [path]             # upload .terraform.lock.hcl (default ./)
├── state
│   ├── ls                      # workspaces in the org
│   ├── versions <workspace>    # version history (serial, actor, pushed_at)
│   ├── promote <workspace> <versionId>       # restore a historical version to latest
│   ├── rollback <workspace> [--to <serial>]  # promote previous (or --to) serial
│   ├── unlock <workspace>      # force-unlock
│   └── publish [--all] […]     # upload each workspace's .terraform.lock.hcl + posture
├── repo
│   └── tag [--dry-run]         # mono: create + push <module>@<version> release tags
├── auth
│   ├── login                   # device flow (human) / auto in CI
│   ├── logout
│   ├── whoami                  # identity, org(s), token type + expiry
│   ├── token                   # print the bearer to stdout (for $(terramantle auth token))
│   └── env [--format …]        # shell exports for eval "$(terramantle auth env)"
├── console [path]              # open the portal in your default browser
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

## Scaffolding (`init` / `upgrade`)

`init` scaffolds a repository; `upgrade` re-scaffolds it idempotently. Both show
a plan and confirm before writing — the same create/plan/apply loop you know
from `terraform`.

`init` is a wizard: anything it cannot infer, it asks. Every question has a flag
override, and `--yes` (or a non-TTY) runs headless from flags plus what it can
detect.

```sh
cd my-infra-repo
terramantle init
#   · artefact 'workspaces' (from the directory layout)
#   + terramantle.hcl                    # the repo manifest (your source of truth)
#   + terramate.tm.hcl                   # stack orchestration + generated backends
#   + .github/workflows/terramantle.yml
#   + .github/CODEOWNERS
#   + .tflint.hcl
#   ~ .gitignore                         # terramantle ignore lines merged in
```

It will also offer to register this repo under your org's VCS trust. Without
that entry a correct pipeline still returns 401 on its first push, because the
missing state is server-side.

- **Artefact** — `modules` or `workspaces` (alias: `states`). Detected from the
  directory layout when unambiguous; `--artefact` to set it.
- **Structure** — `mono` (many independently-versioned artefacts,
  `<module>@<version>` tags) or `poly` (one artefact per repo).
- **VCS** — inferred from `git remote get-url origin`. A self-managed GitLab is
  detected and switched to `auth = bot`: the registry trusts only the literal
  `gitlab.com` and `token.actions.githubusercontent.com` issuers, so keyless
  OIDC there can never validate.
- **Terramate** — offered for workspaces repos. `terramate generate` then owns
  each stack's `_backend.tf`, and CI fails when the committed output is stale.
- **CI knobs** — `--ci-auth oidc|bot`, `--tofu 1.13.2`, `--no-terramate`,
  `--no-lint`, `--no-scan`, `--no-docs`, `--no-pr-comment`, `--default-branch`,
  `--runner`, `--versioning conventional|manual`. A disabled feature is omitted
  from the emitted pipeline, not left commented out.

Edit `terramantle.hcl`, then re-render:

```sh
terramantle upgrade --diff    # show the plan (+ ~ -), change nothing, exit 0
terramantle upgrade --check   # same, but exit 3 when drifted — for CI
terramantle upgrade           # apply after a confirmation prompt (--yes to skip)
```

> **0.9.0 changed the manifest and does not migrate.** A `terramantle.hcl`
> written by 0.8.x is refused by name (`ci.tofu` → `ci.engine`,
> `terraform_versions` → `tofu_versions`, `terraform_docs` → `docs`). Delete it
> and re-run `terramantle init`.

## Checking a repo (`doctor`)

Read-only. Every check here exists because its failure mode is silent or
misleading somewhere else.

```sh
terramantle doctor
#   [ok  ] scaffold               up to date with terramantle.hcl
#   [ok  ] terramate generate     generated code is up to date
#   [FAIL] lock files             1 workspace(s) missing .terraform.lock.hcl (stacks/data)
#   [FAIL] backend workspaces     address does not match the directory basename
#   [ok  ] vcs trust              repo:acme/infra is trusted
```

Exits 3 when anything failed. `--offline` skips the checks that call the
registry.

## VCS trust (`trust`)

Which repos the org accepts keyless CI tokens from. The stored subject is
matched against the CI token's `sub` claim.

```sh
terramantle trust ls
terramantle trust add                      # this repo, from its `origin` remote
terramantle trust add --subject repo:acme/infra --provider github
```

`upgrade` is a stateless re-render — **no lock file**. Ownership is carried by the
provenance header every managed file starts with (`# managed by terramantle …`):

- `+` create · `~` update a managed file that differs from the freshly rendered
  output · `-` prune a marked file no longer in the manifest (e.g. the old VCS's
  pipeline after you switch `vcs`).
- Skeleton files (`*.tf`, `README.md`) carry no header, so they are **yours after
  creation** — never rewritten or pruned. Managed files are overwritten on
  `upgrade` (the header says "do not edit above this line"); review the plan first
  with `--diff`.

## Publishing (`modules publish` / `state publish` / `repo tag`)

Repo-aware commands read `terramantle.hcl` and act per artefact. For **mono**
repos each `discovery` match is versioned independently and tagged
`<module>@<X.Y.Z>` (e.g. `vpc@1.4.0`); **poly** repos version the whole repo
(`vX.Y.Z`). A module's **first** release is always `<module>@1.0.0`; every release
after that is driven by Conventional Commits since the last tag.

**Change detection & versioning** — `modules changed` lists what changed since each
artefact's last tag and the proposed next semver from Conventional Commits
(`feat!`/`BREAKING CHANGE:` → major, `feat:` → minor, `fix:` → patch). It is
network-free and CI-consumable:

```sh
terramantle modules changed              # only the changed set
terramantle modules changed --all -o json
# [{ "name", "path", "last_version", "bump", "next_version", "changed_files": [...] }]
```

**`modules publish`** packages each target module deterministically (reproducible
`tar.gz`, zeroed mtimes → stable `sha256`), regenerates its README with
`terraform-docs` (if on `PATH`), and uploads the version to the registry:

```sh
terramantle modules publish --provider aws            # changed modules, computed versions
terramantle modules publish --module vpc --version 1.4.0 --provider aws
terramantle modules publish --all --bump minor --provider aws --dry-run
```

**`repo tag`** is the mono release-tagging step (runs on the default branch after
publish). It works out each module's next version from the `<module>@<semver>`
tags — first release `<module>@1.0.0`, thereafter Conventional Commits — packages
the module, then creates + pushes the `<module>@<version>` git tags. It skips
modules with no release-worthy change and never re-creates an existing tag, so it
is safe to re-run:

```sh
terramantle repo tag --dry-run   # show the tags that would be created
terramantle repo tag             # create + push <module>@<version> tags
```

> Publishing and tagging are separate steps: `modules publish` uploads the
> artefact; `repo tag` stamps the git tags. The scaffolded CI runs both, in order,
> on the default branch.

- **name/provider** — module `name` is the directory basename; `--provider` is
  required (the manifest carries no provider field).
- **version** — `--version X.Y.Z` (strict semver2; a bad value exits 2) or `--bump
  major|minor|patch`, else the conventional-commit computation; a released module
  with no bump-worthy commits is skipped.
- **`--dry-run`** packages/hashes and prints what *would* upload + tag, with no
  network and no tag; **`--ci`** is non-interactive + machine output.

**`state publish`** (workspace repos) fans out over `lock push`: for each workspace
dir carrying a `.terraform.lock.hcl` it uploads the lock file (with the `origin`
remote as provenance) and aggregates posture. `--fail-on-atrisk` gates on the
eventually-consistent Trust posture (exit 3):

```sh
terramantle state publish --all
terramantle state publish prod staging --fail-on-atrisk -o json
```

## Authentication

Auto-detected from the environment; override with `--auth-mode` /
`TERRAMANTLE_AUTH_MODE`. Before any auth, the CLI fetches
`{api_url}/.well-known/terramantle-cli.json` to discover the OIDC
issuer/audience/scopes, then the issuer's standard
`.well-known/openid-configuration` for the token and device endpoints — nothing
vendor-specific is hardcoded in the binary; any OIDC-compliant IdP works.

| Mode | Trigger | How |
|---|---|---|
| `github` | `GITHUB_ACTIONS=true` | Ambient GitHub OIDC id-token (needs `id-token: write`), sent as `Authorization: Bearer`. No static secret. |
| `gitlab` | `GITLAB_CI=true` | GitLab ID token from `TERRAMANTLE_ID_TOKEN`, same bearer trust path. |
| `device` | interactive TTY (`auth login`) | OIDC device flow; access/refresh stored in the OS keyring. |
| `client` | `TERRAMANTLE_CLIENT_ID`/`_SECRET` present | A **bot token** created in the portal (Settings → Tokens). See the note below — the two values are not an OAuth client id/secret. |
| `token` | `TERRAMANTLE_TOKEN` set | Raw bearer, verbatim; skips all flows (escape hatch / testing). A bearer that has **expired** falls through to the keyring and is refreshed, so `eval "$(terramantle auth env)"` twice in one shell doesn't strand you on a dead token. |

> **Bot tokens are not OAuth clients.** The IdP has no per-bot client: a bot
> token is a service account under one shared confidential provider. So the
> `client_credentials` grant sends `client_id` = that provider (discovered from
> `oidc.bot_client_id`, the same for every token in the deployment),
> `username` = your token's **Client ID** and `password` = its **Client
> Secret**. Sending the token's own id/secret as `client_id`/`client_secret` —
> the obvious reading, and what the CLI did before 0.9.0 — is rejected, because
> no OAuth client by that name exists. A registry that advertises no
> `oidc.bot_client_id` cannot do bot auth at all, and the CLI says so rather
> than failing at the token endpoint.

For **human** tokens the org defaults from `GET /api/orgs` when you belong to
exactly one org. For **CI OIDC / bot** tokens the org is server-resolved from repo
trust and there is no org endpoint, so `--org` / `TERRAMANTLE_ORG` is **required**
in CI.

> **Note:** `device` login requires the API operator to have provisioned a public
> device-flow client (`OIDC_CLI_CLIENT_ID`). If it isn't configured, `auth login`
> exits with a clear message — use a bot token or CI OIDC in the meantime.

### Dot-sourcing & eval

`auth token` prints **only** the bearer to stdout (refreshing if near expiry; all
narration goes to stderr), and `auth env` emits shell exports for `eval`:

```sh
export TERRAMANTLE_TOKEN=$(terramantle auth token)
eval "$(terramantle auth env)"                    # TOKEN (+ _API_URL, _ORG if resolved)
eval "$(terramantle auth env --terraform)"        # + TF_HTTP_PASSWORD, TF_TOKEN_<org>.<host>
terramantle auth env --format fish|powershell|json  # shell-correct quoting
terramantle auth env --write [--path ~/.config/terramantle/token.env]  # mode-600 dotenv
```

Both commands export `TERRAMANTLE_TOKEN`, which would otherwise put the *next*
invocation in `token` mode holding the previous bearer. They refresh a stale one
from the keyring instead, so re-running the `eval` after your 12-hour access
token expires just works. When the session genuinely can't be refreshed (a
revoked or expired refresh token) they fail with exit 5 and tell you to run
`auth login`, rather than handing Terraform a bearer that is already dead.

### `console`

```sh
terramantle console                        # open the portal root
terramantle console org/acme/modules       # open a specific page
terramantle console --print                # print the URL instead of launching
```

The portal origin comes from the deployment's discovery document
(`portal_url`), falling back to `https://portal.terramantle.dev`. No auth: the
portal runs its own sign-in. On a headless box the URL is always printed to
stderr, so a missing browser handler is a warning, not a failure.

## Environment variables

All `TERRAMANTLE_`-prefixed; URLs have defaults and are overridable.

| Var | Default | Purpose |
|---|---|---|
| `TERRAMANTLE_API_URL` | `https://registry.terramantle.dev` | API base |
| `TERRAMANTLE_OIDC_ISSUER` | discovered | OIDC issuer override |
| `TERRAMANTLE_AUDIENCE` | discovered | token audience override |
| `TERRAMANTLE_PORTAL_URL` | discovered | portal origin for `console` |
| `TERRAMANTLE_ORG` | — | org slug |
| `TERRAMANTLE_WORKSPACE` | — | default workspace |
| `TERRAMANTLE_CONTEXT` | — | context override |
| `TERRAMANTLE_TOKEN` | — | raw bearer; skip all auth flows |
| `TERRAMANTLE_CLIENT_ID` / `_CLIENT_SECRET` | — | a portal bot token's Client ID / Client Secret |
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

`upgrade --check` and `doctor` both use 3: a drifted scaffold is a gate result,
not a crash.

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
