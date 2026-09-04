#!/bin/bash
#
# cava-bg installation script
#
# Always builds from source so what you run always matches the tree on
# `main` (or the branch / tag you pin with --version). The prebuilt
# GitHub release tarball is opt-in via --binary, but the default
# path is the deterministic "git clone + cargo build --locked" so a
# broken release attachment or stale cache cannot leak a binary that
# no longer matches the source code.
#
# Usage:
#   Local:        ./install.sh [OPTIONS]
#   Remote:       curl -fsSL https://raw.githubusercontent.com/leriart/cava-bg/main/install.sh | bash -s -- [OPTIONS]
#
# Options:
#   --system              Install binary to /usr/local/bin (requires sudo)
#   --user                Install binary to ~/.local/bin (default in non-tty)
#   --no-cava             Skip cava installation
#   --no-config           Skip creating the user configuration file
#   --no-completions      Skip installing shell completions
#   --binary              Try the GitHub release prebuilt tarball first,
#                         fall back to source build if the asset is missing.
#                         Off by default; the curl | bash flow always
#                         builds from source.
#   --version <ref>       Ref to build from (default: main). Examples:
#                         --version main, --version v0.2.6, --version <sha>
#   --repo <url>          Git repository (default: https://github.com/leriart/cava-bg.git)
#   --arch <triple>       Target arch for --binary (default: x86_64-unknown-linux-gnu)
#   --help                Show this help message and exit
#
# Examples:
#   curl -fsSL https://raw.githubusercontent.com/leriart/cava-bg/main/install.sh | bash
#   curl -fsSL .../install.sh | bash -s -- --system
#   curl -fsSL .../install.sh | bash -s -- --version v0.2.6
#   curl -fsSL .../install.sh | bash -s -- --binary       # opt into prebuilt
#

set -e

REPO_URL="https://github.com/leriart/cava-bg.git"
INSTALL_MODE=""
SKIP_CAVA=0
SKIP_CONFIG=0
SKIP_COMPLETIONS=0
USE_BINARY=0
REQUESTED_REF="main"
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
        --binary)
            USE_BINARY=1
            shift
            ;;
        --version)
            REQUESTED_REF="$2"
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

# ─── Always build from the requested ref via git + cargo ────────────────
# The default --version is "main", so the canonical curl|bash flow
# always rebuilds from whatever is currently at the tip of the main
# branch. A specific tag/sha can be pinned with --version.
TARGET_REF="$REQUESTED_REF"
ARCHIVE_NAME="cava-bg-${TARGET_REF/v/}-${ARCH}"
ARCHIVE_URL="https://github.com/leriart/cava-bg/releases/download/${TARGET_REF}/${ARCHIVE_NAME}.tar.gz"
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

# ─── Optional: download prebuilt release binary (opt-in via --binary) ─────
download_prebuilt() {
    if [ "$USE_BINARY" -eq 0 ]; then
        return 1
    fi
    if ! command -v curl &> /dev/null; then
        log_warn "curl not available, falling back to source build."
        return 1
    fi

    # Only attempt a download when --version looks like a release tag
    # (e.g. v0.2.6). Branches like "main" don't have release tarballs.
    if [[ ! "$TARGET_REF" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        log_info "--binary needs a release tag (e.g. --version v0.2.6); got '$TARGET_REF'."
        return 1
    fi

    TMP_DIR="$(mktemp -d -t cava-bg-XXXXXX)"
    ARCHIVE_PATH="$TMP_DIR/$ARCHIVE_NAME.tar.gz"
    EXTRACT_DIR="$TMP_DIR/extract"

    log_info "Downloading prebuilt release $TARGET_REF ($ARCH)..."
    if ! curl --proto '=https' --tlsv1.2 -sSfL -o "$ARCHIVE_PATH" "$ARCHIVE_URL"; then
        log_warn "Prebuilt binary not available for $TARGET_REF on $ARCH."
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

# ─── Default: build from source (git clone + cargo build --release --locked) ─
build_from_source() {
    if [ -f "./Cargo.toml" ] && [ -f "./config.toml" ]; then
        # Local checkout detected; use it as-is. Setting CAVA_BG_SOURCE_DIR
        # also forces the path even when the script is run from elsewhere.
        if [ -n "${CAVA_BG_SOURCE_DIR:-}" ]; then
            cd "$CAVA_BG_SOURCE_DIR"
            SCRIPT_DIR="$CAVA_BG_SOURCE_DIR"
        else
            SCRIPT_DIR="$(pwd)"
        fi
        echo "==> Using local source: $SCRIPT_DIR"
        echo "    (CARGO_TERM_COLOR=always cargo will pick up the existing target/)"
    else
        echo "==> No local source found, cloning repository..."
        if ! command -v git &> /dev/null; then
            echo "Error: git is required to install cava-bg."
            echo "Install git or clone the repository manually first."
            exit 1
        fi

        TMP_DIR="$(mktemp -d -t cava-bg-XXXXXX)"
        trap 'rm -rf "$TMP_DIR"' EXIT

        # For branch-like refs (main, master, dev/feature) use --branch.
        # For tag-like refs (v0.2.6) or commit SHAs, just use the rev directly
        # via git's --branch (which accepts tags too) or fall back to
        # shallow-clone + checkout.
        if [[ "$TARGET_REF" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] \
            || [[ "$TARGET_REF" =~ ^[0-9a-f]{7,40}$ ]] \
            || [[ "$TARGET_REF" == "main" ]] \
            || [[ "$TARGET_REF" == "master" ]]; then
            git clone --depth 1 --branch "$TARGET_REF" "$REPO_URL" "$TMP_DIR/cava-bg"
        else
            git clone --depth 1 "$REPO_URL" "$TMP_DIR/cava-bg"
            (cd "$TMP_DIR/cava-bg" && git fetch --depth 1 origin "$TARGET_REF" \
                && git checkout FETCH_HEAD) || {
                echo "Error: failed to fetch ref '$TARGET_REF'."
                exit 1
            }
        fi
        cd "$TMP_DIR/cava-bg"
        SCRIPT_DIR="$TMP_DIR/cava-bg"
    fi

    echo "==> Working in: $SCRIPT_DIR"
    echo "==> Building cava-bg from $TARGET_REF (this may take a few minutes)..."

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

    # --locked enforces Cargo.lock so a downstream tweak to Cargo.toml
    # cannot silently pull a newer ffmpeg-next or any other dependency
    # at install time. The first run on a fresh checkout may briefly
    # need `cargo update -p ffmpeg-next` if the system FFmpeg headers
    # are newer than what Cargo.lock was generated against; install.sh
    # surfaces a clear error in that case rather than silently fetching
    # a different version.
    if ! cargo build --release --locked; then
        echo ""
        echo "Error: 'cargo build --release --locked' failed."
        echo "This usually means the system FFmpeg headers are newer than"
        echo "the locked versions in Cargo.lock. The fix is to update the"
        echo "ffmpeg-next lockfile from the project root and re-run:"
        echo ""
        echo "    cargo update -p ffmpeg-next"
        echo "    $0"
        exit 1
    fi

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
    INSTALL_SOURCE="prebuilt $TARGET_REF"
else
    build_from_source
    INSTALL_SOURCE="built from source ($TARGET_REF)"
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
