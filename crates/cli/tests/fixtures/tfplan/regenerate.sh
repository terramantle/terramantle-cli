#!/usr/bin/env bash
#
# How the saved-plan fixtures in this directory were made, and how to remake them.
#
#   crates/cli/tests/fixtures/tfplan/regenerate.sh
#
# They are REAL saved plans, written by the real binaries, because the thing under
# test (`crates/cli/src/tfplan_zip.rs`) reads a file format neither tool documents:
# a zip whose `tfstate` entry is the prior state the plan was computed from. A
# hand-built zip would only prove the reader agrees with its author.
#
# Everything is offline. The configuration uses `terraform_data` only, which is
# the builtin provider (terraform.io/builtin/terraform) compiled into both
# binaries, so `init` downloads nothing and the plans hold no credential, no
# hostname and no value worth protecting.
#
#   tofu-fresh.tfplan        tofu, planned against NO state: the prior state has
#                            no lineage, which is the case neither tool stale-checks.
#   tofu-applied.tfplan      tofu, planned after one apply: lineage + serial 1.
#   terraform-fresh.tfplan   the same two cases written by terraform, which starts
#   terraform-applied.tfplan a new state at a different serial than tofu does.
#   expected.json            lineage and serial of each, read back out of the local
#                            state the plan was made from. The test compares against
#                            this file rather than hard-coded values, so a
#                            regeneration needs no edit to the test.
#
# Lineages are random per run, so regenerating changes every fixture AND
# expected.json together. Commit them together.

set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

config_v1='resource "terraform_data" "a" {
  input = "one"
}
'
# The second revision changes one resource and adds another, so the applied plan
# has something to do (an update and a create) instead of being a no-op.
config_v2='resource "terraform_data" "a" {
  input = "two"
}

resource "terraform_data" "b" {
  input = "added"
}
'

# pointer <state-file>: {"lineage": …, "serial": …} of a local state file.
pointer() {
  jq -c '{lineage: (if .lineage == "" then null else .lineage end), serial: .serial}' "$1"
}

make_pair() {
  local bin="$1" dir="$work/$1"
  mkdir -p "$dir"
  printf '%s' "$config_v1" > "$dir/main.tf"
  (
    cd "$dir"
    "$bin" init -input=false -no-color >/dev/null

    # 1. No state yet.
    "$bin" plan -input=false -no-color -out="$here/$bin-fresh.tfplan" >/dev/null

    # 2. Apply once (local backend, terraform.tfstate), then plan a change.
    "$bin" apply -input=false -no-color -auto-approve >/dev/null
    printf '%s' "$config_v2" > main.tf
    "$bin" plan -input=false -no-color -out="$here/$bin-applied.tfplan" >/dev/null
    pointer terraform.tfstate > "$work/$bin-applied.json"
  )
}

make_pair tofu
make_pair terraform

jq -n \
  --slurpfile t "$work/tofu-applied.json" \
  --slurpfile h "$work/terraform-applied.json" \
  --arg tofu "$(tofu version -json | jq -r .terraform_version)" \
  --arg terraform "$(terraform version -json | jq -r .terraform_version)" \
  '{
     made_with: {tofu: $tofu, terraform: $terraform},
     "tofu-fresh.tfplan":        {lineage: null, has_state: false},
     "tofu-applied.tfplan":      ($t[0] + {has_state: true}),
     "terraform-fresh.tfplan":   {lineage: null, has_state: false},
     "terraform-applied.tfplan": ($h[0] + {has_state: true})
   }' > "$here/expected.json"

ls -l "$here"
cat "$here/expected.json"
