#!/bin/bash
# cava-bg — Build ALL packaging artifacts
# Run from the project root: ./packaging/build-all.sh
# Requires: cargo, cargo-deb, cargo-generate-rpm, git, tar
set -e

export PATH="$HOME/.cargo/bin:$PATH"

VERSION="0.2.4"
PKG_NAME="cava-bg"
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="$PROJECT_DIR/target/dist"

cd "$PROJECT_DIR"

echo "╔════════════════════════════════════════════════╗"
echo "║  cava-bg v${VERSION} — Package All the Things  ║"
echo "╚════════════════════════════════════════════════╝"
echo ""

# Clean dist dir
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

# ─── 1. Build release binary ───
echo "┌─ [1/6] Building release binary..."
cargo build --release
echo "├─ Done: target/release/${PKG_NAME}"
echo ""

# ─── 2. Source tarball (.tar.gz) ───
echo "┌─ [2/6] Creating source tarball..."
git archive --prefix="${PKG_NAME}-${VERSION}/" \
    -o "$DIST_DIR/${PKG_NAME}-${VERSION}.tar.gz" HEAD 2>/dev/null || {
    cd /tmp
    rm -rf "$PKG_NAME-$VERSION"
    cp -r "$PROJECT_DIR" "$PKG_NAME-$VERSION"
    rm -rf "$PKG_NAME-$VERSION/.git" "$PKG_NAME-$VERSION/target"
    tar czf "$DIST_DIR/${PKG_NAME}-${VERSION}.tar.gz" "$PKG_NAME-$VERSION"
    rm -rf "$PKG_NAME-$VERSION"
    cd "$PROJECT_DIR"
}
sha256sum "$DIST_DIR/${PKG_NAME}-${VERSION}.tar.gz" > "$DIST_DIR/${PKG_NAME}-${VERSION}.tar.gz.sha256"
echo "├─ Done: ${PKG_NAME}-${VERSION}.tar.gz"
echo ""

# ─── 3. Binary tarball (.tar.gz) ───
echo "┌─ [3/6] Creating binary tarball..."
ARCH="x86_64-unknown-linux-gnu"
BIN_DIR="$DIST_DIR/${PKG_NAME}-${VERSION}-${ARCH}"
mkdir -p "$BIN_DIR"
cp target/release/${PKG_NAME} "$BIN_DIR/"
cp config.toml "$BIN_DIR/"
cp README.md "$BIN_DIR/"
cp LICENSE "$BIN_DIR/"
tar czf "$DIST_DIR/${PKG_NAME}-${VERSION}-${ARCH}.tar.gz" -C "$DIST_DIR" "${PKG_NAME}-${VERSION}-${ARCH}"
sha256sum "$DIST_DIR/${PKG_NAME}-${VERSION}-${ARCH}.tar.gz" > "$DIST_DIR/${PKG_NAME}-${VERSION}-${ARCH}.tar.gz.sha256"
rm -rf "$BIN_DIR"
echo "├─ Done: ${PKG_NAME}-${VERSION}-${ARCH}.tar.gz"
echo ""

# ─── 4. DEB package ───
echo "┌─ [4/6] Creating .deb package..."
if command -v cargo-deb &> /dev/null; then
    cargo deb
    cp target/debian/${PKG_NAME}_${VERSION}-1_amd64.deb "$DIST_DIR/" 2>/dev/null || \
    cp target/debian/${PKG_NAME}_*.deb "$DIST_DIR/"
    echo "├─ Done (cargo-deb): ${PKG_NAME}_${VERSION}-1_amd64.deb"
else
    echo "├─ cargo-deb not installed. Run: cargo install cargo-deb"
    echo "├─ Or use manual build: ./packaging/deb/build-deb.sh"
fi
echo ""

# ─── 5. RPM package ───
echo "┌─ [5/6] Creating .rpm package..."
if command -v cargo-generate-rpm &> /dev/null; then
    cargo generate-rpm
    cp target/generate-rpm/${PKG_NAME}-${VERSION}-1.x86_64.rpm "$DIST_DIR/" 2>/dev/null || \
    cp target/generate-rpm/${PKG_NAME}-*.rpm "$DIST_DIR/"
    echo "├─ Done (cargo-generate-rpm): ${PKG_NAME}-${VERSION}-1.x86_64.rpm"
else
    echo "├─ cargo-generate-rpm not installed. Run: cargo install cargo-generate-rpm"
    echo "├─ Or use manual build: ./packaging/rpm/build-rpm.sh"
fi
echo ""

# ─── 6. AUR submission tarball ───
echo "┌─ [6/6] Creating AUR submission tarball..."
AUR_DIR="$DIST_DIR/aur-${PKG_NAME}"
rm -rf "$AUR_DIR"
mkdir -p "$AUR_DIR"
cp packaging/aur/PKGBUILD "$AUR_DIR/"
cp packaging/aur/.SRCINFO "$AUR_DIR/"
tar czf "$DIST_DIR/${PKG_NAME}-aur-${VERSION}.tar.gz" -C "$DIST_DIR" "aur-${PKG_NAME}"
rm -rf "$AUR_DIR"
echo "├─ Done: ${PKG_NAME}-aur-${VERSION}.tar.gz"
echo ""

# ─── Summary ───
echo "╔════════════════════════════════════════════════╗"
echo "║  All artifacts ready in target/dist/           ║"
echo "╚════════════════════════════════════════════════╝"
echo ""
ls -lh "$DIST_DIR/"
echo ""
echo "=== What was generated ==="
echo ""
echo "  Source tarball:"
echo "    target/dist/${PKG_NAME}-${VERSION}.tar.gz"
echo "    target/dist/${PKG_NAME}-${VERSION}.tar.gz.sha256"
echo ""
echo "  Binary tarball:"
echo "    target/dist/${PKG_NAME}-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
echo "    target/dist/${PKG_NAME}-${VERSION}-x86_64-unknown-linux-gnu.tar.gz.sha256"
echo ""
echo "  DEB package:"
echo "    target/dist/${PKG_NAME}_${VERSION}-1_amd64.deb"
echo ""
echo "  RPM package:"
echo "    target/dist/${PKG_NAME}-${VERSION}-1.x86_64.rpm"
echo ""
echo "  AUR package (PKGBUILD + .SRCINFO):"
echo "    target/dist/${PKG_NAME}-aur-${VERSION}.tar.gz"
echo "    → Upload to AUR: https://aur.archlinux.org/packages/cava-bg"
echo ""
echo "=== GitHub Release Instructions ==="
echo ""
echo "  1. git tag v${VERSION} && git push --tags"
echo "  2. Upload these to GitHub release:"
echo "     - target/dist/${PKG_NAME}-${VERSION}.tar.gz"
echo "     - target/dist/${PKG_NAME}-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
echo "     - target/dist/${PKG_NAME}_${VERSION}-1_amd64.deb"
echo "     - target/dist/${PKG_NAME}-${VERSION}-1.x86_64.rpm"
echo "     - target/dist/${PKG_NAME}-${VERSION}.tar.gz.sha256"
echo "     - target/dist/${PKG_NAME}-${VERSION}-x86_64-unknown-linux-gnu.tar.gz.sha256"
echo ""
echo "  3. AUR: upload PKGBUILD + .SRCINFO → https://aur.archlinux.org/packages/${PKG_NAME}"
echo ""
echo "  ¡Listo, compadre! 🚀"
