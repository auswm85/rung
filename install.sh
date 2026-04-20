#!/bin/sh
# Rung installer script
# Usage: curl -sSf https://rungstack.com/install.sh | sh
#    or: curl -sSf https://rungstack.com/install.sh | sh -s -- --version v0.5.0
#
# Environment variables:
#   INSTALL_DIR - Custom installation directory (default: /usr/local/bin or ~/.local/bin)

set -e

REPO="auswm85/rung"
BINARY_NAME="rung"

# Colors (only if terminal supports it)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    BOLD='\033[1m'
    NC='\033[0m' # No Color
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    BOLD=''
    NC=''
fi

info() {
    printf "${BLUE}info:${NC} %s\n" "$1"
}

warn() {
    printf "${YELLOW}warning:${NC} %s\n" "$1"
}

error() {
    printf "${RED}error:${NC} %s\n" "$1" >&2
}

success() {
    printf "${GREEN}✓${NC} %s\n" "$1"
}

usage() {
    cat <<EOF
Rung installer

Usage:
    curl -sSf https://rungstack.com/install.sh | sh
    curl -sSf https://rungstack.com/install.sh | sh -s -- [OPTIONS]

Options:
    --version <tag>    Install a specific version (e.g., v0.8.0)
    --help             Show this help message

Environment Variables:
    INSTALL_DIR        Custom installation directory
                       (default: /usr/local/bin if writable, else ~/.local/bin)

Examples:
    # Install latest version
    curl -sSf https://rungstack.com/install.sh | sh

    # Install specific version
    curl -sSf https://rungstack.com/install.sh | sh -s -- --version v0.7.0

    # Install to custom directory
    INSTALL_DIR=~/bin curl -sSf https://rungstack.com/install.sh | sh
EOF
}

detect_platform() {
    OS=$(uname -s)
    ARCH=$(uname -m)

    case "$OS" in
        Darwin)
            OS_TYPE="apple-darwin"
            ;;
        Linux)
            OS_TYPE="unknown-linux-gnu"
            ;;
        MINGW* | MSYS* | CYGWIN*)
            error "Windows detected. Please download manually from:"
            error "https://github.com/${REPO}/releases"
            error "Or use: scoop install rung"
            exit 1
            ;;
        *)
            error "Unsupported operating system: $OS"
            exit 1
            ;;
    esac

    case "$ARCH" in
        x86_64 | amd64)
            ARCH_TYPE="x86_64"
            ;;
        arm64 | aarch64)
            ARCH_TYPE="aarch64"
            ;;
        *)
            error "Unsupported architecture: $ARCH"
            exit 1
            ;;
    esac

    # Linux currently only supports x86_64
    if [ "$OS" = "Linux" ] && [ "$ARCH_TYPE" = "aarch64" ]; then
        error "Linux ARM64 binaries are not currently available."
        error "Please build from source: cargo install rung-cli"
        exit 1
    fi

    TARGET="${ARCH_TYPE}-${OS_TYPE}"
}

get_latest_version() {
    LATEST_URL="https://api.github.com/repos/${REPO}/releases/latest"

    if command -v curl >/dev/null 2>&1; then
        VERSION=$(curl -sSf "$LATEST_URL" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
    elif command -v wget >/dev/null 2>&1; then
        VERSION=$(wget -qO- "$LATEST_URL" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
    else
        error "Neither curl nor wget found. Please install one of them."
        exit 1
    fi

    if [ -z "$VERSION" ]; then
        error "Failed to fetch latest version from GitHub"
        exit 1
    fi
}

determine_install_dir() {
    if [ -n "$INSTALL_DIR" ]; then
        # User-specified directory
        INSTALL_PATH="$INSTALL_DIR"
    elif [ -w "/usr/local/bin" ]; then
        # System directory if writable
        INSTALL_PATH="/usr/local/bin"
    else
        # Fallback to user local bin
        INSTALL_PATH="$HOME/.local/bin"
    fi

    # Create directory if it doesn't exist
    if [ ! -d "$INSTALL_PATH" ]; then
        info "Creating directory: $INSTALL_PATH"
        mkdir -p "$INSTALL_PATH"
    fi
}

download_and_install() {
    VERSION_NUM=$(echo "$VERSION" | sed 's/^v//')
    ARCHIVE_NAME="${BINARY_NAME}-${VERSION_NUM}-${TARGET}.tar.gz"
    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE_NAME}"

    info "Detected: ${TARGET}"
    info "Downloading rung ${VERSION}..."

    # Create temp directory
    TMP_DIR=$(mktemp -d)
    trap 'rm -rf "$TMP_DIR"' EXIT

    # Download
    if command -v curl >/dev/null 2>&1; then
        HTTP_CODE=$(curl -sSL -w "%{http_code}" -o "$TMP_DIR/$ARCHIVE_NAME" "$DOWNLOAD_URL")
        if [ "$HTTP_CODE" != "200" ]; then
            error "Download failed (HTTP $HTTP_CODE)"
            error "URL: $DOWNLOAD_URL"
            if [ "$HTTP_CODE" = "404" ]; then
                error "Version ${VERSION} may not exist or may not have a binary for ${TARGET}"
            fi
            exit 1
        fi
    elif command -v wget >/dev/null 2>&1; then
        if ! wget -q -O "$TMP_DIR/$ARCHIVE_NAME" "$DOWNLOAD_URL"; then
            error "Download failed"
            error "URL: $DOWNLOAD_URL"
            exit 1
        fi
    fi

    # Extract
    info "Extracting..."
    tar -xzf "$TMP_DIR/$ARCHIVE_NAME" -C "$TMP_DIR"

    # Find the binary (might be in a subdirectory)
    BINARY_PATH=$(find "$TMP_DIR" -name "$BINARY_NAME" -type f | head -1)
    if [ -z "$BINARY_PATH" ]; then
        error "Binary not found in archive"
        exit 1
    fi

    # Install
    info "Installing to ${INSTALL_PATH}/${BINARY_NAME}..."

    # Check if we need sudo
    if [ -w "$INSTALL_PATH" ]; then
        mv "$BINARY_PATH" "$INSTALL_PATH/$BINARY_NAME"
        chmod +x "$INSTALL_PATH/$BINARY_NAME"
    else
        warn "Root permissions required to install to $INSTALL_PATH"
        sudo mv "$BINARY_PATH" "$INSTALL_PATH/$BINARY_NAME"
        sudo chmod +x "$INSTALL_PATH/$BINARY_NAME"
    fi

    success "rung ${VERSION} installed successfully!"
    echo ""

    # Check if install path is in PATH
    case ":$PATH:" in
        *":$INSTALL_PATH:"*)
            printf "Run '${BOLD}rung --help${NC}' to get started.\n"
            ;;
        *)
            warn "$INSTALL_PATH is not in your PATH"
            echo ""
            echo "Add it to your shell profile:"
            echo "  export PATH=\"\$PATH:$INSTALL_PATH\""
            ;;
    esac
}

main() {
    VERSION=""

    # Parse arguments
    while [ $# -gt 0 ]; do
        case "$1" in
            --version)
                if [ -z "$2" ]; then
                    error "--version requires a version argument (e.g., v0.8.0)"
                    exit 1
                fi
                VERSION="$2"
                shift 2
                ;;
            --help | -h)
                usage
                exit 0
                ;;
            *)
                error "Unknown option: $1"
                usage
                exit 1
                ;;
        esac
    done

    detect_platform

    if [ -z "$VERSION" ]; then
        get_latest_version
    fi

    determine_install_dir
    download_and_install
}

main "$@"
