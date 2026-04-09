#!/bin/bash

# cava-bg installation script
set -e

echo "Installing cava-bg..."

# Check for cargo
if ! command -v cargo &> /dev/null; then
    echo "Error: Rust cargo not found. Please install Rust first."
    echo "Visit https://rustup.rs/ for installation instructions."
    exit 1
fi

# Check for cava
if ! command -v cava &> /dev/null; then
    echo "Warning: cava not found. Audio visualization will not work."
    echo "Install cava with: sudo pacman -S cava (Arch) or equivalent for your distro."
fi

# Check and install cava if needed
if ! command -v cava &> /dev/null; then
    echo "Installing cava..."
    # Try to detect package manager
    if command -v pacman &> /dev/null; then
        sudo pacman -S --noconfirm cava
    elif command -v apt &> /dev/null; then
        sudo apt update && sudo apt install -y cava
    elif command -v dnf &> /dev/null; then
        sudo dnf install -y cava
    else
        echo "Warning: Could not install cava automatically. Please install cava manually."
        echo "Arch: sudo pacman -S cava"
        echo "Debian/Ubuntu: sudo apt install cava"
        echo "Fedora: sudo dnf install cava"
    fi
fi

# Build in release mode
echo "Building cava-bg..."
cargo build --release

# Create config directory
echo "Creating config directory..."
mkdir -p ~/.config/cava-bg

# Copy default config if it doesn't exist
if [ ! -f ~/.config/cava-bg/config.toml ]; then
    echo "Copying default configuration..."
    cp config.toml ~/.config/cava-bg/
else
    echo "Configuration already exists at ~/.config/cava-bg/config.toml"
fi

# Offer to install system-wide
read -p "Install system-wide to /usr/local/bin? [y/N] " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    echo "Installing to /usr/local/bin..."
    sudo cp target/release/cava-bg /usr/local/bin/
    echo "Installation complete! Run 'cava-bg' to start."
else
    echo "Installation complete! Run './target/release/cava-bg' to start."
    echo "You can also copy the binary to your PATH manually."
fi

echo ""
echo "Features:"
echo "  • Adaptive gradient colors from wallpaper"
echo "  • Automatic wallpaper change detection"
echo "  • Real-time audio visualization with cava"
echo "  • Hyprland optimized with wlr-layer-shell"
echo ""
echo "To use with Hyprland, add to your hyprland.conf:"
echo "exec-once = cava-bg"
echo ""
echo "Configuration: ~/.config/cava-bg/config.toml"
echo "  • Set 'auto_detect_wallpaper_changes' to enable/disable wallpaper tracking"
echo "  • Adjust 'wallpaper_check_interval' for change detection frequency"
echo ""
echo "For AUR installation: yay -S cava-bg  or  paru -S cava-bg"