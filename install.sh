#!/bin/bash

# Cavabg installation script
set -e

echo "Installing Cavabg..."

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

# Build in release mode
echo "Building Cavabg..."
cargo build --release

# Create config directory
echo "Creating config directory..."
mkdir -p ~/.config/cavabg

# Copy default config if it doesn't exist
if [ ! -f ~/.config/cavabg/config.toml ]; then
    echo "Copying default configuration..."
    cp config.toml ~/.config/cavabg/
else
    echo "Configuration already exists at ~/.config/cavabg/config.toml"
fi

# Offer to install system-wide
read -p "Install system-wide to /usr/local/bin? [y/N] " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    echo "Installing to /usr/local/bin..."
    sudo cp target/release/cavabg /usr/local/bin/
    echo "Installation complete! Run 'cavabg' to start."
else
    echo "Installation complete! Run './target/release/cavabg' to start."
    echo "You can also copy the binary to your PATH manually."
fi

echo ""
echo "To use with Hyprland, add to your hyprland.conf:"
echo "exec-once = cavabg"
echo ""
echo "For more configuration options, see ~/.config/cavabg/config.toml"