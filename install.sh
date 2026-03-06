#!/bin/bash
# ForgeIndex installer — one-line install for macOS (Apple Silicon)
# Usage: curl -fsSL https://raw.githubusercontent.com/chrismicah/forgeindex/main/install.sh | sh

set -euo pipefail

REPO="chrismicah/forgeindex"
INSTALL_DIR="${FORGEINDEX_INSTALL_DIR:-/usr/local/bin}"
BINARY_NAME="forgeindex"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { printf "${CYAN}[forgeindex]${NC} %s\n" "$*"; }
ok()    { printf "${GREEN}[forgeindex]${NC} %s\n" "$*"; }
warn()  { printf "${YELLOW}[forgeindex]${NC} %s\n" "$*"; }
fail()  { printf "${RED}[forgeindex]${NC} %s\n" "$*"; exit 1; }

# Detect platform
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin) PLATFORM="apple-darwin" ;;
  Linux)  PLATFORM="unknown-linux-gnu" ;;
  *)      fail "Unsupported OS: $OS. ForgeIndex currently supports macOS and Linux." ;;
esac

case "$ARCH" in
  arm64|aarch64) TARGET="aarch64-${PLATFORM}" ;;
  x86_64)        TARGET="x86_64-${PLATFORM}" ;;
  *)             fail "Unsupported architecture: $ARCH" ;;
esac

info "Detected platform: ${TARGET}"

# Get latest release tag
info "Fetching latest release..."
if command -v curl &>/dev/null; then
  LATEST=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')
elif command -v wget &>/dev/null; then
  LATEST=$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')
else
  fail "Neither curl nor wget found. Install one and retry."
fi

if [ -z "$LATEST" ]; then
  # No releases yet — build from source
  warn "No binary releases found. Building from source..."
  
  if ! command -v cargo &>/dev/null; then
    info "Rust not found. Installing via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
  fi

  TMPDIR=$(mktemp -d)
  trap 'rm -rf "$TMPDIR"' EXIT
  
  info "Cloning repository..."
  git clone --depth 1 "https://github.com/${REPO}.git" "$TMPDIR/forgeindex"
  
  info "Building (release mode)..."
  cd "$TMPDIR/forgeindex"
  cargo build --release
  
  info "Installing to ${INSTALL_DIR}..."
  if [ -w "$INSTALL_DIR" ]; then
    cp "target/release/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
  else
    sudo cp "target/release/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
  fi
  
  chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
  ok "ForgeIndex installed successfully! (built from source)"
  echo ""
  info "Get started:"
  echo "  cd /path/to/your/project"
  echo "  forgeindex init"
  echo "  forgeindex serve"
  exit 0
fi

# Download binary release
ASSET_NAME="forgeindex-${LATEST}-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${LATEST}/${ASSET_NAME}"

info "Downloading ForgeIndex ${LATEST} for ${TARGET}..."
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

if command -v curl &>/dev/null; then
  curl -fsSL "$DOWNLOAD_URL" -o "$TMPDIR/${ASSET_NAME}"
else
  wget -q "$DOWNLOAD_URL" -O "$TMPDIR/${ASSET_NAME}"
fi

info "Extracting..."
tar -xzf "$TMPDIR/${ASSET_NAME}" -C "$TMPDIR"

# Verify checksum if available
CHECKSUM_URL="${DOWNLOAD_URL}.sha256"
if curl -fsSL "$CHECKSUM_URL" -o "$TMPDIR/checksum" 2>/dev/null; then
  info "Verifying checksum..."
  cd "$TMPDIR"
  if command -v sha256sum &>/dev/null; then
    sha256sum -c checksum
  elif command -v shasum &>/dev/null; then
    shasum -a 256 -c checksum
  fi
  cd - >/dev/null
fi

# Install
info "Installing to ${INSTALL_DIR}..."
if [ -w "$INSTALL_DIR" ]; then
  cp "$TMPDIR/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
else
  sudo cp "$TMPDIR/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
fi

chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

# Verify
if command -v forgeindex &>/dev/null; then
  VERSION=$(forgeindex --version 2>/dev/null || echo "$LATEST")
  ok "ForgeIndex ${VERSION} installed successfully!"
else
  ok "ForgeIndex installed to ${INSTALL_DIR}/${BINARY_NAME}"
  warn "Make sure ${INSTALL_DIR} is in your PATH"
fi

echo ""
info "Get started:"
echo "  cd /path/to/your/project"
echo "  forgeindex init"
echo "  forgeindex serve"
echo ""
info "Add to Claude Desktop / Conductor:"
echo '  { "mcpServers": { "forgeindex": { "command": "forgeindex", "args": ["serve", "--root", "."] } } }'
