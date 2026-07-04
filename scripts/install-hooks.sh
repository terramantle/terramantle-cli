#!/usr/bin/env bash
# Point git at the in-tree, version-controlled hooks. Run once after cloning.
# Reverse with: git config --unset core.hooksPath
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

git config core.hooksPath scripts/hooks
chmod +x scripts/hooks/* 2>/dev/null || true

echo "Installed git hooks (core.hooksPath=scripts/hooks)."
echo "The pre-commit hook runs cargo fmt/clippy/test. Bypass with: git commit --no-verify"
