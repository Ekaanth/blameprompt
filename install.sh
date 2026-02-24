#!/bin/sh
# BlamePrompt installer — https://blameprompt.com
# Usage: curl -sSL https://blameprompt.com/install.sh | bash
set -e

VERSION="0.1.0"
REPO="ekaanth/blameprompt"
BINARY_NAME="blameprompt"
INSTALL_DIR="${BLAMEPROMPT_INSTALL_DIR:-$HOME/.local/bin}"

# Colors
BOLD="\033[1m"
GREEN="\033[1;32m"
CYAN="\033[1;36m"
DIM="\033[2m"
RED="\033[1;31m"
RESET="\033[0m"

info()  { printf "  ${GREEN}[info]${RESET} %s\n" "$1"; }
error() { printf "  ${RED}[error]${RESET} %s\n" "$1" >&2; exit 1; }

# Detect OS and architecture
detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)   OS="linux" ;;
        Darwin)  OS="darwin" ;;
        MINGW*|MSYS*|CYGWIN*) OS="windows" ;;
        *) error "Unsupported OS: $OS" ;;
    esac

    case "$ARCH" in
        x86_64|amd64)  ARCH="x86_64" ;;
        arm64|aarch64) ARCH="aarch64" ;;
        *) error "Unsupported architecture: $ARCH" ;;
    esac

    if [ "$OS" = "windows" ]; then
        TARGET="${ARCH}-pc-windows-msvc"
        EXT=".exe"
    elif [ "$OS" = "darwin" ]; then
        TARGET="${ARCH}-apple-darwin"
        EXT=""
    else
        TARGET="${ARCH}-unknown-linux-gnu"
        EXT=""
    fi
}

# Check for required tools
check_deps() {
    for cmd in curl tar; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            error "Required command not found: $cmd"
        fi
    done
}

main() {
    printf "\n"
    printf "  ${CYAN}BlamePrompt Installer${RESET} ${DIM}v%s${RESET}\n" "$VERSION"
    printf "  ${DIM}Track AI-generated code in Git${RESET}\n"
    printf "\n"

    check_deps
    detect_platform

    info "Detected platform: ${OS}/${ARCH}"

    # Build download URL
    TARBALL="${BINARY_NAME}-v${VERSION}-${TARGET}.tar.gz"
    URL="https://github.com/${REPO}/releases/download/v${VERSION}/${TARBALL}"

    # Download
    TMPDIR="$(mktemp -d)"
    trap 'rm -rf "$TMPDIR"' EXIT

    info "Downloading ${TARBALL}..."
    if ! curl -fsSL "$URL" -o "${TMPDIR}/${TARBALL}"; then
        error "Download failed. Check https://github.com/${REPO}/releases for available builds."
    fi

    # Extract
    info "Extracting..."
    tar -xzf "${TMPDIR}/${TARBALL}" -C "$TMPDIR"

    # Install
    mkdir -p "$INSTALL_DIR"
    mv "${TMPDIR}/${BINARY_NAME}${EXT}" "${INSTALL_DIR}/${BINARY_NAME}${EXT}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}${EXT}"

    info "Installed to ${INSTALL_DIR}/${BINARY_NAME}${EXT}"

    # Check if INSTALL_DIR is in PATH
    case ":$PATH:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            printf "\n"
            printf "  ${BOLD}Add to your PATH:${RESET}\n"
            printf "    ${CYAN}export PATH=\"%s:\$PATH\"${RESET}\n" "$INSTALL_DIR"
            printf "    ${DIM}Add this to your ~/.bashrc, ~/.zshrc, or shell profile.${RESET}\n"
            ;;
    esac

    # Run global init
    printf "\n"
    info "Running global setup..."
    "${INSTALL_DIR}/${BINARY_NAME}${EXT}" init --global

    printf "\n"
    printf "  ${GREEN}Done!${RESET} BlamePrompt v${VERSION} is ready.\n"
    printf "\n"
}

main "$@"
