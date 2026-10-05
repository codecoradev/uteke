#!/usr/bin/env bash
# Render integrations/hermes/ from the embedded hermes-uteke-memory templates.
#
# The committed package in integrations/hermes/ must stay byte-identical to
# crates/uteke-cli/assets/hermes-uteke-memory/*.tmpl (what `uteke init --agent
# hermes --memory-provider` writes). A guard test in init.rs enforces this at
# test time; this script is the human-facing way to fix drift.
#
# Usage:
#   scripts/render-hermes-integration.sh          # (re)write rendered files
#   scripts/render-hermes-integration.sh --check  # exit 1 on drift, print fix
set -euo pipefail
cd "$(dirname "$0")/.."

SRC_YAML="crates/uteke-cli/assets/hermes-uteke-memory/plugin.yaml.tmpl"
SRC_INIT="crates/uteke-cli/assets/hermes-uteke-memory/__init__.py.tmpl"
OUT_DIR="integrations/hermes"

fail=0

if [ "${1:-}" = "--check" ]; then
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    cp "$SRC_YAML" "$tmp/plugin.yaml"
    cp "$SRC_INIT" "$tmp/__init__.py"
    if ! cmp -s "$tmp/plugin.yaml" "$OUT_DIR/plugin.yaml"; then
        echo "DRIFT: $OUT_DIR/plugin.yaml is stale."
        fail=1
    fi
    if ! cmp -s "$tmp/__init__.py" "$OUT_DIR/__init__.py"; then
        echo "DRIFT: $OUT_DIR/__init__.py is stale."
        fail=1
    fi
    if [ "$fail" -ne 0 ]; then
        echo "Fix: run scripts/render-hermes-integration.sh and commit the result."
        exit 1
    fi
    echo "integrations/hermes is up to date with embedded templates."
    exit 0
fi

for src in "$SRC_YAML" "$SRC_INIT"; do
    if [ ! -f "$src" ]; then
        echo "Missing template: $src" >&2
        exit 1
    fi
done

mkdir -p "$OUT_DIR"
cp "$SRC_YAML" "$OUT_DIR/plugin.yaml"
cp "$SRC_INIT" "$OUT_DIR/__init__.py"
echo "Rendered $OUT_DIR/{plugin.yaml,__init__.py} from embedded templates."
