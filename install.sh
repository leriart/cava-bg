#!/bin/bash
#
# cava-bg installation script
#
# Usage:
#   Local:        ./install.sh [OPTIONS]
#   Remote:       curl -fsSL https://raw.githubusercontent.com/leriart/cava-bg/main/install.sh | bash -s -- [OPTIONS]
#
# Options:
#   --system              Install binary to /usr/local/bin (non-interactive)
#   --user                Install binary to ~/.local/bin (non-interactive)
#   --no-cava             Skip cava installation
#   --no-config           Skip creating the user configuration file
#   --no-completions      Skip installing shell completions
#   --repo <url>          Git repository to clone (default: https://github.com/leriart/cava-bg.git)
#   --branch <name>       Git branch/tag to checkout (default: main)
#   --help                Show this help message and exit

set -e

REPO_URL="https://github.com/leriart/cava-bg.git"
BRANCH="main"
INSTALL_MODE=""
SKIP_CAVA=0
SKIP_CONFIG=0
SKIP_COMPLETIONS=0
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
    sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
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
        --repo)
            REPO_URL="$2"
            shift 2
            ;;
        --branch)
            BRANCH="$2"
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

# Detect if running via pipe (curl | bash) - no local repo available
NEEDS_CLONE=0
if [ -z "${CAVA_BG_SOURCE_DIR:-}" ] && [ ! -f "./Cargo.toml" ] && [ ! -f "./config.toml" ]; then
    NEEDS_CLONE=1
fi

if [ "$NEEDS_CLONE" -eq 1 ]; then
    echo "==> No local source found, cloning repository..."
    if ! command -v git &> /dev/null; then
        echo "Error: git is required to install cava-bg remotely."
        echo "Install git or clone the repository manually first."
        exit 1
    fi

    TMP_DIR="$(mktemp -d -t cava-bg-XXXXXX)"
    trap 'rm -rf "$TMP_DIR"' EXIT

    git clone --depth 1 --branch "$BRANCH" "$REPO_URL" "$TMP_DIR/cava-bg"
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

# Check for cava
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

# Build in release mode
echo "==> Building cava-bg (this may take a few minutes)..."
cargo build --release

BIN_PATH="target/release/cava-bg"
COMPLETIONS_DIR="target/release/completions"

if [ ! -f "$BIN_PATH" ]; then
    echo "Error: Build failed, binary not found at $BIN_PATH"
    exit 1
fi

# Determine install mode
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

install_user() {
    echo "==> Installing binary to ~/.local/bin/cava-bg..."
    mkdir -p "$HOME/.local/bin"
    cp "$BIN_PATH" "$HOME/.local/bin/cava-bg"
    chmod +x "$HOME/.local/bin/cava-bg"

    if [ "$SKIP_COMPLETIONS" -eq 0 ] && [ -d "$COMPLETIONS_DIR" ]; then
        echo "==> Installing shell completions (user)..."
        mkdir -p "$HOME/.local/share/bash-completion/completions"
        cp "$COMPLETIONS_DIR/cava-bg.bash" "$HOME/.local/share/bash-completion/completions/cava-bg" 2>/dev/null || true
        mkdir -p "$HOME/.local/share/zsh/site-functions"
        cp "$COMPLETIONS_DIR/_cava-bg" "$HOME/.local/share/zsh/site-functions/_cava-bg" 2>/dev/null || true
        mkdir -p "$HOME/.config/fish/completions"
        cp "$COMPLETIONS_DIR/cava-bg.fish" "$HOME/.config/fish/completions/cava-bg.fish" 2>/dev/null || true
    fi
}

install_system() {
    echo "==> Installing binary to /usr/local/bin/cava-bg (requires sudo)..."
    sudo cp "$BIN_PATH" /usr/local/bin/cava-bg
    sudo chmod +x /usr/local/bin/cava-bg

    if [ "$SKIP_COMPLETIONS" -eq 0 ] && [ -d "$COMPLETIONS_DIR" ]; then
        echo "==> Installing shell completions (system)..."
        sudo mkdir -p /usr/share/bash-completion/completions
        sudo cp "$COMPLETIONS_DIR/cava-bg.bash" /usr/share/bash-completion/completions/cava-bg 2>/dev/null || true
        sudo mkdir -p /usr/share/zsh/site-functions
        sudo cp "$COMPLETIONS_DIR/_cava-bg" /usr/share/zsh/site-functions/_cava-bg 2>/dev/null || true
        sudo mkdir -p /usr/share/fish/vendor_completions.d
        sudo cp "$COMPLETIONS_DIR/cava-bg.fish" /usr/share/fish/vendor_completions.d/cava-bg.fish 2>/dev/null || true
    fi
}

if [ "$INSTALL_MODE" = "system" ]; then
    install_system
else
    install_user
    if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
        echo ""
        echo "Note: ~/.local/bin is not in your PATH."
        echo "Add this to your shell rc file (~/.bashrc, ~/.zshrc, etc.):"
        echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    fi
fi

# Create config directory
if [ "$SKIP_CONFIG" -eq 0 ]; then
    echo "==> Creating user configuration..."
    mkdir -p "$HOME/.config/cava-bg"
    if [ ! -f "$HOME/.config/cava-bg/config.toml" ]; then
        cp config.toml "$HOME/.config/cava-bg/config.toml"
        echo "    → ~/.config/cava-bg/config.toml (default config installed)"
    else
        echo "    → existing config kept at ~/.config/cava-bg/config.toml"
    fi
fi

echo ""
echo "==============================================="
echo " cava-bg has been installed successfully!"
echo "==============================================="
echo ""
echo "  Binary:  cava-bg ($( [ "$INSTALL_MODE" = "system" ] && echo "/usr/local/bin" || echo "~/.local/bin" ))"
echo "  Config:  ~/.config/cava-bg/config.toml"
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
