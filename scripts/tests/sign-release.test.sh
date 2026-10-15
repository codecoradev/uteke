#!/usr/bin/env bash
# Tests for scripts/sign-release.sh using stub gh/cosign on PATH and a
# throwaway minisign key. Never touches a real key or the network.
#   bash scripts/tests/sign-release.test.sh
# bash 3.2 compatible.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT_SRC="$HERE/../sign-release.sh"
command -v minisign >/dev/null 2>&1 || { echo "SKIP: minisign not installed"; exit 0; }

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin" "$T/nocosign" "$T/sc"
cp "$SCRIPT_SRC" "$T/sc/sign-release.sh"
minisign -G -W -p "$T/sc/uteke-release.pub" -s "$T/test.key" -f >/dev/null

cat > "$T/bin/gh" <<'STUB'
#!/usr/bin/env bash
case "$2" in
  view) for a in ${STUB_ASSETS-}; do echo "$a"; done ;;
  download)
    while [ $# -gt 0 ]; do
      case "$1" in --pattern) p="$2" ;; --dir) d="$2" ;; esac
      shift
    done
    echo "abc  file.tar.gz" > "$d/checksums-sha256.txt"
    [ "$p" = "checksums-sha256.txt" ] || echo bundle > "$d/$p"
    ;;
  upload) echo "UPLOAD $*" >> "$STUB_LOG" ;;
esac
STUB
cat > "$T/bin/cosign" <<'STUB'
#!/usr/bin/env bash
exit "${STUB_COSIGN_RC:-0}"
STUB
# minisign wrapper that records every signing call
REAL_MINISIGN="$(command -v minisign)"
cat > "$T/bin/minisign" <<STUB
#!/usr/bin/env bash
case "\$1" in -S) echo SIGN >> "\$STUB_LOG" ;; esac
exec "$REAL_MINISIGN" "\$@"
STUB
chmod +x "$T/bin/"*

# Minimal PATH without cosign: symlink the tools the script needs.
for tool in bash env cat grep sed mktemp rm dirname cd; do
  p="$(command -v "$tool" 2>/dev/null || true)"
  [ -n "$p" ] && [ -x "$p" ] && ln -sf "$p" "$T/nocosign/$tool"
done
cp "$T/bin/gh" "$T/bin/minisign" "$T/nocosign/"

FAILS=0
ASSETS_OK="checksums-sha256.txt checksums-sha256.txt.bundle"

# run <input> <path-prefix> <assets> [cosign_rc] -- args...  ; sets RC, OUT
run() {
  local input="$1" pathdir="$2" assets="$3" crc="$4"
  shift 4
  : > "$T/log"
  OUT="$(printf '%s\n' "$input" | env PATH="$pathdir:/usr/bin:/bin" STUB_LOG="$T/log" \
    STUB_ASSETS="$assets" STUB_COSIGN_RC="$crc" UTEKE_MINISIGN_KEY="$T/test.key" \
    bash "$T/sc/sign-release.sh" "$@" 2>&1)"
  RC=$?
}
expect() { # name expected_rc
  if [ "$RC" -eq "$2" ]; then echo "ok   - $1"; else echo "FAIL - $1 (rc=$RC, want $2)"; echo "$OUT" | sed 's/^/       /'; FAILS=$((FAILS+1)); fi
}
log_has() { grep -q "$1" "$T/log"; }
check() { # name condition-result
  if [ "$2" = 0 ]; then echo "ok   - $1"; else echo "FAIL - $1"; echo "$OUT" | sed 's/^/       /'; FAILS=$((FAILS+1)); fi
}

run y "$T/bin" "$ASSETS_OK" 1 --dry-run v1.2.3
expect "cosign fails -> exit 1" 1
log_has SIGN; check "cosign fails -> minisign never invoked" $((log_has_rc=$? == 0 ? 1 : 0))

run y "$T/bin" "checksums-sha256.txt" 0 --dry-run v1.2.3
expect "no bundle asset -> exit 1" 1

run y "$T/nocosign" "$ASSETS_OK" 0 --dry-run v1.2.3
expect "cosign missing -> exit 1" 1

run n "$T/bin" "$ASSETS_OK" 0 --dry-run v1.2.3
expect "answer n -> aborts" 1
log_has SIGN; check "answer n -> minisign never invoked" $((log_has_rc=$? == 0 ? 1 : 0))

run y "$T/bin" "$ASSETS_OK checksums-sha256.txt.minisig" 0 --dry-run v1.2.3
expect "existing .minisig without --force -> refuses" 1

run y "$T/bin" "$ASSETS_OK" 0 --dry-run 1.2.3
expect "bad tag (no v) -> exit 2" 2

run y "$T/bin" "$ASSETS_OK" 0 --no-cosign --dry-run v1.2.3
expect "unknown flag -> exit 2" 2

run y "$T/bin" "$ASSETS_OK" 0 --dry-run v1.2.3
expect "happy path --dry-run -> exit 0" 0
log_has SIGN; check "happy path signs" $?
echo "$OUT" | grep -q "Trusted comment: uteke v1.2.3"; check "happy path verifies trusted comment" $?
log_has UPLOAD; check "happy path does not upload" $((log_has_rc=$? == 0 ? 1 : 0))

if [ "$FAILS" -ne 0 ]; then echo "$FAILS failure(s)"; exit 1; fi
echo "all passed"
