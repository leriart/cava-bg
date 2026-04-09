# Installation Guide

## Binary Installation (Recommended)

### 1. Download the latest release

```bash
# Download the binary archive
VERSION="0.1.0"
ARCH="x86_64-unknown-linux-gnu"
wget https://github.com/yourusername/cavabg/releases/download/v${VERSION}/cavabg-v${VERSION}-${ARCH}.tar.gz

# Verify checksum
wget https://github.com/yourusername/cavabg/releases/download/v${VERSION}/cavabg-v${VERSION}-${ARCH}.tar.gz.sha256
sha256sum -c cavabg-v${VERSION}-${ARCH}.tar.gz.sha256

# Extract
tar -xzf cavabg-v${VERSION}-${ARCH}.tar.gz
```

### 2. Install system-wide

```bash
# Copy to /usr/local/bin
sudo cp cavabg /usr/local/bin/

# Or to ~/.local/bin (if in PATH)
mkdir -p ~/.local/bin
cp cavabg ~/.local/bin/
```

### 3. Install cava (required)

```bash
# Arch Linux
sudo pacman -S cava

# Ubuntu/Debian
sudo apt install cava

# Fedora
sudo dnf install cava
```

### 4. Create configuration

```bash
mkdir -p ~/.config/cavabg
cp config.toml ~/.config/cavabg/
# Edit ~/.config/cavabg/config.toml as needed
```

## From Source

### Prerequisites

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Install build dependencies
# Arch Linux
sudo pacman -S cava base-devel pkg-config wayland wayland-protocols libxkbcommon

# Ubuntu/Debian
sudo apt install cava build-essential pkg-config libwayland-dev libegl-dev mesa-common-dev libxkbcommon-dev wayland-protocols
```

### Build and Install

```bash
# Clone repository
git clone https://github.com/yourusername/cavabg.git
cd cavabg

# Build
cargo build --release

# Install
sudo cp target/release/cavabg /usr/local/bin/
```

## Package Manager Installation (Future)

### AUR (Arch Linux)
```bash
yay -S cavabg
```

### Nix
```bash
nix-env -iA cavabg
```

## Verification

After installation, verify it works:

```bash
# Check version
cavabg --version

# Test run (should show visualizer)
cavabg
```

## Troubleshooting

### "cava not found"
Install cava as shown above.

### "wl_compositor not available"
Make sure you're running under Wayland:
```bash
echo $XDG_SESSION_TYPE
# Should output "wayland"
```

### Permission denied
Make binary executable:
```bash
chmod +x /path/to/cavabg
```

## Uninstallation

### Binary installation
```bash
sudo rm /usr/local/bin/cavabg
# or
rm ~/.local/bin/cavabg
```

### Source installation
```bash
# Remove binary
sudo rm /usr/local/bin/cavabg

# Remove configuration (optional)
rm -rf ~/.config/cavabg
```

## Next Steps

1. Configure `~/.config/cavabg/config.toml` to your liking
2. Add to Hyprland autostart: `exec-once = cavabg`
3. Enjoy your audio visualizer!