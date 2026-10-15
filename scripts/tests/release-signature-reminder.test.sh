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
mkdir -p "$T/bin"

# Stub: api -> $STUB_RELEASE, issue list -> $STUB_ISSUES, everything else is
# a mutation and is logged one line per call.
cat > "$T/bin/gh" <<'STUB'
#!/usr/bin/env bash
case "$1" in
  api) printf '%s' "$STUB_RELEASE" ;;
  issue)
    case "$2" in
      list) printf '%s' "$STUB_ISSUES" ;;
      create)
        echo "CREATE $*" >> "$STUB_LOG"
        if [ -n "${STUB_FAIL_ASSIGN-}" ]; then
          case "$*" in *--assignee*) exit 1 ;; esac
        fi ;;
      *) echo "$(echo "$2" | tr a-z A-Z) $*" >> "$STUB_LOG" ;;
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
TITLE="Sign release v1.2.3 (minisign)"
issue() { # age-hours-of-creation [comment-age-hours]
  local c='[]'
  [ -n "${2-}" ] && c="[{\"createdAt\":\"$(iso $((NOW - $2*H)))\"}]"
  printf '[{"number":7,"title":"%s","createdAt":"%s","comments":%s},{"number":8,"title":"%s extra","createdAt":"%s","comments":[]}]' \
    "$TITLE" "$(iso $((NOW - $1*H)))" "$c" "$TITLE" "$(iso $((NOW - 99*H)))"
}

FAILS=0
# run <release-json> <issues-json> args...  ; sets RC, OUT, and $T/log
run() {
  local r="$1" i="$2"; shift 2
  : > "$T/log"
  OUT="$(env PATH="$T/bin:$PATH" STUB_LOG="$T/log" STUB_RELEASE="$r" STUB_ISSUES="$i" \
    GH_REPO=o/r RELEASE_SIGNER=ajianaz REMINDER_NOW_EPOCH=$NOW \
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

run "$(rel v1.2.3 "$A_SIGNED")" "$(issue 5)"
t "signed + open reminder -> exit 0" rc_is 0
t "signed + open reminder -> one comment" test "$(nlog '^COMMENT issue comment 7 ')" = 1
t "signed + open reminder -> closes #7 only" test "$(nlog '^CLOSE ')" = 1
grep -q '^CLOSE issue close 7 ' "$T/log"; check "signed -> closed the exact-title issue, not #8" $?

run "$(rel v1.2.3 "$A_SIGNED")" '[]'
t "signed + no reminder -> nothing mutating" no_mutation

run "$(rel v1.2.3 "$A_BUNDLE")" '[]'
t "unsigned + no reminder -> exit 0" rc_is 0
t "unsigned + no reminder -> one create" test "$(nlog '^CREATE ')" = 1
grep -qF -- "--title $TITLE " "$T/log"; check "create has the exact title" $?
grep -qF 'scripts/sign-release.sh v1.2.3' "$T/log"; check "create body has the sign command" $?
grep -qF -- '--assignee ajianaz' "$T/log"; check "create assigns the signer" $?

STUB_FAIL_ASSIGN=1 run "$(rel v1.2.3 "$A_BUNDLE")" '[]'
t "assign fails -> still exit 0" rc_is 0
t "assign fails -> retried without assignee" test "$(nlog '^CREATE ')" = 2
t "assign fails -> second create has no --assignee" test "$(nlog "--assignee")" = 1

run "$(rel v1.2.3 "$A_BUNDLE")" "$(issue 30 3)"
t "unsigned + reminder younger than interval (recent comment) -> no mutation" no_mutation
run "$(rel v1.2.3 "$A_BUNDLE")" "$(issue 11)"
t "unsigned + reminder 11h old -> no mutation" no_mutation

run "$(rel v1.2.3 "$A_BUNDLE")" "$(issue 13)"
t "unsigned + reminder 13h old -> exit 0" rc_is 0
t "unsigned + older reminder -> exactly one comment, nothing else" test "$(nlog '')" = 1
grep -q '^COMMENT issue comment 7 .*@ajianaz' "$T/log"; check "comment mentions the signer" $?
grep -q 'about 30 hours' "$T/log"; check "comment says how long it was unsigned" $?

run "$(rel v1.2.3 "$A_BUNDLE")" "$(issue 40 13)"
t "old issue but comment 13h ago -> comments again" test "$(nlog '^COMMENT ')" = 1
run "$(rel v1.2.3 "$A_BUNDLE")" "$(issue 40 2)"
t "old issue but comment 2h ago -> no spam" no_mutation

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
t "--dry-run unsigned + no reminder -> no mutating call" no_mutation
echo "$OUT" | grep -q 'dry-run'; check "--dry-run prints the intended action" $?
run "$(rel v1.2.3 "$A_BUNDLE")" "$(issue 13)" --dry-run
t "--dry-run stale reminder -> no mutating call" no_mutation
run "$(rel v1.2.3 "$A_SIGNED")" "$(issue 5)" --dry-run
t "--dry-run signed + reminder -> no mutating call" no_mutation
echo "$OUT" | grep -q 'dry-run\] gh issue close 7'; check "--dry-run prints the close" $?

run "$(rel v1.2.3 "$A_BUNDLE")" '[]' --bogus
t "unknown flag -> exit 2" rc_is 2

if [ "$FAILS" -ne 0 ]; then echo "$FAILS failure(s)"; exit 1; fi
echo "all passed"
