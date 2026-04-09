# cava-bg AUR Package

This directory contains the files needed to publish cava-bg to the Arch User Repository (AUR).

## Files

- `PKGBUILD` - The build script for Arch Linux
- `.SRCINFO` - Package metadata for AUR
- `AUR-README.md` - This file

## Publishing to AUR

### Prerequisites

1. Install `git` and `base-devel`:
   ```bash
   sudo pacman -S git base-devel
   ```

2. Set up AUR SSH key (recommended) or use HTTPS.

### Publishing Steps

1. Clone the AUR repository:
   ```bash
   git clone ssh://aur@aur.archlinux.org/cava-bg.git
   cd cava-bg
   ```

2. Copy the AUR files:
   ```bash
   cp /path/to/cava-bg/PKGBUILD .
   cp /path/to/cava-bg/.SRCINFO .
   ```

3. Verify the package builds:
   ```bash
   makepkg -si
   ```

4. Commit and push:
   ```bash
   git add PKGBUILD .SRCINFO
   git commit -m "Initial release v0.1.0"
   git push origin master
   ```

## Updating the Package

1. Update version in `PKGBUILD` (pkgver)
2. Update checksums if source URL changed
3. Regenerate `.SRCINFO`:
   ```bash
   makepkg --printsrcinfo > .SRCINFO
   ```
4. Commit and push changes

## Package Features

- Automatically installs `cava` as a dependency
- Includes systemd user service for autostart
- Provides example configuration
- Post-install instructions
- Supports adaptive color detection from wallpapers

## Testing Locally

```bash
# Build and install without pushing to AUR
makepkg -