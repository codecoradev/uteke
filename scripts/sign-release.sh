#!/usr/bin/env bash
# Sign the checksums of a published uteke release with the offline minisign key
# and upload the signature (#1327). Run on the maintainer's machine, never in CI.
#
#   scripts/sign-release.sh [--dry-run] [--force] vX.Y.Z
#
# Environment:
#   UTEKE_MINISIGN_KEY  secret key path (default: $HOME/.minisign/uteke-release.key)
#
# minisign prompts for the key password itself; this script never reads,
# prints or stores it.
set -euo pipefail

usage() {
  echo "usage: $0 [--dry-run] [--force] <tag>   (e.g. v0.22.0)" >&2
  exit 2
}

DRY_RUN=0
FORCE=0
TAG=""
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --force) FORCE=1 ;;
    -h | --help) usage ;;
    -*) echo "unknown option: $arg" >&2; usage ;;
    *) [ -z "$TAG" ] || usage; TAG="$arg" ;;
  esac
done
[ -n "$TAG" ] || usage
if ! [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.]+)?$ ]]; then
  echo "error: '$TAG' does not look like a release tag (vX.Y.Z)" >&2
  exit 2
fi

for tool in gh minisign; do
  command -v "$tool" >/dev/null 2>&1 || { echo "error: '$tool' is required" >&2; exit 1; }
done

REPO="codecoradev/uteke"
KEY="${UTEKE_MINISIGN_KEY:-$HOME/.minisign/uteke-release.key}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PUB="$SCRIPT_DIR/uteke-release.pub"
SIG_NAME="checksums-sha256.txt.minisig"

[ -f "$KEY" ] || { echo "error: secret key not found: $KEY (set UTEKE_MINISIGN_KEY)" >&2; exit 1; }
[ -f "$PUB" ] || { echo "error: public key not found: $PUB" >&2; exit 1; }

if gh release view "$TAG" --repo "$REPO" --json assets --jq '.assets[].name' | grep -qx "$SIG_NAME"; then
  if [ "$FORCE" -ne 1 ]; then
    echo "error: $TAG already has $SIG_NAME; use --force to replace it" >&2
    exit 1
  fi
  echo "warning: $TAG already has $SIG_NAME; it will be replaced (--force)" >&2
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> Downloading checksums-sha256.txt for $TAG"
gh release download "$TAG" --repo "$REPO" --pattern checksums-sha256.txt --dir "$WORK"

echo "==> Signing (minisign will ask for the key password)"
minisign -S -s "$KEY" -m "$WORK/checksums-sha256.txt" -t "uteke $TAG" -x "$WORK/$SIG_NAME"

echo "==> Verifying the fresh signature with $PUB"
minisign -V -p "$PUB" -m "$WORK/checksums-sha256.txt" -x "$WORK/$SIG_NAME"
echo "Trusted comment: $(sed -n 3p "$WORK/$SIG_NAME" | sed 's/^trusted comment: //')"

if [ "$DRY_RUN" -eq 1 ]; then
  echo "==> --dry-run: stopping before upload"
  exit 0
fi

printf 'Upload %s to release %s of %s? [y/N] ' "$SIG_NAME" "$TAG" "$REPO"
read -r answer
case "$answer" in
  y | Y | yes | YES) ;;
  *) echo "aborted"; exit 1 ;;
esac

UPLOAD_ARGS=()
if [ "$FORCE" -eq 1 ]; then UPLOAD_ARGS+=(--clobber); fi
gh release upload "$TAG" "$WORK/$SIG_NAME" --repo "$REPO" ${UPLOAD_ARGS[@]+"${UPLOAD_ARGS[@]}"}
echo "==> Uploaded. Check: gh release view $TAG --repo $REPO --json assets --jq '.assets[].name'"
