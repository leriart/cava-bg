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
# Inside a container the checkout is owned by a different uid than the one
# running this, so git refuses to touch it as a dubious-ownership repository.
git config --global --get-all safe.directory >/dev/null 2>&1 || true
git config --global --add safe.directory "$PROJECT_DIR" 2>/dev/null || true
git archive --prefix="${PKG_NAME}-${VERSION}/" \
    -o "$RPMBUILD_DIR/SOURCES/${PKG_NAME}-${VERSION}.tar.gz" HEAD

# Render the spec from the template so `Version:` can never drift from Cargo.toml.
sed -e "s/@VERSION@/$VERSION/g" "$PROJECT_DIR/packaging/rpm/cava-bg.spec" \
    >"$RPMBUILD_DIR/SPECS/cava-bg.spec"

echo "Building RPM..."
# Tee the build output so a failure can be inspected afterwards: the RPM is
# built in a Fedora container whose logs are not reachable from here, and
# "rpmbuild exited non-zero" on its own says nothing about which BuildRequires
# or macro was wrong.
LOG="$SPEC_DIR/rpmbuild.log"
if ! rpmbuild -ba --define "_topdir $RPMBUILD_DIR" \
        --define "_rpmdir $SPEC_DIR" \
        "$RPMBUILD_DIR/SPECS/cava-bg.spec" 2>&1 | tee "$LOG"; then
    echo ""
    echo "=== rpmbuild failed; last 60 lines of $LOG ===" >&2
    tail -60 "$LOG" >&2
    exit 1
fi

echo ""
echo "=== Packages created ==="
find "$SPEC_DIR" -name "${PKG_NAME}-*.rpm" -exec ls -lh {} \;
