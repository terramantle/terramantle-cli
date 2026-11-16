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
- 🧪 **Plan gate** — `plan push` checks your saved plan against the workspace's current state, sends it (binary + `show -json`) and prints the policy gate's verdict — PASS, WARN or BLOCKED — in the same round trip; `plan show` and `plan approve` are the review; `plan pull` downloads the gated binary, pinned to the sha-256 you expect, and hands it to `terraform apply`. The URL is derived from your backend `address`, nothing to configure.
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
├── plan
│   ├── push [tfplan] [--ref r] # pre-flight, render + binary -> <backend address>/plan; exit 3 if the gate blocks
│   ├── pull [--ref r|--id id] [--expect-sha h]  # download the gated binary (sha-256 verified) to apply
│   ├── ls [workspace]          # plan versions: counts, prior serial, binary?, scan, pushed
│   ├── show <workspace> <id>   # verdict, risk, indicators, impact, supply chain, policies, approvals
│   └── approve <workspace> <id> --reason "…"   # record your approval (human logins only)
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

> **0.11.0 changes the workspaces pipeline (template v3).** Plans are no longer
> passed from the plan job to the apply job as a CI artifact: each is pushed to
> the workspace's plan gate and pulled back, pinned to its sha-256 (see
> [Plans](#plans-plan-push--pull--show--approve)). `upgrade --check` reports a
> v2 pipeline as behind; `terramantle upgrade` re-renders it. The old pipeline
> keeps working until you do.
>
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

## Plans (`plan push` / `pull` / `show` / `approve`)

Terraform and OpenTofu push state to the backend but never push a plan. The CLI fills
that gap, so a plan is judged by the workspace's policy gate before it is applied and
the apply job fetches exactly the bytes that were judged:

```bash
terraform plan -out=tfplan
terramantle plan push                # no --ref: this plan gates the workspace
#   ==> Resolving backend
#       org acme · workspace prod · no ref (this plan gates the workspace)
#   ==> Rendering plan (tofu show -json)
#   ==> Checking the plan against the workspace's current state
#       current: made from serial 14, which is where the workspace is
#   ==> Pushing (github oidc)
#       version 0f8fad5b-… · prior serial 14 (from the plan)
#       binary sha256 f3bc91ef… (stored as sent)
#   ==> Verdict
#       ✕ BLOCKED by builtin
#         - 60% of the workspace destroyed or replaced exceeds the limit of 25%
#       ▲ 1 warning (not what blocked it)
#         - Risk score 62 is over the limit of 40
#       risk 62 of 100 · high
#         ✕ Share of the workspace destroyed or replaced: 60% (limit 25%)
#         ▲ Risk score: 62 (limit 40, warns only)
#       apply is refused for workspace `prod` (plan still works) until a passing plan is pushed or this plan is approved
#       to approve: terramantle plan approve prod 0f8fad5b-… --reason "…"

terramantle plan show prod 0f8fad5b-…                       # the review page, in the terminal
terramantle plan approve prod 0f8fad5b-… --reason "the two replaced instances are expected"

# …on the apply job:
terramantle plan pull --out tfplan --expect-sha "$(cat tfplan.sha256)"
terraform apply tfplan
```

**What a push means depends on `--ref`.**

| | pushed without `--ref` | pushed with `--ref pr-42` |
|---|---|---|
| Gets a verdict, and exits 3 when blocked | yes | yes |
| Blocked: refuses the workspace's next apply | **yes** — the state LOCK and the state write are refused; `plan` still works | no |
| Passing: lifts a block an earlier plan raised | yes, if pushed with its binary, current, not `-refresh-only` and not errored | no |
| Binary is pullable while blocked and unapproved | no | no |

A plan without a ref is the one the apply pipeline makes of what it is about to apply. A
plan with one is a pull request's: a failing PR must not stop an unrelated apply on the
main branch, and a passing PR must not lift a block the main plan raised. With "require a
passing plan" switched on for the org or workspace, an apply additionally needs the
latest un-ref'd, binary-backed plan to be passing (or approved) *and* computed from the
state as it is now.

- **Three verdicts, one of which stops anything.** `PASS`, `WARN`, `BLOCKED`. Advisory
  policies and limits in `warn` mode produce warnings: each is printed on its own line,
  the verdict is `WARN`, and the exit code is 0. Only `BLOCKED` exits 3. The verdict is
  synchronous — everything that gates is decided in the push, so there is nothing to
  wait for afterwards. The lines under it are the risk score and the indicators that are
  over a limit, could not be measured against one, or matter on their own (a class of
  resource lost, an exploited or blocked provider).
- **Pre-flight: is the plan current?** A saved plan is a zip whose `tfstate` entry is the
  state it was computed from. Before uploading, `push` reads that state's lineage and
  serial and compares the pair with the workspace's newest state version. With state in
  the workspace, both must match; with none, only a plan made against no state is
  current. A mismatch is refused **before anything is uploaded**, exit 7, with what
  happened and what to run next — a stale serial (state changed since the plan: re-plan)
  reads differently from a lineage mismatch (a different state altogether: check the
  backend address first). The second rule catches the case neither tool checks at apply
  time: a plan made against an empty state, for a workspace that has since gained one.
  If the workspace's state cannot be read the push goes ahead and the server makes the
  same check. `--allow-stale` skips the refusal and stores the plan for the record; it
  still cannot be applied.
- **The render must be of the binary.** The gate judges the render, apply runs the
  binary, and the server refuses a pair that does not describe the same changes
  (`plan_render_mismatch`, exit 2). Let `push` do the rendering and this cannot happen.
  After the upload the CLI checks the server stored the bytes it sent (`tfplan_sha256`);
  a difference is a hard error, exit 1.
- **Re-pushing.** Pushing the identical plan again returns the same version only if it
  had passed or been approved (the output says so). A plan that is still blocked is
  stored as a new version and judged again against the gate as it is now, so "fix the
  limit, re-run the push" works.
- **Render-only.** `--render-only --render plan.json` pushes just the `show -json` text
  (the passive-drift recipe). Such a plan can *raise* a block but can never lift one or
  satisfy "require a passing plan", and it is not pullable. `show -json` carries neither
  serial nor lineage, so pass `--prior-serial` / `--prior-lineage` to say what state it
  was made from; without them the CLI warns that the server cannot tell.
- **Pull.** Needs **write** access to the workspace's state, not read: the binary holds
  the whole prior state and every variable value, and its only use is applying. Without
  `--ref` it addresses the workspace's gating plan; `--ref pr-42` a pull request's;
  `--id` one version. The server refuses a plan that is blocked and not approved
  (exit 3, with the reason from `X-Terramantle-Refusal`) and one the state has moved
  past (exit 7, unless `--allow-stale`). The body's sha-256 must match the server's
  `X-Terramantle-Tfplan-Sha256`, and with `--expect-sha <sha256>` it must also be the
  plan you expected — the hash the plan job recorded with `sha256sum tfplan`. On either
  mismatch the exit is 1, nothing is written, and a plan already at the output path is
  removed so an older one cannot be applied in its place.
- **Show.** `plan show <workspace> <id>`: the verdict; the risk score and the factors it
  is made of; every indicator against its limit; how the plan was run and who produced
  it (verified only for a CI OIDC push); what it reaches beyond its change list
  (surviving resources that depend on something it removes, other workspaces that read
  this state); the providers and modules it runs with, naming the ones that are blocked,
  exploited, critical or unverified; each policy's result, with an advisory failure
  shown as `WARN`; and the approvals — how many of how many, who, when and why.
- **Approve.** `plan approve <workspace> <id> --reason "…"` records *your* approval of
  that exact version. Human logins only: a CI or bot token is refused before any request
  is made, because the pipeline that pushed a plan must not be able to wave it through.
  An org can require several approvers, a minimum role, and that the person who pushed a
  plan does not approve it; the output says where the plan stands (`1 of 2 approvals`),
  and exits 0 whenever the approval was recorded.
- **Where it goes.** The org and workspace are read from the stack's backend
  configuration — `.terraform/terraform.tfstate` after `init` (so `-backend-config`
  partials resolve), else the `terraform { backend "http" { address } }` block — and the
  plan surface is `<address>/plan`, the way terraform derives `lock_address`. Pass
  `--address` to override, `--chdir` to point at the stack. The backend's origin must be
  the API this CLI is configured for; a mismatch is an error (exit 2), not a silent push
  elsewhere.
- `TERRAMANTLE_TF_BIN` / `--tf-bin` picks the terraform-compatible binary used for
  `show -json` (default: `tofu`, then `terraform`, on `PATH`). It runs in `--chdir`, which
  must be where `init` ran.

**When a plan command is refused**, the first line says what it means for the command
you ran and carries the server's stable machine code; the lines under it say what to do,
and the last is the server's own message:

```text
error: this plan is out of date: the workspace's state changed after it was made (plan_stale)
    The plan was computed from state serial 4; workspace `prod` is now at serial 6.
    Something was applied, or the state was otherwise written, after this plan was made.
    `tofu apply` would refuse this plan as stale, so it cannot gate or be applied.
    Run `tofu plan -out=tfplan` again, so the plan is made from the current state, and push that.
    Nothing was uploaded. To store this plan for the record anyway, add --allow-stale; it still cannot be applied.
```

With `-o json` the same refusal is also written to stdout as a document a pipeline can
branch on — `error` is the contract, the prose is not:

```json
{ "ok": false, "error": "plan_stale", "status": null, "explanation": "…", "hint": ["…"], "message": null, "exit_code": 7 }
```

| `error` | from | exit | meaning |
|---|---|---|---|
| `plan_stale` | push pre-flight, `pushPlan`, pull | 7 | not computed from the workspace's current state: re-plan |
| `lineage_mismatch` | push pre-flight, `pushPlan` | 7 | computed from a different state altogether |
| `plan_render_mismatch` | `pushPlan` | 2 | the render and the binary are not the same plan |
| `invalid_tfplan` | push (local), `pushPlan` | 2 | the file is not a saved plan |
| `policy_blocked` | pull | 3 | blocked and not approved, or the org's kill switch |
| `plan_no_binary` | pull | 6 | the newest plan there was pushed render-only |
| `forbidden` | any | 5 | push/pull: no write access to state; approve: not an admin, or not a person |
| `approver_role_too_low` | approve | 5 | the org requires a higher role to approve |
| `self_approval` | approve | 5 | you pushed this plan and the org does not allow approving your own |
| `human_state_access_blocked` | push, pull | 5 | the org does not let people use the state backend directly |
| `tfplan_sha_mismatch` | push (local) | 1 | the server did not store the binary that was sent |
| `plan_sha_mismatch` | pull (local) | 1 | the download is not what the server's hash, or `--expect-sha`, says |

`status` is the HTTP status, and is `null` for a check the CLI made itself. A blocked
*push* is not a refusal: it is a successful push whose result has `"gate": "blocked"`,
alongside the server's whole response (`verdict`, `version_id`, `tfplan_sha256`,
`stale`, `deduped`, …) and, when they could be read back, `risk` and `indicators`.

