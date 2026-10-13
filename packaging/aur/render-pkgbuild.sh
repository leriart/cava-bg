#!/usr/bin/env bash
# Render packaging/aur/PKGBUILD.in + regenerate .SRCINFO.
#
# The PKGBUILD is a template so that `pkgver` and `sha256sums` always come from
# Cargo.toml and from the checksum the CI job actually uploaded. The rendered
# result is what gets committed to the AUR repository.
#
# Usage:  VERSION=0.2.6 ARCH=x86_64 SHA256=... ./packaging/aur/render-pkgbuild.sh
#         (when SHA256 is omitted the template's @SHA256SUM@ placeholder is kept,
#          which makes `makepkg` fail loudly instead of building a wrong package)
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=../../scripts/version.sh
source "$PROJECT_DIR/scripts/version.sh"
export CAVA_BG_ROOT="$PROJECT_DIR"

VERSION="${VERSION:-$(cava_bg_version)}"
ARCH="${ARCH:-$(rustc -vV | sed -n 's/^host: //p')}"
SHA256="${SHA256:-@SHA256SUM@}"

TEMPLATE="$PROJECT_DIR/packaging/aur/PKGBUILD.in"
OUT_DIR="${OUT_DIR:-$PROJECT_DIR/target/dist/aur-files}"
[ -d "$OUT_DIR" ] || mkdir -p "$OUT_DIR"

render() {
    sed -e "s|@VERSION@|$VERSION|g" \
        -e "s|@ARCH@|$ARCH|g" \
        -e "s|@SHA256SUM@|$SHA256|g" \
        "$TEMPLATE"
}

PKGBUILD="$OUT_DIR/PKGBUILD"
render >"$PKGBUILD"

# .SRCINFO is what makepkg --printsrcinfo emits and what the AUR web UI and
# `git aur` consume. Only regenerate it when makepkg is actually available;
# otherwise ship the PKGBUILD alone (the AUR only requires PKGBUILD).
if command -v makepkg >/dev/null; then
    ( cd "$OUT_DIR" && makepkg --printsrcinfo >.SRCINFO )
    echo "==> Wrote $PKGBUILD and $OUT_DIR/.SRCINFO"
else
    echo "==> Wrote $PKGBUILD (makepkg not available, skipped .SRCINFO)"
fi
