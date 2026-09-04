#!/bin/bash
#
# cava-bg installation script
#
# Usage:
#   Local:        ./install.sh [OPTIONS]
#   Remote:       curl -fsSL https://raw.githubusercontent.com/leriart/cava-bg/main/install.sh | bash -s -- [OPTIONS]
#
# Options:
#   --system              Install binary to /usr/local/bin (overwrites system copy)
#   --user                Install binary to ~/.local/bin (overwrites user copy)
#   --no-cava             Skip cava installation
#   --no-config           Skip creating the user configuration file
#   --no-completions      Skip installing shell completions
#   --source              Force building from source instead of downloading the
#                         prebuilt release binary
#   --version <tag>       cava-bg release to install (default: latest from GitHub,
#                         falls back to "main" if no releases exist). Examples:
#                         --version v0.2.6, --version main
#   --repo <url>          Git repository (default: https://github.com/leriart/cava-bg.git)
#   --arch <triple>       Target architecture for the prebuilt binary
#                         (default: x86_64-unknown-linux-gnu)
#   --help                Show this help message and exit
#
# Examples:
#   curl -fsSL https://raw.githubusercontent.com/leriart/cava-bg/main/install.sh | bash
#   curl -fsSL .../install.sh | bash -s -- --system
#   curl -fsSL .../install.sh | bash -s -- --version v0.2.6 --user
#   curl -fsSL .../install.sh | bash -s -- --source --system
#

set -e

REPO_URL="https://github.com/leriart/cava-bg.git"
INSTALL_MODE=""
SKIP_CAVA=0
SKIP_CONFIG=0
SKIP_COMPLETIONS=0
FORCE_SOURCE=0
REQUESTED_VERSION=""
ARCH="x86_64-unknown-linux-gnu"
GITHUB_API="https://api.github.com"
SCRIPT_DIR=""

print_banner() {
    cat <<'EOF'

 ███   ███  █   █  ███       ████   ███
█     █   █ █   █ █   █      █   █ █
█     █████ █   █ █████ ████ ████  █  ██
█     █   █  █ █  █   █      █   █ █   █
 ███  █   █   █   █   █      ████   ███

EOF
    echo "  cava-bg - Native CAVA Visualizer for Wayland"
    echo ""
}

show_help() {
    sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --system)
            INSTALL_MODE="system"
            shift
            ;;
        --user)
            INSTALL_MODE="user"
            shift
            ;;
        --no-cava)
            SKIP_CAVA=1
            shift
            ;;
        --no-config)
            SKIP_CONFIG=1
            shift
            ;;
        --no-completions)
            SKIP_COMPLETIONS=1
            shift
            ;;
        --source)
            FORCE_SOURCE=1
            shift
            ;;
        --version)
            REQUESTED_VERSION="$2"
            shift 2
            ;;
        --repo)
            REPO_URL="$2"
            shift 2
            ;;
        --arch)
            ARCH="$2"
            shift 2
            ;;
        --help|-h)
            show_help
            ;;
        *)
            echo "Unknown option: $1"
            echo "Run with --help for usage information."
            exit 1
            ;;
    esac
done

print_banner

# ─── Determine target version ──────────────────────────────────────────────
# Order of preference:
#   1. --version (explicit tag or "main")
#   2. Latest GitHub release tag (queries api.github.com)
#   3. Fall back to "main" if the API is unreachable or has no releases
TARGET_VERSION="${REQUESTED_VERSION}"
TARGET_TAG=""
SOURCE_BUILD=0

resolve_latest_release_tag() {
    local repo_path="${REPO_URL#https://github.com/}"
    repo_path="${repo_path%.git}"
    if ! command -v curl &> /dev/null; then
        return 1
    fi
    local api_url="${GITHUB_API}/repos/${repo_path}/releases/latest"
    local tag
    tag=$(curl --proto '=https' --tlsv1.2 -sSf -H "Accept: application/json" "$api_url" 2>/dev/null \
            | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)
    if [ -n "$tag" ]; then
        echo "$tag"
        return 0
    fi
    return 1
}

if [ -z "$TARGET_VERSION" ]; then
    echo "==> Resolving latest cava-bg release..."
    if TARGET_TAG=$(resolve_latest_release_tag); then
        TARGET_VERSION="$TARGET_TAG"
        echo "    → using release $TARGET_VERSION"
    else
        TARGET_VERSION="main"
        echo "    → GitHub API unreachable, falling back to branch: $TARGET_VERSION"
    fi
