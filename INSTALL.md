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
VERSION="0.2.4"
ARCH="x86_64-unknown-linux-gnu"
wget https://github.com/leriart/cava-bg/releases/download/${VERSION}/cava-bg-${VERSION}-${ARCH}.tar.gz

# Verify checksum
wget https://github.com/leriart/cava-bg/releases/download/${VERSION}/cava-bg-${VERSION}-${ARCH}.tar.gz.sha256
sha256sum -c cava-bg-${VERSION}-${ARCH}.tar.gz.sha256

# Extract
tar -xzf cava-bg-${VERSION}-${ARCH}.tar.gz
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
chmod +x /path/to/cava-bg
```

## Systemd Service

Install the systemd user service for automatic startup:

```bash
# Copy the service file
cp cava-bg.service ~/.config/systemd/user/

# Enable and start
systemctl --user enable --now cava-bg

# Check status
systemctl --user status cava-bg
```

The service file supports both single-process mode (default) and per-output supervisor mode (uncomment `--supervisor`). Hardware watchdog can be enabled by uncommenting `WatchdogSec=30`.

## Shell Completions

Shell completions (bash/zsh/fish) are automatically generated at build time. Install via `./install.sh` or copy manually from `target/release/completions/`:

```bash
# Bash
mkdir -p ~/.local/share/bash-completion/completions
cp target/release/completions/cava-bg.bash ~/.local/share/bash-completion/completions/cava-bg

# Zsh
mkdir -p ~/.local/share/zsh/site-functions
cp target/release/completions/_cava-bg ~/.local/share/zsh/site-functions/_cava-bg

# Fish
mkdir -p ~/.config/fish/completions
cp target/release/completions/cava-bg.fish ~/.config/fish/completions/cava-bg.fish
```

## Uninstallation

### Binary installation
```bash
sudo rm /usr/local/bin/cava-bg
# or
rm ~/.local/bin/cava-bg
```

### Source installation
```bash
# Remove binary
sudo rm /usr/local/bin/cava-bg

# Remove configuration (optional)
rm -rf ~/.config/cava-bg
```

## Next Steps

1. Configure `~/.config/cava-bg/config.toml` to your liking
2. Add to Hyprland autostart: `exec-once = cava-bg`
3. Enjoy your audio visualizer!