#!/usr/bin/env bash
# cava-bg .rpm builder — requires rpmbuild (Fedora / openSUSE / Arch's `rpm`).
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

PROJECT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=../../scripts/version.sh
source "$PROJECT_DIR/scripts/version.sh"
export CAVA_BG_ROOT="$PROJECT_DIR"

PKG_NAME="$CAVA_BG_NAME"
VERSION="${1:-${VERSION:-$(cava_bg_version)}}"
RPMBUILD_DIR="${RPMBUILD_DIR:-$HOME/rpmbuild}"
SPEC_DIR="$PROJECT_DIR/target/rpm"

echo "=== Building cava-bg v${VERSION} .rpm package ==="

command -v rpmbuild >/dev/null || {
    echo "error: rpmbuild not found. Install rpm-build / rpmdevtools." >&2
    exit 1
}

mkdir -p "$RPMBUILD_DIR"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}
mkdir -p "$SPEC_DIR"

cd "$PROJECT_DIR"
echo "Creating source tarball..."
git archive --prefix="${PKG_NAME}-${VERSION}/" \
    -o "$RPMBUILD_DIR/SOURCES/${PKG_NAME}-${VERSION}.tar.gz" HEAD

# Render the spec from the template so `Version:` can never drift from Cargo.toml.
sed -e "s/@VERSION@/$VERSION/g" "$PROJECT_DIR/packaging/rpm/cava-bg.spec" \
    >"$RPMBUILD_DIR/SPECS/cava-bg.spec"

echo "Building RPM..."
rpmbuild -ba --define "_topdir $RPMBUILD_DIR" \
    --define "_rpmdir $SPEC_DIR" \
    "$RPMBUILD_DIR/SPECS/cava-bg.spec"

echo ""
echo "=== Packages created ==="
find "$SPEC_DIR" -name "${PKG_NAME}-*.rpm" -exec ls -lh {} \;
