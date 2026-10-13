#!/usr/bin/env bash
# cava-bg .deb builder — no cargo-deb required, only dpkg-deb.
#
# Usage:  ./packaging/deb/build-deb.sh [VERSION]
# Needs to run on a Debian/Ubuntu host (or a container with dpkg-deb).
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

PROJECT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=../../scripts/version.sh
source "$PROJECT_DIR/scripts/version.sh"
export CAVA_BG_ROOT="$PROJECT_DIR"

PKG_NAME="$CAVA_BG_NAME"
VERSION="${1:-${VERSION:-$(cava_bg_version)}}"
ARCH="${DEB_ARCH:-$(dpkg --print-architecture 2>/dev/null || echo amd64)}"
BUILD_DIR="$PROJECT_DIR/packaging/deb"
DEB_DIR="$PROJECT_DIR/target/deb/${PKG_NAME}_${VERSION}_${ARCH}"
DIST_DIR="${OUT_DIR:-$PROJECT_DIR/target/dist}"

echo "=== Building cava-bg v${VERSION} .deb package (${ARCH}) ==="

command -v dpkg-deb >/dev/null || {
    echo "error: dpkg-deb not found. Run this on Debian/Ubuntu, or install cargo-deb." >&2
    exit 1
}

if [ ! -x "$PROJECT_DIR/target/release/${PKG_NAME}" ]; then
    echo "Building release binary..."
    cava_bg_build_release
fi

rm -rf "$DEB_DIR"
mkdir -p "$DEB_DIR/DEBIAN"
mkdir -p "$DEB_DIR/usr/bin"
mkdir -p "$DEB_DIR/usr/share/doc/${PKG_NAME}"
mkdir -p "$DEB_DIR/usr/share/applications"
mkdir -p "$DEB_DIR/usr/share/licenses/${PKG_NAME}"
# dpkg-deb --build does not create the destination directory, so this has to
# exist before it runs or the whole package build fails at the very last step,
# after the two-minute release compile.
mkdir -p "$DIST_DIR"

cp "target/release/${PKG_NAME}" "$DEB_DIR/usr/bin/"

if [ -d target/release/completions ]; then
    mkdir -p "$DEB_DIR/usr/share/bash-completion/completions"
    mkdir -p "$DEB_DIR/usr/share/zsh/site-functions"
    mkdir -p "$DEB_DIR/usr/share/fish/vendor_completions.d"
    cp target/release/completions/cava-bg.bash "$DEB_DIR/usr/share/bash-completion/completions/cava-bg"
    cp target/release/completions/_cava-bg "$DEB_DIR/usr/share/zsh/site-functions/_cava-bg"
    cp target/release/completions/cava-bg.fish "$DEB_DIR/usr/share/fish/vendor_completions.d/cava-bg.fish"
else
    echo "warning: target/release/completions missing, packaging without completions" >&2
fi

cp README.md "$DEB_DIR/usr/share/doc/${PKG_NAME}/"
cp LICENSE "$DEB_DIR/usr/share/licenses/${PKG_NAME}/"

# Default config, emitted by the binary itself.
"target/release/${PKG_NAME}" __default-config >"$DEB_DIR/usr/share/doc/${PKG_NAME}/config.toml"

# Single shared desktop entry (the previously inlined one shipped
# `Exec=cava-bg --config`, which starts the visualiser instead of the GUI).
cp "$PROJECT_DIR/packaging/cava-bg.desktop" "$DEB_DIR/usr/share/applications/"

# systemd user unit, so `systemctl --user enable cava-bg` works out of the box.
mkdir -p "$DEB_DIR/usr/lib/systemd/user"
cp "$PROJECT_DIR/packaging/systemd/cava-bg.service" "$DEB_DIR/usr/lib/systemd/user/"

sed -e "s/^Version:.*/Version: ${VERSION}/" -e "s/^Architecture:.*/Architecture: ${ARCH}/" \
    "$BUILD_DIR/control" >"$DEB_DIR/DEBIAN/control"
cp "$BUILD_DIR/postinst" "$DEB_DIR/DEBIAN/"
chmod 755 "$DEB_DIR/DEBIAN/postinst"

# Fail loudly rather than shipping an unpackable package.
dpkg-deb --root-owner-group --build "$DEB_DIR" "$DIST_DIR/${PKG_NAME}_${VERSION}_${ARCH}.deb"

echo ""
echo "=== Package created ==="
ls -lh "$DIST_DIR/${PKG_NAME}_${VERSION}_${ARCH}.deb"
echo "Install with: sudo apt install ./${PKG_NAME}_${VERSION}_${ARCH}.deb"
