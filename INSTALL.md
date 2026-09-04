# Installation Guide

## Recommended: source build from `main`

By design, the canonical install path always builds from the current
`main` branch of the repo, never from a prebuilt artifact. That way
what you run is byte-for-byte the same code that lives on `main` at
the moment of the install.

```bash
curl -fsSL https://raw.githubusercontent.com/leriart/cava-bg/main/install.sh | bash
```

The installer accepts `--version v0.2.6` (or any tag / commit) to
pin a specific revision, and `--system` for a sudo'd install into
`/usr/local/bin`. See "From Source" below for the full option list
and the equivalent manual steps.

## AUR (Arch Linux)

```bash
# Using paru
paru -S cava-bg

# Using yay
yay -S cava-bg
```

The AUR package automatically:
1. Installs cava if not present
2. Installs all required dependencies (cava, ffmpeg, …)
3. Builds and installs cava-bg from the AUR tarball

## Binary installation (opt-in)

The GitHub Releases attach a prebuilt `x86_64-unknown-linux-gnu`
tarball for every published version. Use this if you want a 2-second
install without a Rust toolchain. The 0.2.5 binary is linked
against FFmpeg 8 (`libavutil.so.60`) and **will not work on a
system that has already moved to FFmpeg 9** — use 0.2.6 or newer
on FFmpeg 9 hosts.

### 1. Download the release you want

```bash
VERSION="0.2.6"
ARCH="x86_64-unknown-linux-gnu"
wget https://github.com/leriart/cava-bg/releases/download/v${VERSION}/cava-bg-${VERSION}-${ARCH}.tar.gz
wget https://github.com/leriart/cava-bg/releases/download/v${VERSION}/cava-bg-${VERSION}-${ARCH}.tar.gz.sha256
sha256sum -c cava-bg-${VERSION}-${ARCH}.tar.gz.sha256
tar -xzf cava-bg-${VERSION}-${ARCH}.tar.gz
```

### 2. Install

```bash
# System-wide
sudo cp cava-bg /usr/local/bin/

# Or user-only (if in PATH)
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

### One-line installer (recommended)

The canonical install path is `curl | bash` straight from `main`:

```bash
curl -fsSL https://raw.githubusercontent.com/leriart/cava-bg/main/install.sh | bash
```

By default the installer always builds from the current `main` branch
of the repo with `cargo build --release --locked`. This way what you
run is always byte-for-byte the same code that lives on `main` at
the moment of the install — no stale prebuilt, no cache, no surprises
when a release tarball happens to lag behind a fix. The prebuilt
GitHub release tarball is still available as an opt-in via `--binary`,
but the default `curl | bash` flow does not use it.

```bash
# Latest source from main, user install
curl -fsSL https://raw.githubusercontent.com/leriart/cava-bg/main/install.sh | bash

# Pin to a specific release tag
curl -fsSL https://raw.githubusercontent.com/leriart/cava-bg/main/install.sh | bash -s -- --version v0.2.6

# System-wide install (needs sudo)
curl -fsSL https://raw.githubusercontent.com/leriart/cava-bg/main/install.sh | bash -s -- --system

# Opt into the prebuilt release tarball (still falls back to source build)
curl -fsSL https://raw.githubusercontent.com/leriart/cava-bg/main/install.sh | bash -s -- --binary
```

### Manual build (no installer)

If you prefer to drive `cargo` yourself:

```bash
# Install build dependencies
# Arch Linux
sudo pacman -S cava base-devel pkg-config wayland wayland-protocols libxkbcommon ffmpeg
# Ubuntu / Debian
sudo apt install cava build-essential pkg-config libwayland-dev libegl-dev \
    mesa-common-dev libxkbcommon-dev wayland-protocols libavformat-dev \
    libavcodec-dev libavutil-dev libswscale-dev

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

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