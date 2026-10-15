#!/usr/bin/env bash
# Tests for scripts/release-signature-reminder.sh with a stub gh on PATH.
# Never touches the network.
#   bash scripts/tests/release-signature-reminder.test.sh
# bash 3.2 compatible.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$HERE/../release-signature-reminder.sh"
command -v jq >/dev/null 2>&1 || { echo "SKIP: jq not installed"; exit 0; }

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin" "$T/tags"

# Stub: api -> $STUB_RELEASE, issue list -> $STUB_ISSUES, everything else is
# a mutation and is logged one line per call.
cat > "$T/bin/gh" <<'STUB'
#!/usr/bin/env bash
case "$1" in
  api)
    case "$2" in
      */releases/latest) printf '%s' "$STUB_RELEASE" ;;
    esac ;;
  *) echo "OTHER $*" >> "$STUB_LOG" ;;
esac
STUB
chmod +x "$T/bin/gh"

NOW=1800000000   # fixed clock
iso() { date -u -r "$1" +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -d "@$1" +%Y-%m-%dT%H:%M:%SZ; }
H=3600

rel() { # tag assets-json [prerelease] [draft]
  printf '{"tag_name":"%s","prerelease":%s,"draft":%s,"published_at":"%s","assets":%s}' \
    "$1" "${3:-false}" "${4:-false}" "$(iso $((NOW - 30*H)))" "$2"
}
A_BUNDLE='[{"name":"checksums-sha256.txt"},{"name":"checksums-sha256.txt.bundle"}]'
A_SIGNED='[{"name":"checksums-sha256.txt"},{"name":"checksums-sha256.txt.bundle"},{"name":"checksums-sha256.txt.minisig"}]'
A_NOBUNDLE='[{"name":"checksums-sha256.txt"}]'
FAILS=0
# run <release-json> <issues-json> args...  ; sets RC, OUT, and $T/log
run() {
  local r="$1" i="$2"; shift 2
  : > "$T/log"; rm -f "$T/log.n"
  OUT="$(env PATH="$T/bin:$PATH" STUB_LOG="$T/log" STUB_TAGDIR="$T/tags" STUB_RELEASE="$r" STUB_ISSUES="$i" \
    GH_REPO=o/r RELEASE_SIGNER=ajianaz REMINDER_NOW_EPOCH=$NOW \
    STUB_ISSUES2="${STUB_ISSUES2-}" STUB_API_ERR="${STUB_API_ERR-}" \
    STUB_FAIL_ASSIGN="${STUB_FAIL_ASSIGN-}" bash "$SCRIPT" "$@" 2>&1)"
  RC=$?
}
check() { # name 0|1
  if [ "$2" = 0 ]; then echo "ok   - $1"; else echo "FAIL - $1 (rc=$RC)"; echo "$OUT" | sed 's/^/       /'; echo "  log:"; sed 's/^/       /' "$T/log"; FAILS=$((FAILS+1)); fi
}
rc_is() { [ "$RC" -eq "$1" ]; }
nlog() { grep -c -e "$1" "$T/log" || true; }
no_mutation() { [ ! -s "$T/log" ]; }
t() { local n="$1"; shift; if "$@"; then check "$n" 0; else check "$n" 1; fi; }

run "$(rel v1.2.3 "$A_SIGNED")" '[]'
t "signed -> exit 0, reported, nothing mutating" sh -c "[ $RC -eq 0 ] && [ ! -s '$T/log' ]"
echo "$OUT" | grep -q 'v1.2.3 is signed'; check "signed -> says so" $?
run "$(rel v1.2.3 "$A_BUNDLE")" '[]'
t "unsigned -> exit 0, reported, nothing mutating" sh -c "[ $RC -eq 0 ] && [ ! -s '$T/log' ]"
echo "$OUT" | grep -q 'v1.2.3 is unsigned'; check "unsigned -> says so" $?


run "$(rel v1.2.3 "$A_NOBUNDLE")" '[]'
t "no bundle -> exit 0 and nothing mutating" no_mutation
t "no bundle -> exit code 0" rc_is 0

for bad in 'v1.2.3; rm -rf' 'v1.2.3-rc.1' '1.2.3' 'v1.2' 'v1.2.3+b1'; do
  run "$(rel "$bad" "$A_BUNDLE")" '[]'
  t "bad tag '$bad' -> exit 0, nothing mutating" sh -c "[ $RC -eq 0 ] && [ ! -s '$T/log' ]"
done

run "$(rel v1.2.3 "$A_BUNDLE" true)" '[]'
t "prerelease latest -> exit 0, nothing mutating" sh -c "[ $RC -eq 0 ] && [ ! -s '$T/log' ]"
run "$(rel v1.2.3 "$A_BUNDLE" false true)" '[]'
t "draft latest -> exit 0, nothing mutating" sh -c "[ $RC -eq 0 ] && [ ! -s '$T/log' ]"

run "$(rel v1.2.3 "$A_BUNDLE")" '[]' --dry-run
t "--dry-run unsigned -> exit 0, no mutating call" sh -c "[ $RC -eq 0 ] && [ ! -s '$T/log' ]"


run "$(rel v1.2.3 "$A_BUNDLE")" '[]' --bogus
t "unknown flag -> exit 2" rc_is 2
run "$(rel v1.2.3 "$A_BUNDLE")" '[]' --dry-run --bogus
t "extra unknown flag after --dry-run -> exit 2, nothing mutating" sh -c "[ $RC -eq 2 ] && [ ! -s '$T/log' ]"
run "$(rel v1.2.3 "$A_BUNDLE")" '[]' --bogus --dry-run
t "unknown flag before --dry-run -> exit 2" rc_is 2
run "$(rel v1.2.3 "$A_BUNDLE")" '[]' --dry-run --dry-run
t "repeated --dry-run is harmless" no_mutation
# HOURS=08
run "$(rel v1.2.3 "$A_BUNDLE")" '[]'
OUT="$(env PATH="$T/bin:$PATH" STUB_LOG="$T/log" STUB_TAGDIR="$T/tags" STUB_RELEASE="$(rel v1.2.3 "$A_BUNDLE")" GH_REPO=o/r REMINDER_AFTER_HOURS=08 bash "$SCRIPT" 2>&1)"; RC=$?
t "REMINDER_AFTER_HOURS=08 accepted" rc_is 0
OUT="$(env PATH="$T/bin:$PATH" REMINDER_AFTER_HOURS=abc GH_REPO=o/r bash "$SCRIPT" 2>&1)"; RC=$?
t "non-numeric hours -> exit 2" rc_is 2

if [ "$FAILS" -ne 0 ]; then echo "$FAILS failure(s)"; exit 1; fi
echo "all passed"
