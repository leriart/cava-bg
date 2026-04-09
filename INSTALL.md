# Installation Guide

## AUR Installation (Recommended for Arch Linux)

```bash
# Using paru
paru -S cava-bg

# Using yay
yay -S cava-bg
```

The AUR package will automatically:
1. Install cava if not present
2. Install all required dependencies
3. Build and install cava-bg

## Binary Installation

### 1. Download the latest release

```bash
# Download the binary archive
VERSION="0.1.0"
ARCH="x86_64-unknown-linux-gnu"
wget https://github.com/leriart/cava-bg/releases/download/v${VERSION}/cava-bg-v${VERSION}-${ARCH}.tar.gz

# Verify checksum
wget https://github.com/leriart/cava-bg/releases/download/v${VERSION}/cava-bg-v${VERSION}-${ARCH}.tar.gz.sha256
sha256sum -c cava-bg-v${VERSION}-${ARCH}.tar.gz.sha256

# Extract
tar -xzf cava-bg-v${VERSION}-${ARCH}.tar.gz
```

### 2. Install system-wide

```bash
# Copy to /usr/local/bin
sudo cp cava-bg /usr/local/bin/

# Or to ~/.local/bin (if in PATH)
mkdir -p ~/.local/bin
cp cava-bg ~/.local/bin/
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
mkdir -p ~/.config/cava-bg
cp config.toml ~/.config/cava-bg/
# Edit ~/.config/cava-bg/config.toml as needed
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
git clone https://github.com/leriart/cava-bg.git
cd cava-bg

# Build
cargo build --release

# Install
sudo cp target/release/cava-bg /usr/local/bin/
```

## Package Manager Installation

### AUR (Arch Linux)
```bash
# Using paru
paru -S cava-bg

# Using yay
yay -S cava-bg
```

### Nix
```bash
nix-env -iA cavabg
```

## Verification

After installation, verify it works:

```bash
# Check version
cava-bg --version

# Test run (should show visualizer)
cava-bg
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

1. Configure `~/.config/cava-bg/config.toml` to your liking
2. Add to Hyprland autostart: `exec-once = cava-bg`
3. Enjoy your audio visualizer!