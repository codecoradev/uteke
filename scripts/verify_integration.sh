#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────────
# verify_integration.sh — Smoke-test the Uteke CLI + Python wrapper.
#
# Exits 0 on success, 1 on failure.
# Can be used as a daily validation / CI check.
#
# Requirements:
#   - Rust toolchain (cargo)
#   - Python 3.8+
#   - No external Python packages needed
# ──────────────────────────────────────────────────────────────────────────────
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARY="$PROJECT_ROOT/target/release/uteke"
TEST_STORE=""
FAILED=0

cleanup() {
    if [ -n "$TEST_STORE" ] && [ -d "$TEST_STORE" ]; then
        rm -rf "$TEST_STORE"
    fi
}
trap cleanup EXIT

log()  { printf "[verify] %s\n" "$*"; }
fail() { printf "[verify] FAIL: %s\n" "$*" >&2; FAILED=1; }

# ── 1. Build ─────────────────────────────────────────────────────────────────

log "Building uteke (release)..."
if ! cargo build --release --manifest-path "$PROJECT_ROOT/Cargo.toml" 2>/dev/null; then
    fail "cargo build --release failed"
    exit 1
fi

if [ ! -x "$BINARY" ]; then
    fail "Binary not found at $BINARY"
    exit 1
fi
log "Binary: $BINARY"

# ── 2. CLI smoke tests ──────────────────────────────────────────────────────

TEST_STORE="$(mktemp -d "${TMPDIR:-/tmp}/uteke_verify.XXXXXX")"
UTEKE="$BINARY --store $TEST_STORE"

log "--- CLI smoke tests (store: $TEST_STORE) ---"

# remember
OUT=$($UTEKE --json remember "Integration test memory" --tags smoke,test 2>/dev/null) || fail "remember failed"
ID=$(echo "$OUT" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])" 2>/dev/null) || fail "remember JSON parse failed"
log "remember → $ID"

if [ -z "$ID" ]; then
    fail "remember returned empty ID"
fi

# get
OUT=$($UTEKE --json get "$ID" 2>/dev/null) || fail "get failed"
CONTENT=$(echo "$OUT" | python3 -c "import sys,json; print(json.load(sys.stdin)['content'])" 2>/dev/null) || fail "get JSON parse failed"
log "get → $CONTENT"
[ "$CONTENT" = "Integration test memory" ] || fail "get content mismatch"

# recall
OUT=$($UTEKE --json recall "integration" --limit 3 2>/dev/null) || fail "recall failed"
COUNT=$(echo "$OUT" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null) || fail "recall JSON parse failed"
log "recall → $COUNT result(s)"
[ "$COUNT" -ge 1 ] || fail "recall returned 0 results"

# search
OUT=$($UTEKE --json search "test" 2>/dev/null) || fail "search failed"
COUNT=$(echo "$OUT" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null) || fail "search JSON parse failed"
log "search → $COUNT result(s)"
[ "$COUNT" -ge 1 ] || fail "search returned 0 results"

# list
OUT=$($UTEKE --json list --limit 10 2>/dev/null) || fail "list failed"
COUNT=$(echo "$OUT" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null) || fail "list JSON parse failed"
log "list → $COUNT item(s)"
[ "$COUNT" -ge 1 ] || fail "list returned 0 items"

# stats
OUT=$($UTEKE --json stats 2>/dev/null) || fail "stats failed"
TOTAL=$(echo "$OUT" | python3 -c "import sys,json; print(json.load(sys.stdin)['total_memories'])" 2>/dev/null) || fail "stats JSON parse failed"
log "stats → $TOTAL memories"
[ "$TOTAL" -ge 1 ] || fail "stats shows 0 memories"

# forget
OUT=$($UTEKE --json forget "$ID" 2>/dev/null) || fail "forget failed"
FORGOTTEN=$(echo "$OUT" | python3 -c "import sys,json; print(json.load(sys.stdin)['forgotten'])" 2>/dev/null) || fail "forget JSON parse failed"
log "forget → $FORGOTTEN"
[ "$FORGOTTEN" = "$ID" ] || fail "forget ID mismatch"

# verify deletion
OUT=$($UTEKE --json stats 2>/dev/null)
TOTAL=$(echo "$OUT" | python3 -c "import sys,json; print(json.load(sys.stdin)['total_memories'])" 2>/dev/null)
log "stats after forget → $TOTAL memories"
[ "$TOTAL" -eq 0 ] || fail "stats should show 0 memories after forget"

# ── 3. Python wrapper test ──────────────────────────────────────────────────

log "--- Python wrapper tests ---"

export UTEKE_BIN="$BINARY"

if ! python3 "$PROJECT_ROOT/examples/test_uteke_integration.py" 2>&1; then
    fail "Python integration tests failed"
fi

# ── 4. Result ───────────────────────────────────────────────────────────────

if [ "$FAILED" -ne 0 ]; then
    log "RESULT: FAILED"
    exit 1
fi

log "RESULT: ALL PASSED ✓"
exit 0