### In a pipeline

`terramantle init` scaffolds this for workspaces repos; the shape is:

```bash
# pull request: a verdict that fails the check, and never touches the workspace
tofu plan -out=tfplan
terramantle plan push tfplan --ref "pr-${PR_NUMBER}"

# default branch, plan job: the plan that gates the apply
tofu plan -out=tfplan
terramantle plan push tfplan
sha256sum tfplan | cut -d' ' -f1 > tfplan.sha256     # hand ONLY this to the apply job

# default branch, apply job
terramantle plan pull --out tfplan --expect-sha "$(cat tfplan.sha256)"
tofu apply -lock-timeout=300s tfplan
```

The plan file never becomes a CI artifact — it holds the full configuration and every
value in cleartext, so it is stored with the state, behind the same access. In the
scaffolded pipelines a blocked plan on the default branch does not fail the plan job:
the apply job's pull is refused instead, so that once the plan is approved, re-running
the apply job applies *that* plan rather than planning again.

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
| 1 | generic / unexpected error; also a plan whose bytes fail a hash check (`plan push`: the server stored something else; `plan pull`: not the server's hash, or not `--expect-sha`) |
| 2 | usage error (bad flags/args), or input that is wrong in itself: a file that is not a saved plan (`invalid_tfplan`), a render that is not of the binary (`plan_render_mismatch`) |
| 3 | a gate said no: `--fail-on-atrisk`, a `plan push` the policy gate BLOCKED (warnings alone exit 0), or a `plan pull` of a blocked, unapproved plan (`policy_blocked`) |
| 4 | confirmation required but refused / non-interactive without `--yes` |
| 5 | auth error (no/invalid/expired token, missing role); for plans also: no write access to state on a pull, a machine identity or too low a role on `plan approve` (`approver_role_too_low`), approving a plan you pushed (`self_approval`) |
| 6 | not found (org / workspace / version), or no binary to pull (`plan_no_binary`) |
| 7 | state conflict — workspace locked, serial conflict, or a plan that was not computed from the workspace's current state (`plan_stale`, `lineage_mismatch`), whether the `plan push` pre-flight or the server noticed |

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
