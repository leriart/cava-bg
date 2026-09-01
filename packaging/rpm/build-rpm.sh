#!/bin/bash
# cava-bg .rpm builder
# Run this on a Fedora/RHEL/openSUSE system with rpmbuild available
set -e

VERSION="0.2.6"
PKG_NAME="cava-bg"
BUILD_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$BUILD_DIR/../.." && pwd)"

echo "=== Building cava-bg v${VERSION} .rpm package ==="

# Check for rpmbuild
if ! command -v rpmbuild &> /dev/null; then
    echo "Error: rpmbuild not found. Install rpm-build package."
    echo "  Fedora: sudo dnf install rpm-build rpmdevtools"
    echo "  openSUSE: sudo zypper install rpm-build"
    exit 1
fi

# Setup rpmbuild tree
RPMBUILD_DIR="$HOME/rpmbuild"
mkdir -p "$RPMBUILD_DIR"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

# Build release binary
cd "$PROJECT_DIR"
echo "Building release binary..."
cargo build --release

# Create source tarball from current directory
echo "Creating source tarball..."
cd "$PROJECT_DIR"
git archive --prefix="${PKG_NAME}-${VERSION}/" -o "$RPMBUILD_DIR/SOURCES/${PKG_NAME}-${VERSION}.tar.gz" HEAD 2>/dev/null || {
    # Fallback: create tarball manually
    cd /tmp
    rm -rf "${PKG_NAME}-${VERSION}"
    cp -r "$PROJECT_DIR" "${PKG_NAME}-${VERSION}"
    rm -rf "${PKG_NAME}-${VERSION}/.git" "${PKG_NAME}-${VERSION}/target"
    tar czf "$RPMBUILD_DIR/SOURCES/${PKG_NAME}-${VERSION}.tar.gz" "${PKG_NAME}-${VERSION}"
    rm -rf "${PKG_NAME}-${VERSION}"
}

# Copy spec
cp "$BUILD_DIR/cava-bg.spec" "$RPMBUILD_DIR/SPECS/"

# Build RPM
echo "Building RPM..."
rpmbuild -ba "$RPMBUILD_DIR/SPECS/cava-bg.spec"

echo ""
echo "=== Package created ==="
find "$RPMBUILD_DIR/RPMS" -name "${PKG_NAME}-*.rpm" -exec ls -lh {} \;
echo ""
echo "Install with: sudo dnf install $RPMBUILD_DIR/RPMS/x86_64/${PKG_NAME}-${VERSION}-*.rpm"