else
    if [ "$TARGET_VERSION" = "main" ] || [ "$TARGET_VERSION" = "master" ]; then
        TARGET_TAG=""
        echo "==> Using branch: $TARGET_VERSION"
    else
        TARGET_TAG="$TARGET_VERSION"
        echo "==> Using pinned release: $TARGET_TAG"
    fi
fi

ARCHIVE_NAME="cava-bg-${TARGET_VERSION/v/}-${ARCH}"
ARCHIVE_URL="https://github.com/leriart/cava-bg/releases/download/${TARGET_VERSION}/${ARCHIVE_NAME}.tar.gz"
CHECKSUM_URL="${ARCHIVE_URL}.sha256"

# ─── Decide install mode (system vs user) ─────────────────────────────────
if [ -z "$INSTALL_MODE" ]; then
    if [ -t 0 ]; then
        read -p "Install binary system-wide to /usr/local/bin? [y/N] " -n 1 -r
        echo
        if [[ $REPLY =~ ^[Yy]$ ]]; then
            INSTALL_MODE="system"
        else
            INSTALL_MODE="user"
        fi
    else
        # Non-interactive shell (e.g. curl | bash) without flags: default to user
        INSTALL_MODE="user"
    fi
fi

choose_install_dest() {
    if [ "$INSTALL_MODE" = "system" ]; then
        INSTALL_BIN_DIR="/usr/local/bin"
    else
        INSTALL_BIN_DIR="$HOME/.local/bin"
    fi
}

# ─── Path A: download prebuilt release binary ─────────────────────────────
download_prebuilt() {
    if [ "$FORCE_SOURCE" -eq 1 ]; then
        log_info "--source was passed, skipping binary download."
        return 1
    fi
    if [ -z "$TARGET_TAG" ]; then
        log_info "No release tag resolved, skipping binary download."
        return 1
    fi
    if ! command -v curl &> /dev/null; then
        log_info "curl not available, skipping binary download."
        return 1
    fi

    TMP_DIR="$(mktemp -d -t cava-bg-XXXXXX)"
    ARCHIVE_PATH="$TMP_DIR/$ARCHIVE_NAME.tar.gz"
    EXTRACT_DIR="$TMP_DIR/extract"

    log_info "Downloading prebuilt release $TARGET_TAG ($ARCH)..."
    if ! curl --proto '=https' --tlsv1.2 -sSfL -o "$ARCHIVE_PATH" "$ARCHIVE_URL"; then
        log_warn "Prebuilt binary not available for $TARGET_TAG on $ARCH."
        rm -rf "$TMP_DIR"
        return 1
    fi

    if curl --proto '=https' --tlsv1.2 -sSfL -o "$ARCHIVE_PATH.sha256" "$CHECKSUM_URL" 2>/dev/null; then
        (cd "$TMP_DIR" && sha256sum -c "$ARCHIVE_PATH.sha256" 2>/dev/null) \
            && log_info "Checksum OK" \
            || log_warn "Checksum verification failed (continuing anyway)"
    else
        log_warn "No .sha256 published for this release; skipping checksum."
    fi

    mkdir -p "$EXTRACT_DIR"
    tar -xzf "$ARCHIVE_PATH" -C "$EXTRACT_DIR"
    local extracted_bin="$EXTRACT_DIR/$ARCHIVE_NAME/cava-bg"
    if [ ! -f "$extracted_bin" ]; then
        log_warn "Archive did not contain expected binary at $ARCHIVE_NAME/cava-bg"
        rm -rf "$TMP_DIR"
        return 1
    fi
    BIN_PATH="$extracted_bin"
    COMPLETIONS_DIR="$EXTRACT_DIR/$ARCHIVE_NAME/completions"
    RESOURCE_DIR="$EXTRACT_DIR/$ARCHIVE_NAME"
    log_info "Prebuilt binary ready."
    return 0
}

