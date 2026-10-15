#!/usr/bin/env bash
# Open (and re-ping) a "Sign release <tag> (minisign)" issue while the latest
# stable release has no checksums-sha256.txt.minisig, and close it once the
# signature is on the release. Signing itself stays offline
# (scripts/sign-release.sh); this script only watches and nags, holds no key
# and reads no secret beyond the GitHub token gh already uses.
#
#   scripts/release-signature-reminder.sh [--dry-run]
#
# env: GH_REPO or GITHUB_REPOSITORY (owner/name), RELEASE_SIGNER (default
#      ajianaz), REMINDER_AFTER_HOURS (default 12), REMINDER_NOW_EPOCH (tests).
# bash 3.2 compatible.
set -euo pipefail

DRY=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY=1 ;;
    *) echo "usage: $0 [--dry-run]" >&2; exit 2 ;;
  esac
done

REPO="${GH_REPO:-${GITHUB_REPOSITORY:-}}"
[ -n "$REPO" ] || { echo "error: set GH_REPO or GITHUB_REPOSITORY" >&2; exit 2; }
SIGNER="${RELEASE_SIGNER:-ajianaz}"
HOURS="${REMINDER_AFTER_HOURS:-12}"
case "$HOURS" in ''|*[!0-9]*) echo "error: REMINDER_AFTER_HOURS must be an integer" >&2; exit 2 ;; esac
HOURS=$((10#$HOURS))   # "08" is a valid number here, not a bad octal
NOW="${REMINDER_NOW_EPOCH:-$(date +%s)}"
command -v gh >/dev/null 2>&1 || { echo "error: gh is required" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "error: jq is required" >&2; exit 2; }

# Run a mutating gh command, or only print it under --dry-run.
mutate() {
  # </dev/null: this runs inside a `while read` loop fed by a here-document, and a
  # command that reads stdin would swallow the rest of the list.
  if [ "$DRY" -eq 1 ]; then echo "[dry-run] gh $*"; else gh "$@" </dev/null; fi
}

# /releases/latest already excludes drafts and pre-releases; re-check anyway.
REL="$(gh api "repos/$REPO/releases/latest")"
TAG="$(printf '%s' "$REL" | jq -r '.tag_name // ""')"
IS_PRE="$(printf '%s' "$REL" | jq -r '.prerelease // false')"
IS_DRAFT="$(printf '%s' "$REL" | jq -r '.draft // false')"

if [ "$IS_PRE" != "false" ] || [ "$IS_DRAFT" != "false" ]; then
  echo "Latest release is a draft or pre-release; nothing to do."
  exit 0
fi
if ! printf '%s' "$TAG" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "Latest release tag is not a plain vX.Y.Z tag; nothing to do."
  exit 0
fi

has_asset() {
  printf '%s' "$REL" | jq -e --arg n "$1" '[.assets[]?.name] | index($n) != null' >/dev/null
}
if ! has_asset "checksums-sha256.txt.bundle"; then
  echo "$TAG has no checksums-sha256.txt.bundle (not signable by sign-release.sh); nothing to do."
  exit 0
fi
SIGNED=0
if has_asset "checksums-sha256.txt.minisig"; then SIGNED=1; fi

if [ "$SIGNED" -eq 1 ]; then echo "$TAG is signed"; else echo "$TAG is unsigned"; fi
