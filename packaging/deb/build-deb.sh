#!/bin/bash
# cava-bg .deb builder
# Run this on a Debian/Ubuntu system with dpkg-deb available
set -e

VERSION="0.2.5"
ARCH="amd64"
PKG_NAME="cava-bg"
BUILD_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$BUILD_DIR/../.." && pwd)"
DEB_DIR="$PROJECT_DIR/target/deb/${PKG_NAME}_${VERSION}_${ARCH}"
DIST_DIR="$PROJECT_DIR/target/deb"

echo "=== Building cava-bg v${VERSION} .deb package ==="

# Check for dpkg-deb
if ! command -v dpkg-deb &> /dev/null; then
    echo "Error: dpkg-deb not found. Run this on Debian/Ubuntu."
    exit 1
fi

# Build release binary
cd "$PROJECT_DIR"
echo "Building release binary..."
cargo build --release

# Clean and prepare
rm -rf "$DEB_DIR"
mkdir -p "$DEB_DIR/DEBIAN"
mkdir -p "$DEB_DIR/usr/bin"
mkdir -p "$DEB_DIR/usr/share/doc/${PKG_NAME}"
mkdir -p "$DEB_DIR/usr/share/applications"
mkdir -p "$DEB_DIR/usr/share/licenses/${PKG_NAME}"

# Copy binary
cp target/release/${PKG_NAME} "$DEB_DIR/usr/bin/"

# Shell completions
mkdir -p "$DEB_DIR/usr/share/bash-completion/completions"
mkdir -p "$DEB_DIR/usr/share/zsh/site-functions"
mkdir -p "$DEB_DIR/usr/share/fish/vendor_completions.d"
cp target/release/completions/cava-bg.bash "$DEB_DIR/usr/share/bash-completion/completions/cava-bg"
cp target/release/completions/_cava-bg "$DEB_DIR/usr/share/zsh/site-functions/_cava-bg"
cp target/release/completions/cava-bg.fish "$DEB_DIR/usr/share/fish/vendor_completions.d/cava-bg.fish"

# Copy docs
cp README.md "$DEB_DIR/usr/share/doc/${PKG_NAME}/"
cp config.toml "$DEB_DIR/usr/share/doc/${PKG_NAME}/"
cp LICENSE "$DEB_DIR/usr/share/licenses/${PKG_NAME}/"

# Control file
cp "$BUILD_DIR/control" "$DEB_DIR/DEBIAN/"
sed -i "s/^Version:.*/Version: ${VERSION}/" "$DEB_DIR/DEBIAN/control"

# Postinst
cp "$BUILD_DIR/postinst" "$DEB_DIR/DEBIAN/"
chmod 755 "$DEB_DIR/DEBIAN/postinst"

# Desktop entry
cat > "$DEB_DIR/usr/share/applications/${PKG_NAME}.desktop" << 'EOF'
[Desktop Entry]
Name=cava-bg Config
Comment=Configure cava-bg audio visualizer
Exec=cava-bg --config
Icon=audio-card
Terminal=false
Type=Application
Categories=AudioVideo;Audio;
EOF

# Build .deb
mkdir -p "$DIST_DIR"
dpkg-deb --root-owner-group --build "$DEB_DIR" "$DIST_DIR/${PKG_NAME}_${VERSION}_${ARCH}.deb"

echo ""
echo "=== Package created ==="
ls -lh "$DIST_DIR/${PKG_NAME}_${VERSION}_${ARCH}.deb"
echo ""
echo "Install with: sudo dpkg -i $DIST_DIR/${PKG_NAME}_${VERSION}_${ARCH}.deb"
echo "Or: sudo apt install ./$DIST_DIR/${PKG_NAME}_${VERSION}_${ARCH}.deb"