# ─── Path B: build from source (clone + cargo build --release --locked) ──
build_from_source() {
    if [ ! -f "./Cargo.toml" ] || [ ! -f "./config.toml" ]; then
        echo "==> No local source found, cloning repository..."
        if ! command -v git &> /dev/null; then
            echo "Error: git is required to install cava-bg."
            exit 1
        fi

        TMP_DIR="$(mktemp -d -t cava-bg-XXXXXX)"
        trap 'rm -rf "$TMP_DIR"' EXIT

        # When TARGET_VERSION is "main"/"master", clone that branch.
        # When it's a tag, clone with --branch <tag>.
        if [ "$TARGET_VERSION" = "main" ] || [ "$TARGET_VERSION" = "master" ]; then
            git clone --depth 1 "$REPO_URL" "$TMP_DIR/cava-bg"
        else
            git clone --depth 1 --branch "$TARGET_VERSION" "$REPO_URL" "$TMP_DIR/cava-bg"
        fi
        cd "$TMP_DIR/cava-bg"
        SCRIPT_DIR="$TMP_DIR/cava-bg"
    else
        if [ -n "${CAVA_BG_SOURCE_DIR:-}" ]; then
            cd "$CAVA_BG_SOURCE_DIR"
            SCRIPT_DIR="$CAVA_BG_SOURCE_DIR"
        else
            SCRIPT_DIR="$(pwd)"
        fi
    fi

    echo "==> Working in: $SCRIPT_DIR"

    # Check for Rust toolchain
    if ! command -v cargo &> /dev/null; then
        echo "==> Rust toolchain not found, installing rustup..."
        if ! command -v curl &> /dev/null; then
            echo "Error: curl is required to install Rust automatically."
            exit 1
        fi
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
        # shellcheck disable=SC1091
        source "$HOME/.cargo/env"
    fi

    echo "==> Building cava-bg (this may take a few minutes)..."
    # --locked enforces Cargo.lock so a downstream tweak to Cargo.toml won't
    # silently pull a new ffmpeg-next or any other dependency at build time.
    cargo build --release --locked

    BIN_PATH="target/release/cava-bg"
    COMPLETIONS_DIR="target/release/completions"
    RESOURCE_DIR="$SCRIPT_DIR"

    if [ ! -f "$BIN_PATH" ]; then
        echo "Error: Build failed, binary not found at $BIN_PATH"
        exit 1
    fi
}

# ─── Tiny log helpers (work in both curl-piped and interactive shells) ────
log_info() { printf '   · %s\n' "$*"; }
log_warn() { printf '   ! %s\n' "$*" >&2; }

if download_prebuilt; then
    INSTALL_SOURCE="prebuilt $TARGET_TAG"
else
    build_from_source
    INSTALL_SOURCE="built from source ($TARGET_VERSION)"
fi

# Detect dynamic linkage mismatch with the system FFmpeg so a wrong binary
# (e.g. an old release still linked against FFmpeg 8 on a system that
# moved to FFmpeg 9) gets flagged before the install silently breaks.
check_ffmpeg_linkage() {
    local lddout
    lddout=$(ldd "$BIN_PATH" 2>/dev/null || true)
    if grep -q "libavutil.so.6" <<<"$lddout"; then
        if [ -z "$(ldconfig -p 2>/dev/null | grep -c 'libavutil.so.6')" ]; then
            log_warn "The binary you are installing expects libavutil.so.6"
            log_warn "but your system only has libavutil.so.61 (or newer)."
            log_warn "This is the very FFmpeg 9 ABI mismatch that 0.2.6 fixes."
            log_warn "Re-run with --source --version v0.2.6 to force a fresh build."
        fi
    fi
}
check_ffmpeg_linkage

# ─── Install the binary + completions ─────────────────────────────────────
choose_install_dest

install_user() {
    echo "==> Installing binary to $INSTALL_BIN_DIR/cava-bg..."
    mkdir -p "$INSTALL_BIN_DIR"
    install -m 0755 "$BIN_PATH" "$INSTALL_BIN_DIR/cava-bg"

    if [ "$SKIP_COMPLETIONS" -eq 0 ] && [ -d "$COMPLETIONS_DIR" ]; then
        echo "==> Installing shell completions (user)..."
        mkdir -p "$HOME/.local/share/bash-completion/completions"
        install -m 0644 "$COMPLETIONS_DIR/cava-bg.bash" \
            "$HOME/.local/share/bash-completion/completions/cava-bg" 2>/dev/null || true
        mkdir -p "$HOME/.local/share/zsh/site-functions"
        install -m 0644 "$COMPLETIONS_DIR/_cava-bg" \
            "$HOME/.local/share/zsh/site-functions/_cava-bg" 2>/dev/null || true
        mkdir -p "$HOME/.config/fish/completions"
        install -m 0644 "$COMPLETIONS_DIR/cava-bg.fish" \
            "$HOME/.config/fish/completions/cava-bg.fish" 2>/dev/null || true
    fi
}

