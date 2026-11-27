#!/usr/bin/env bash
# Fetch the ONNX Runtime shared library that ships next to the uteke binaries
# (the `ort` crate loads it at run time, `load-dynamic`, so no build needs it).
#
# Usage: scripts/fetch-ort.sh <platform> <out-dir>
#   platform: linux-x64 | linux-arm64 | macos-arm64 | win-x64
#
# The source is the `onnxruntime` wheel on PyPI, pinned by URL and SHA-256.
# It replaces the github.com/microsoft/onnxruntime release archives, which
# answered 404 from 2026-11-17 (the v0.24.2 release run could not download
# them). A wheel is a zip that holds the same shared library; the file names
# written here are the ones `ort_init.rs` looks for:
#
#   linux-*      libonnxruntime.so, libonnxruntime_providers_shared.so
#   macos-arm64  libonnxruntime.dylib
#   win-x64      onnxruntime.dll, onnxruntime_providers_shared.dll
#
# To move to another ONNX Runtime version: pick the cp312 wheels of the new
# version on https://pypi.org/project/onnxruntime/#files, update ORT_VERSION
# and the four URL / SHA-256 pairs below, and keep `ORT_VERSION` in
# .github/workflows/release.yml equal to it.
set -euo pipefail

ORT_VERSION="1.24.4"

platform="${1:-}"
out="${2:-}"
if [ -z "$platform" ] || [ -z "$out" ]; then
  echo "usage: $0 <linux-x64|linux-arm64|macos-arm64|win-x64> <out-dir>" >&2
  exit 2
fi

case "$platform" in
  linux-x64)
    url="https://files.pythonhosted.org/packages/6d/ab/5b68110e0460d73fad814d5bd11c7b1ddcce5c37b10177eb264d6a36e331/onnxruntime-1.24.4-cp312-cp312-manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl"
    sha256="0d640eb9f3782689b55cfa715094474cd5662f2f137be6a6f847a594b6e9705c"
    # member in the wheel -> file name written to <out-dir>
    members="libonnxruntime.so.${ORT_VERSION}=libonnxruntime.so libonnxruntime_providers_shared.so=libonnxruntime_providers_shared.so"
    ;;
  linux-arm64)
    url="https://files.pythonhosted.org/packages/aa/60/c4d1c8043eb42f8a9aa9e931c8c293d289c48ff463267130eca97d13357f/onnxruntime-1.24.4-cp312-cp312-manylinux_2_27_aarch64.manylinux_2_28_aarch64.whl"
    sha256="1a5c5a544b22f90859c88617ecb30e161ee3349fcc73878854f43d77f00558b5"
    members="libonnxruntime.so.${ORT_VERSION}=libonnxruntime.so libonnxruntime_providers_shared.so=libonnxruntime_providers_shared.so"
    ;;
  macos-arm64)
    url="https://files.pythonhosted.org/packages/d7/38/31db1b232b4ba960065a90c1506ad7a56995cd8482033184e97fadca17cc/onnxruntime-1.24.4-cp312-cp312-macosx_14_0_arm64.whl"
    sha256="cad1c2b3f455c55678ab2a8caa51fb420c25e6e3cf10f4c23653cdabedc8de78"
    members="libonnxruntime.${ORT_VERSION}.dylib=libonnxruntime.dylib"
    ;;
  win-x64)
    url="https://files.pythonhosted.org/packages/8b/f4/6b89e297b93704345f0f3f8c62229bee323ef25682a3f9b4f89a39324950/onnxruntime-1.24.4-cp312-cp312-win_amd64.whl"
    sha256="535b29475ca42b593c45fbb2152fbf1cdf3f287315bf650e6a724a0a1d065cdb"
    members="onnxruntime.dll=onnxruntime.dll onnxruntime_providers_shared.dll=onnxruntime_providers_shared.dll"
    ;;
  *)
    echo "unknown platform: $platform" >&2
    exit 2
    ;;
esac

if command -v python3 >/dev/null 2>&1; then
  PY=python3
elif command -v python >/dev/null 2>&1; then
  PY=python
else
  echo "python3 is needed to unpack the wheel" >&2
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256_of() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha256_of() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
wheel="$work/onnxruntime.whl"

echo "Downloading onnxruntime ${ORT_VERSION} (${platform}) from PyPI"
curl -fsSL --retry 3 --retry-delay 5 -o "$wheel" "$url"

actual="$(sha256_of "$wheel")"
if [ "$actual" != "$sha256" ]; then
  echo "SHA-256 mismatch for the ${platform} wheel" >&2
  echo "  expected $sha256" >&2
  echo "  actual   $actual" >&2
  exit 1
fi

mkdir -p "$out"
"$PY" -I - "$wheel" "$out" $members <<'PY'
import os
import sys
import zipfile

wheel, out, *pairs = sys.argv[1:]
z = zipfile.ZipFile(wheel)
for i, pair in enumerate(pairs):
    member, name = pair.split("=", 1)
    data = z.read("onnxruntime/capi/" + member)
    # The first member is the runtime itself (megabytes); the providers
    # library next to it is a small stub, so only the first is size-checked.
    if i == 0 and len(data) < 1_000_000:
        sys.exit("%s is only %d bytes, not a runtime library" % (member, len(data)))
    with open(os.path.join(out, name), "wb") as f:
        f.write(data)
PY

chmod 755 "$out"/* 2>/dev/null || true
echo "ONNX Runtime ${ORT_VERSION} libraries in ${out}:"
ls -lh "$out"
