#!/usr/bin/env bash
# Hew installer — fetches the latest release binary for this platform.
#
# Once cargo-dist is initialized for the first release, this script is
# REPLACED by the generated dist installer. Until then, it points users
# at the GitHub releases page so they can install manually.

set -euo pipefail

REPO="droidnoob/hew"
INSTALL_DIR="${HEW_INSTALL_DIR:-$HOME/.local/bin}"
RELEASES_URL="https://github.com/${REPO}/releases"

die() { echo "hew install: $*" >&2; exit 1; }

# Detect platform target triple.
detect_target() {
  local os arch
  os=$(uname -s)
  arch=$(uname -m)
  case "${os}-${arch}" in
    Darwin-arm64)        echo "aarch64-apple-darwin" ;;
    Darwin-x86_64)       echo "x86_64-apple-darwin" ;;
    Linux-x86_64)        echo "x86_64-unknown-linux-gnu" ;;
    Linux-aarch64|Linux-arm64) echo "aarch64-unknown-linux-gnu" ;;
    *) die "unsupported platform: ${os}-${arch}. See ${RELEASES_URL}" ;;
  esac
}

TARGET=$(detect_target)

# Resolve the latest tag.
if command -v curl >/dev/null 2>&1; then
  LATEST=$(curl -sSL "https://api.github.com/repos/${REPO}/releases/latest" |
           grep -E '"tag_name"' | head -n1 | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/' || true)
elif command -v wget >/dev/null 2>&1; then
  LATEST=$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" |
           grep -E '"tag_name"' | head -n1 | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/' || true)
else
  die "neither curl nor wget found"
fi

[[ -z "${LATEST:-}" ]] && die "could not resolve latest release tag. Visit ${RELEASES_URL}"

ASSET="hew-${LATEST}-${TARGET}.tar.gz"
URL="${RELEASES_URL}/download/${LATEST}/${ASSET}"

mkdir -p "${INSTALL_DIR}"
TMP=$(mktemp -d)
trap 'rm -rf "${TMP}"' EXIT

echo "==> downloading ${ASSET}"
if command -v curl >/dev/null 2>&1; then
  curl -fSL "${URL}" -o "${TMP}/hew.tar.gz" || die "download failed. Asset may not exist yet for ${TARGET}. See ${RELEASES_URL}"
else
  wget -qO "${TMP}/hew.tar.gz" "${URL}" || die "download failed."
fi

echo "==> extracting"
tar -xzf "${TMP}/hew.tar.gz" -C "${TMP}"

# Tarball contains a `hew` binary somewhere; find it.
BIN=$(find "${TMP}" -name hew -type f -perm -u+x | head -n1)
[[ -z "${BIN}" ]] && die "could not locate 'hew' binary inside the archive."

install -m 0755 "${BIN}" "${INSTALL_DIR}/hew"

echo "==> installed ${INSTALL_DIR}/hew"
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *) echo "    note: ${INSTALL_DIR} is not on your PATH. Add to your shell rc:"
     echo "          export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
esac

"${INSTALL_DIR}/hew" --version