install_system() {
    echo "==> Installing binary to $INSTALL_BIN_DIR/cava-bg (requires sudo)..."
    sudo install -m 0755 -D "$BIN_PATH" "$INSTALL_BIN_DIR/cava-bg"

    if [ "$SKIP_COMPLETIONS" -eq 0 ] && [ -d "$COMPLETIONS_DIR" ]; then
        echo "==> Installing shell completions (system)..."
        sudo install -m 0644 -D "$COMPLETIONS_DIR/cava-bg.bash" \
            "/usr/share/bash-completion/completions/cava-bg" 2>/dev/null || true
        sudo install -m 0644 -D "$COMPLETIONS_DIR/_cava-bg" \
            "/usr/share/zsh/site-functions/_cava-bg" 2>/dev/null || true
        sudo install -m 0644 -D "$COMPLETIONS_DIR/cava-bg.fish" \
            "/usr/share/fish/vendor_completions.d/cava-bg.fish" 2>/dev/null || true
    fi
}

if [ "$INSTALL_MODE" = "system" ]; then
    install_system
else
    install_user
    if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
        echo ""
        echo "Note: $HOME/.local/bin is not in your PATH."
        echo "Add this to your shell rc file (~/.bashrc, ~/.zshrc, etc.):"
        echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    fi
fi

# ─── Install cava if requested ────────────────────────────────────────────
check_cava() {
    command -v cava &> /dev/null
}

install_cava() {
    echo "==> Installing cava..."
    if command -v pacman &> /dev/null; then
        sudo pacman -S --noconfirm cava
    elif command -v apt &> /dev/null; then
        sudo apt update && sudo apt install -y cava
    elif command -v dnf &> /dev/null; then
        sudo dnf install -y cava
    elif command -v zypper &> /dev/null; then
        sudo zypper --non-interactive install cava
    elif command -v apk &> /dev/null; then
        sudo apk add cava
    elif command -v xbps-install &> /dev/null; then
        sudo xbps-install -y cava
    else
        echo "Warning: Could not detect a supported package manager."
        echo "Please install cava manually from: https://github.com/karlstav/cava"
        return 1
    fi
}

if [ "$SKIP_CAVA" -eq 0 ]; then
    if ! check_cava; then
        echo "==> cava not found."
        if [ -n "$INSTALL_MODE" ]; then
            install_cava || echo "Warning: cava installation failed; continuing anyway."
        else
            read -p "Install cava now? [Y/n] " -n 1 -r
            echo
            if [[ ! $REPLY =~ ^[Nn]$ ]]; then
                install_cava || echo "Warning: cava installation failed; continuing anyway."
            else
                echo "Skipping cava. The visualizer will not work without it."
            fi
        fi
    else
        echo "==> cava is already installed."
    fi
else
    echo "==> Skipping cava installation (--no-cava)."
fi

# ─── Create config directory ───────────────────────────────────────────────
if [ "$SKIP_CONFIG" -eq 0 ]; then
    if [ -d "$RESOURCE_DIR" ] && [ -f "$RESOURCE_DIR/config.toml" ]; then
        echo "==> Creating user configuration..."
        mkdir -p "$HOME/.config/cava-bg"
        if [ ! -f "$HOME/.config/cava-bg/config.toml" ]; then
            cp "$RESOURCE_DIR/config.toml" "$HOME/.config/cava-bg/config.toml"
            echo "    → ~/.config/cava-bg/config.toml (default config installed)"
        else
            echo "    → existing config kept at ~/.config/cava-bg/config.toml"
        fi
    fi
fi

INSTALLED_VERSION="$($INSTALL_BIN_DIR/cava-bg --version 2>/dev/null || echo 'unknown')"

echo ""
echo "==============================================="
echo " cava-bg has been installed successfully!"
echo "==============================================="
echo ""
echo "  Source:   $INSTALL_SOURCE"
echo "  Version:  $INSTALLED_VERSION"
echo "  Binary:   cava-bg ($([ "$INSTALL_MODE" = "system" ] && echo "/usr/local/bin" || echo "$HOME/.local/bin"))"
echo "  Config:   ~/.config/cava-bg/config.toml"
echo ""
echo "Usage:"
echo "  cava-bg on      # start daemon"
echo "  cava-bg off     # stop daemon"
echo "  cava-bg gui     # open configuration GUI"
echo ""
echo "To autostart on Hyprland, add to hyprland.conf:"
echo "  exec-once = cava-bg"
echo ""
echo "For AUR: yay -S cava-bg   |   For Nix: nix run github:leriart/cava-bg"
