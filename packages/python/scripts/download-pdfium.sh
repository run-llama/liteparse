#!/usr/bin/env bash
#
# Download the prebuilt pdfium binary for a given target triple and stage it
# at `packages/python/liteparse/<dylib>` so maturin's `[tool.maturin] include`
# rule packs it into the wheel (and records it in RECORD) at build time.
#
# Usage: ./scripts/download-pdfium.sh <target-triple>
#
# The release tag is parsed from `crates/pdfium-sys/build.rs` so we only have
# one source of truth for the pdfium version.

set -euo pipefail

TARGET="${1:?usage: download-pdfium.sh <target-triple>}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PKG_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$PKG_DIR/../.." && pwd)"

BUILD_RS="$REPO_ROOT/crates/pdfium-sys/build.rs"
TAG=$(grep -E 'const PDFIUM_RELEASE_TAG' "$BUILD_RS" | sed -E 's/.*"([^"]+)".*/\1/')
[ -n "$TAG" ] || { echo "Could not parse PDFIUM_RELEASE_TAG from $BUILD_RS" >&2; exit 1; }

case "$TARGET" in
  aarch64-apple-darwin)        ASSET="pdfium-mac-arm64";      DYLIB="libpdfium.dylib" ;;
  x86_64-apple-darwin)         ASSET="pdfium-mac-x64";        DYLIB="libpdfium.dylib" ;;
  x86_64-unknown-linux-gnu)    ASSET="pdfium-linux-x64";      DYLIB="libpdfium.so"    ;;
  aarch64-unknown-linux-gnu)   ASSET="pdfium-linux-arm64";    DYLIB="libpdfium.so"    ;;
  x86_64-unknown-linux-musl)   ASSET="pdfium-linux-musl-x64"; DYLIB="libpdfium.so"    ;;
  aarch64-unknown-linux-musl)  ASSET="pdfium-linux-arm64";    DYLIB="libpdfium.so"    ;;
  x86_64-pc-windows-msvc)      ASSET="pdfium-win-x64";        DYLIB="pdfium.dll"      ;;
  aarch64-pc-windows-msvc)     ASSET="pdfium-win-arm64";      DYLIB="pdfium.dll"      ;;
  *) echo "Unsupported target: $TARGET" >&2; exit 1 ;;
esac

URL_TAG="${TAG//\//%2F}"
URL="https://github.com/run-llama/pdfium-binaries/releases/download/${URL_TAG}/${ASSET}.tgz"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# Same pinned sha256 as crates/pdfium-sys/build.rs (one ("<asset>", "<sha256>"), per line).
SHA_TAG=$(grep -E 'const PDFIUM_SHA256_TAG' "$BUILD_RS" | sed -E 's/.*"([^"]+)".*/\1/')
[ "$SHA_TAG" = "$TAG" ] || { echo "PDFIUM_SHA256_TAG ($SHA_TAG) != PDFIUM_RELEASE_TAG ($TAG) in $BUILD_RS" >&2; exit 1; }
EXPECTED_SHA256=$(grep -E "^ *\(\"${ASSET}\", \"[0-9a-f]{64}\"\)," "$BUILD_RS" | sed -E 's/.*"([0-9a-f]{64})".*/\1/')
[ -n "$EXPECTED_SHA256" ] || { echo "No sha256 for $ASSET in PDFIUM_SHA256 in $BUILD_RS" >&2; exit 1; }

echo "Downloading $URL"
# Retry transient failures (DNS, resets, 5xx) with backoff: 4 attempts in all.
curl -fsSL --retry 3 --retry-all-errors --retry-delay 2 "$URL" -o "$TMP/${ASSET}.tgz"

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL_SHA256=$(sha256sum "$TMP/${ASSET}.tgz" | cut -d' ' -f1)
else
  ACTUAL_SHA256=$(shasum -a 256 "$TMP/${ASSET}.tgz" | cut -d' ' -f1)
fi
if [ "$ACTUAL_SHA256" != "$EXPECTED_SHA256" ]; then
  echo "sha256 mismatch for ${ASSET}.tgz: expected $EXPECTED_SHA256, got $ACTUAL_SHA256" >&2
  exit 1
fi
tar -xzf "$TMP/${ASSET}.tgz" -C "$TMP"

# pdfium-binaries layout: lib/<DYLIB> on unix, bin/pdfium.dll on windows
SRC=""
for candidate in "$TMP/lib/$DYLIB" "$TMP/bin/$DYLIB"; do
  if [ -f "$candidate" ]; then
    SRC="$candidate"
    break
  fi
done
[ -n "$SRC" ] || { echo "Could not find $DYLIB in archive" >&2; ls -R "$TMP" >&2; exit 1; }

# Mirror the install-name fix in crates/pdfium-sys/build.rs so the dylib resolves
# via @rpath at runtime. pdfium-binaries ships macOS dylibs with install name
# `./libpdfium.dylib` which won't be found via our rpath.
if [ "$DYLIB" = "libpdfium.dylib" ] && command -v install_name_tool >/dev/null 2>&1; then
  install_name_tool -id "@rpath/libpdfium.dylib" "$SRC"
fi

DEST_DIR="$PKG_DIR/liteparse"
mkdir -p "$DEST_DIR"
cp "$SRC" "$DEST_DIR/$DYLIB"
echo "Staged $DEST_DIR/$DYLIB"
