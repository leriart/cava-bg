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
#                         --version main, --version 0.2.6, --version <sha>
#   --repo <url>          Git repository (default: https://github.com/leriart/cava-bg.git)
#   --arch <triple>       Target arch for --binary (default: x86_64-unknown-linux-gnu)
#   --help                Show this help message and exit
#
# Examples:
#   curl -fsSL https://raw.githubusercontent.com/leriart/cava-bg/main/install.sh | bash
#   curl -fsSL .../install.sh | bash -s -- --system
#   curl -fsSL .../install.sh | bash -s -- --version 0.2.6
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
ARCHIVE_NAME="cava-bg-${TARGET_REF#v}-${ARCH}"

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
    # (e.g. 0.2.6 or v0.2.6). Branches like "main" have no release tarballs.
    if [[ ! "$TARGET_REF" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        log_info "--binary needs a release tag (e.g. --version 0.2.6); got '$TARGET_REF'."
        return 1
    fi

    TMP_DIR="$(mktemp -d -t cava-bg-XXXXXX)"
    ARCHIVE_PATH="$TMP_DIR/$ARCHIVE_NAME.tar.gz"
    EXTRACT_DIR="$TMP_DIR/extract"

    # Releases are tagged bare (0.2.6) but a `v`-prefixed ref is accepted too,
    # and older releases were published under both names, so try both.
    log_info "Downloading prebuilt release $TARGET_REF ($ARCH)..."
    local downloaded=1
    for candidate_ref in "${TARGET_REF#v}" "$TARGET_REF"; do
        local url="https://github.com/leriart/cava-bg/releases/download/${candidate_ref}/${ARCHIVE_NAME}.tar.gz"
        if curl --proto '=https' --tlsv1.2 -sSfL -o "$ARCHIVE_PATH" "$url"; then
            downloaded=0
            break
        fi
    done
    if [ "$downloaded" -ne 0 ]; then
        log_warn "Prebuilt binary not available for $TARGET_REF on $ARCH."
        rm -rf "$TMP_DIR"
        return 1
    fi

    if curl --proto '=https' --tlsv1.2 -sSfL -o "$ARCHIVE_PATH.sha256" "${url}.sha256" 2>/dev/null; then
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
    log_info "Prebuilt binary ready."
    return 0
}

# ─── Find a cargo that actually runs ─────────────────────────────────────
# `command -v cargo` is not enough: rustup installs a shim that exits with
# "no default is configured" when no toolchain is active. CARGO is an array
# so the fallback can be `rustup run <toolchain> cargo`, which pins the
# toolchain for this build only and leaves the user's default untouched.
CARGO=()

cargo_runs() {
    "$@" --version >/dev/null 2>&1
}

installed_toolchains() {
    rustup toolchain list 2>/dev/null | awk 'NF {print $1}'
}

resolve_cargo() {
    if command -v cargo &> /dev/null; then
        if cargo_runs cargo; then
            CARGO=(cargo)
            return 0
        fi
        echo "==> cargo is installed but does not run:"
        cargo --version 2>&1 | head -n 3 | sed 's/^/    /' || true
    else
        echo "==> cargo not found on PATH."
    fi

    if command -v rustup &> /dev/null; then
        local toolchain="stable" candidate
        # `rustup run` never changes the global default and never downloads
        # implicitly: check first, provision stable explicitly, then pin it
        # for this build only.
        if ! cargo_runs rustup run stable cargo; then
            echo "==> No usable stable Rust toolchain; installing stable..."
            if ! rustup toolchain install stable --profile minimal; then
                # Offline host with another usable toolchain: reuse it rather
                # than failing outright.
                toolchain=""
                while IFS= read -r candidate; do
                    [ -n "$candidate" ] || continue
                    if cargo_runs rustup run "$candidate" cargo; then
                        toolchain="$candidate"
                        break
                    fi
                done <<< "$(installed_toolchains)"
                if [ -z "$toolchain" ]; then
                    echo "Error: could not install the stable Rust toolchain,"
                    echo "and no other installed toolchain can run cargo."
                    echo "Install it manually, then re-run this installer:"
                    echo ""
                    echo "    rustup toolchain install stable --profile minimal"
                    exit 1
                fi
                echo "==> Falling back to installed toolchain '$toolchain'."
            fi
        fi

        CARGO=(rustup run "$toolchain" cargo)
        if ! cargo_runs "${CARGO[@]}"; then
            echo "Error: rustup toolchain '$toolchain' cannot run cargo."
            echo "Check 'rustup show' and reinstall it with:"
            echo ""
            echo "    rustup toolchain install '$toolchain' --profile minimal"
            exit 1
        fi
        echo "==> Building with rustup toolchain '$toolchain' (default unchanged)."
        return 0
    fi

    if ! command -v cargo &> /dev/null; then
        echo "==> Installing Rust with rustup..."
        if ! command -v curl &> /dev/null; then
            echo "Error: curl is required to install Rust automatically."
            exit 1
        fi
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
            | sh -s -- -y --default-toolchain stable --profile minimal
        if [ -f "$HOME/.cargo/env" ]; then
            # shellcheck disable=SC1091
            . "$HOME/.cargo/env"
        fi
        if cargo_runs cargo; then
            CARGO=(cargo)
        elif cargo_runs "$HOME/.cargo/bin/cargo"; then
            CARGO=("$HOME/.cargo/bin/cargo")
        else
            echo "Error: rustup finished but cargo still does not run."
            echo "Start a new shell (or run 'source \"\$HOME/.cargo/env\"') and"
            echo "re-run this installer."
            exit 1
        fi
        return 0
    fi

    echo "Error: cargo is installed but does not run, and rustup is not available."
    echo "Reinstall Rust from https://rust-lang.org/tools/install and re-run."
    exit 1
}

# ─── Default: build from source (git clone + cargo build --release --locked) ─
build_from_source() {
    local source_is_temp=0
    if [ -f "./Cargo.toml" ] && [ -f "./src/main.rs" ]; then
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
        # For tag-like refs (0.2.6) or commit SHAs, just use the rev directly
        # via git's --branch (which accepts tags too) or fall back to
        # shallow-clone + checkout.
        if [[ "$TARGET_REF" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+$ ]] \
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
        source_is_temp=1
    fi

    echo "==> Working in: $SCRIPT_DIR"
    echo "==> Building cava-bg from $TARGET_REF (this may take a few minutes)..."

    resolve_cargo

    # --locked enforces Cargo.lock so a tweak to Cargo.toml cannot silently
    # pull newer dependency versions at install time. On failure the resolved
    # cargo command is printed so the build can be reproduced exactly from
    # the same checkout.
    if ! "${CARGO[@]}" build --release --locked; then
        echo ""
        echo "Error: '${CARGO[*]} build --release --locked' failed."
        echo "The compiler output above is the authoritative reason; read its"
        echo "first error before changing anything."
        echo ""
        echo "Common causes:"
        echo "  · missing system build dependencies (Wayland, EGL, xkbcommon,"
        echo "    FFmpeg development headers)"
        echo "  · FFmpeg headers/ABI newer than the versions pinned in Cargo.lock"
        echo ""
        if [ "$source_is_temp" -eq 1 ]; then
            # The clone above lives in a mktemp directory removed on exit, so
            # point at a checkout the user can keep instead.
            echo "To reproduce from a fresh checkout:"
            echo ""
            echo "    git clone \"$REPO_URL\" cava-bg"
            echo "    cd cava-bg"
            echo "    git checkout \"$TARGET_REF\""
        else
            echo "To reproduce from the same tree:"
            echo ""
            echo "    cd \"$SCRIPT_DIR\""
        fi
        echo "    ${CARGO[*]} build --release --locked"
        echo ""
        echo "If, and only if, the error is an FFmpeg ABI/lockfile mismatch,"
        echo "refresh the FFmpeg crates and rebuild (keep --locked):"
        echo ""
        echo "    ${CARGO[*]} update -p ffmpeg-next"
        echo "    ${CARGO[*]} build --release --locked"
        echo ""
        echo "Build dependencies: docs/INSTALL.md"
        exit 1
    fi

    BIN_PATH="target/release/cava-bg"
    COMPLETIONS_DIR="target/release/completions"

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
            log_warn "Re-run without --binary to force a fresh build from source."
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
        if [ ! -t 0 ]; then
            # Non-interactive (CI, curl | bash from a script, a package manager
            # post-install hook): never invoke sudo unattended, just say so.
            echo "Not a terminal: skipping cava installation."
            echo "Install it manually, then re-run cava-bg: https://github.com/karlstav/cava"
        elif [ "$INSTALL_MODE" = "system" ] || [ "$INSTALL_MODE" = "user" ]; then
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
# The default config is generated by the binary itself, so there is no
# hand-maintained `config.toml` in the tree that can drift away from
# `Config::default()`.
if [ "$SKIP_CONFIG" -eq 0 ]; then
    USER_CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/cava-bg"
    USER_CONFIG="$USER_CONFIG_DIR/config.toml"
    mkdir -p "$USER_CONFIG_DIR"
    if [ -f "$USER_CONFIG" ]; then
        echo "==> Keeping existing config at $USER_CONFIG"
    elif "$INSTALL_BIN_DIR/cava-bg" __default-config >"$USER_CONFIG" 2>/dev/null; then
        echo "==> Wrote default config to $USER_CONFIG"
    else
        echo "==> Could not generate a default config; cava-bg will create it on first run."
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
