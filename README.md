# terramantle

A single static binary, `terramantle`, for the [Terramantle](https://terramantle.dev)
registry: **discover** modules and providers (with Trust Seal verdicts inline),
push provider lock files from CI, and **operate state** (list, version history,
promote, rollback, force-unlock). One tool, two modes (human + CI), one auth
model. The feel is `kubectl`/`helm`: resource-first grammar, borderless tables,
`-o json|yaml|wide`, coloured status, TTY-aware.

> Status: early. This is **Slice 1** of the build plan — the workspace scaffold,
> the config/context precedence engine, and the `context`, `config view`,
> `completion`, and `version` commands. Registry/state commands and auth are
> stubbed (`… : not yet implemented`) and land in later slices. See the
> [full spec](https://github.com/terramantle/terramantle/blob/main/docs/cli/SPEC.md).

## Install

### Homebrew (recommended)

```sh
brew install terramantle/tap/terramantle
# or
brew tap terramantle/tap && brew install terramantle
```

### From source

```sh
git clone https://github.com/terramantle/terramantle-cli
cd terramantle-cli
cargo build --release
# binary at target/release/terramantle
```

Requires a stable Rust toolchain (install via [rustup](https://rustup.rs)).

## Usage

```sh
terramantle --help                 # full command tree
terramantle version
terramantle context set acme --org acme --workspace prod
terramantle context use acme
terramantle context ls             # `*` marks the current context
terramantle config view            # effective resolved config (secrets redacted)
terramantle config view -o json    # machine-readable
terramantle completion bash        # shell completion script
```

## Configuration & precedence

Config lives at the XDG path `~/.config/terramantle/config.toml`
(kubectl-style contexts; never holds secrets — tokens live in the OS keyring).

Values resolve highest-wins:

1. Explicit global flag (`--org`, `--workspace`, `--api-url`, `--context`, `-o/--output`)
2. `TERRAMANTLE_*` environment variable (e.g. `TERRAMANTLE_ORG`)
3. The selected context in the config file
4. A token-derived server default (single-org CI; later slice)
5. Otherwise: a precise error

Defaults: `api_url = https://registry.terramantle.dev`, `output = table`.

## Distribution & releases

Releases are cut by [`cargo-dist`](https://github.com/axodotdev/cargo-dist).
The `[workspace.metadata.dist]` block in `Cargo.toml` declares the shell +
Homebrew installers, the tap (`terramantle/homebrew-tap`), and the five target
triples (linux x86_64/arm64 musl, macOS arm64/x86_64, windows x86_64).

The release workflow (`.github/workflows/release.yml`) is **machine-generated**
by `cargo dist init` from that config and is not hand-authored. On a `v*` tag it
builds all targets, publishes a GitHub Release with checksummed artifacts, and
pushes the generated formula to `terramantle/homebrew-tap` under
`Formula/terramantle.rb`. Formulae are never hand-edited.

The release job needs a `HOMEBREW_TAP_TOKEN` Actions secret (PAT or GitHub App
install token) with `contents:write` on the tap repo. Add it to the CLI repo's
Actions secrets before tagging.

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

CI (`.github/workflows/ci.yml`) enforces all four on push/PR.

## License

MIT — see [LICENSE](LICENSE).
