#!/usr/bin/env bash
# Clone the AMB upstream repo and apply the uteke provider patch series.
# Usage: ./apply_patches.sh <target-dir>
set -euo pipefail

TARGET="${1:?usage: apply_patches.sh <target-dir>}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE="347f551b1014d8f8d5fab9eb38175f457a6ed195"  # upstream commit this series is based on

if [ -d "$TARGET/.git" ]; then
  echo "Reusing existing clone at $TARGET"
else
  git clone https://github.com/vectorize-io/agent-memory-benchmark.git "$TARGET"
fi

cd "$TARGET"
git checkout "$BASE" 2>/dev/null || {
  echo "WARNING: base commit $BASE not found (upstream rebased?)."
  echo "Patches may not apply cleanly — check with --3way."
  git checkout main
}

echo "Applying patch series..."
git apply --3way "$SCRIPT_DIR"/../patches/*.patch

echo
echo "Done. Verify:"
git diff --stat
echo
echo "Sanity check: src/memory_bench/memory/uteke.py must exist and"
echo "src/memory_bench/llm/openai.py must contain AMB_THINKING_DISABLED."
grep -q "AMB_THINKING_DISABLED" src/memory_bench/llm/openai.py && echo "OK: harness patches present"
test -f src/memory_bench/memory/uteke.py && echo "OK: uteke provider present"
