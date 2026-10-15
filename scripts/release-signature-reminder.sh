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
case "${1-}" in
  "") ;;
  --dry-run) DRY=1 ;;
  *) echo "usage: $0 [--dry-run]" >&2; exit 2 ;;
esac

REPO="${GH_REPO:-${GITHUB_REPOSITORY:-}}"
[ -n "$REPO" ] || { echo "error: set GH_REPO or GITHUB_REPOSITORY" >&2; exit 2; }
SIGNER="${RELEASE_SIGNER:-ajianaz}"
HOURS="${REMINDER_AFTER_HOURS:-12}"
case "$HOURS" in ''|*[!0-9]*) echo "error: REMINDER_AFTER_HOURS must be an integer" >&2; exit 2 ;; esac
NOW="${REMINDER_NOW_EPOCH:-$(date +%s)}"
command -v gh >/dev/null 2>&1 || { echo "error: gh is required" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "error: jq is required" >&2; exit 2; }

# Run a mutating gh command, or only print it under --dry-run.
mutate() {
  if [ "$DRY" -eq 1 ]; then echo "[dry-run] gh $*"; else gh "$@"; fi
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

TITLE="Sign release $TAG (minisign)"
ISSUES="$(gh issue list --repo "$REPO" --state open --limit 100 \
  --search "\"$TITLE\" in:title" --json number,title,createdAt,comments)"
# Exact title match in jq, lowest number first; empty when none.
EXISTING="$(printf '%s' "$ISSUES" | jq -c --arg t "$TITLE" \
  '[.[] | select(.title == $t)] | sort_by(.number) | .[0] // empty')"
NUM=""
if [ -n "$EXISTING" ]; then NUM="$(printf '%s' "$EXISTING" | jq -r '.number')"; fi

if [ "$SIGNED" -eq 1 ]; then
  if [ -n "$NUM" ]; then
    echo "$TAG is signed; closing reminder #$NUM."
    mutate issue comment "$NUM" --repo "$REPO" --body "Signature found on the release, closing."
    mutate issue close "$NUM" --repo "$REPO"
  else
    echo "$TAG is signed and no reminder is open; nothing to do."
  fi
  exit 0
fi

if [ -z "$NUM" ]; then
  BODY="Release $TAG has been published but its checksums are not signed yet.

**Why:** \`uteke upgrade\` fails closed and refuses to install $TAG until \`checksums-sha256.txt.minisig\` is on the release (#1327).

**What to do** (on the maintainer's machine; needs \`gh\`, \`cosign\` and \`minisign\`):

\`\`\`bash
git pull
scripts/sign-release.sh $TAG
\`\`\`

The script verifies the release's cosign bundle, shows the checksums, then asks for the minisign key password itself and for a confirmation before uploading. The key lives in the password manager with an offline backup (AGENTS.md, Release procedure, step 6); it is never in CI or GitHub secrets.

This issue closes itself once \`checksums-sha256.txt.minisig\` is on the release. It is re-pinged at most every $HOURS hours until then."
  echo "$TAG is unsigned and no reminder is open; creating \"$TITLE\" (assignee $SIGNER)."
  if [ "$DRY" -eq 1 ]; then
    echo "[dry-run] gh issue create --repo $REPO --title \"$TITLE\" --assignee $SIGNER"
    printf '%s\n' "$BODY" | sed 's/^/[dry-run]   /'
  elif ! gh issue create --repo "$REPO" --title "$TITLE" --body "$BODY" --assignee "$SIGNER"; then
    echo "Creating with assignee $SIGNER failed; retrying without an assignee."
    gh issue create --repo "$REPO" --title "$TITLE" --body "$BODY"
  fi
  exit 0
fi

# Last activity = issue creation or the newest comment.
LAST="$(printf '%s' "$EXISTING" | jq -r \
  '([.createdAt] + [.comments[]?.createdAt]) | map(fromdateiso8601) | max')"
AGE=$(( NOW - LAST ))
if [ "$AGE" -lt $(( HOURS * 3600 )) ]; then
  echo "Reminder #$NUM is recent (last activity $(( AGE / 3600 ))h ago, interval ${HOURS}h); not commenting."
  exit 0
fi
PUB="$(printf '%s' "$REL" | jq -r '.published_at // .created_at // ""')"
UNSIGNED_H="?"
if [ -n "$PUB" ]; then
  PUB_EPOCH="$(printf '%s' "$PUB" | jq -R 'fromdateiso8601' 2>/dev/null || true)"
  [ -n "$PUB_EPOCH" ] && UNSIGNED_H=$(( (NOW - PUB_EPOCH) / 3600 ))
fi
echo "Reminder #$NUM is stale; pinging @$SIGNER."
mutate issue comment "$NUM" --repo "$REPO" --body "@$SIGNER release $TAG has been unsigned for about ${UNSIGNED_H} hours, so \`uteke upgrade\` still refuses it. Run \`git pull\` then \`scripts/sign-release.sh $TAG\`. This issue closes itself once the \`.minisig\` is on the release."
