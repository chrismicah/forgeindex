#!/bin/sh
# ForgeIndex installer — downloads the latest release binary or builds from source.
# Usage: curl -fsSL https://raw.githubusercontent.com/chrismicah/forgeindex/main/install.sh | sh

set -e

REPO="chrismicah/forgeindex"
BINARY="forgeindex"
INSTALL_DIR="${FORGEINDEX_INSTALL_DIR:-$HOME/.local/bin}"

info()  { printf '\033[1;34m[info]\033[0m  %s\n' "$1"; }
ok()    { printf '\033[1;32m[ok]\033[0m    %s\n' "$1"; }
warn()  { printf '\033[1;33m[warn]\033[0m  %s\n' "$1"; }
err()   { printf '\033[1;31m[error]\033[0m %s\n' "$1" >&2; exit 1; }

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)   PLATFORM="linux" ;;
        Darwin)  PLATFORM="darwin" ;;
        MINGW*|MSYS*|CYGWIN*) PLATFORM="windows" ;;
        *)       err "Unsupported OS: $OS" ;;
    esac

    case "$ARCH" in
        x86_64|amd64)   ARCH="x86_64" ;;
        arm64|aarch64)  ARCH="aarch64" ;;
        *)              err "Unsupported architecture: $ARCH" ;;
    esac
}

check_deps() {
    for cmd in curl tar; do
        command -v "$cmd" >/dev/null 2>&1 || err "Missing required tool: $cmd"
    done
}

try_download_release() {
    LATEST_URL="https://api.github.com/repos/${REPO}/releases/latest"
    RELEASE_JSON="$(curl -fsSL "$LATEST_URL" 2>/dev/null)" || return 1

    # Build expected asset name
    if [ "$PLATFORM" = "windows" ]; then
        ASSET_NAME="${BINARY}-${ARCH}-${PLATFORM}.zip"
    else
        ASSET_NAME="${BINARY}-${ARCH}-${PLATFORM}.tar.gz"
    fi

    DOWNLOAD_URL="$(echo "$RELEASE_JSON" | grep -o "\"browser_download_url\": *\"[^\"]*${ASSET_NAME}\"" | head -1 | cut -d'"' -f4)"

    [ -z "$DOWNLOAD_URL" ] && return 1

    info "Downloading $ASSET_NAME..."
    TMPDIR="$(mktemp -d)"
    trap 'rm -rf "$TMPDIR"' EXIT

    curl -fsSL "$DOWNLOAD_URL" -o "$TMPDIR/$ASSET_NAME" || return 1

    if [ "$PLATFORM" = "windows" ]; then
        unzip -q "$TMPDIR/$ASSET_NAME" -d "$TMPDIR" || return 1
    else
        tar xzf "$TMPDIR/$ASSET_NAME" -C "$TMPDIR" || return 1
    fi

    # Find the binary
    BIN_PATH="$(find "$TMPDIR" -name "$BINARY" -type f 2>/dev/null | head -1)"
    [ -z "$BIN_PATH" ] && BIN_PATH="$(find "$TMPDIR" -name "${BINARY}.exe" -type f 2>/dev/null | head -1)"
    [ -z "$BIN_PATH" ] && return 1

    mkdir -p "$INSTALL_DIR"
    cp "$BIN_PATH" "$INSTALL_DIR/$BINARY"
    chmod +x "$INSTALL_DIR/$BINARY"
    return 0
}

build_from_source() {
    info "No prebuilt binary found — building from source..."

    if ! command -v cargo >/dev/null 2>&1; then
        info "Rust not found. Installing via rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        . "$HOME/.cargo/env"
    fi

    if ! command -v cargo >/dev/null 2>&1; then
        err "cargo still not found after rustup install. Please install Rust manually: https://rustup.rs"
    fi

    # Check for C compiler (needed for tree-sitter grammars)
    if ! command -v cc >/dev/null 2>&1 && ! command -v gcc >/dev/null 2>&1 && ! command -v clang >/dev/null 2>&1; then
        warn "No C compiler found. Tree-sitter grammars require a C compiler."
        if [ "$PLATFORM" = "darwin" ]; then
            info "Install with: xcode-select --install"
        elif [ "$PLATFORM" = "linux" ]; then
            info "Install with: sudo apt install build-essential (Debian/Ubuntu) or equivalent"
        fi
        err "Please install a C compiler and try again."
    fi

    info "Building forgeindex (this takes ~1-2 minutes)..."
    cargo install --git "https://github.com/${REPO}.git" --root "$HOME/.local" 2>&1

    if [ ! -f "$INSTALL_DIR/$BINARY" ]; then
        # cargo install puts it in ~/.local/bin by default with --root ~/.local
        CARGO_BIN="$HOME/.cargo/bin/$BINARY"
        if [ -f "$CARGO_BIN" ]; then
            mkdir -p "$INSTALL_DIR"
            cp "$CARGO_BIN" "$INSTALL_DIR/$BINARY"
        fi
    fi
}

check_path() {
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            warn "$INSTALL_DIR is not in your PATH"
            SHELL_NAME="$(basename "$SHELL")"
            case "$SHELL_NAME" in
                zsh)  RC="$HOME/.zshrc" ;;
                bash) RC="$HOME/.bashrc" ;;
                fish) RC="$HOME/.config/fish/config.fish" ;;
                *)    RC="$HOME/.profile" ;;
            esac
            info "Add it with:"
            if [ "$SHELL_NAME" = "fish" ]; then
                echo "  fish_add_path $INSTALL_DIR"
            else
                echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> $RC && source $RC"
            fi
            ;;
    esac
}

main() {
    echo ""
    echo "  ⚡ ForgeIndex Installer"
    echo "  ───────────────────────"
    echo ""

    check_deps
    detect_platform
    info "Platform: ${PLATFORM}/${ARCH}"

    if try_download_release; then
        ok "Installed from release binary"
    else
        build_from_source
    fi

    # Verify
    if [ -f "$INSTALL_DIR/$BINARY" ]; then
        VERSION="$("$INSTALL_DIR/$BINARY" --version 2>/dev/null || echo "unknown")"
        ok "ForgeIndex installed: $INSTALL_DIR/$BINARY ($VERSION)"
        check_path
        echo ""
        echo "  Get started:"
        echo "    cd /path/to/your/project"
        echo "    forgeindex init"
        echo "    forgeindex status"
        echo ""
    else
        err "Installation failed — binary not found at $INSTALL_DIR/$BINARY"
    fi
}

main
