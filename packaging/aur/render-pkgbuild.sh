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

# .SRCINFO is required by the AUR and must agree with the PKGBUILD.
#
# This used to shell out to `makepkg --printsrcinfo`, an Arch-only tool: on the
# Ubuntu CI runner the file was simply never produced, which broke
# packaging/build-all.sh (its `cp .SRCINFO` failed under `set -e`) and left the
# AUR push adding only a PKGBUILD, leaving the two out of sync.
#
# Rather than parse the PKGBUILD with a hand-written parser — an earlier attempt
# at that swallowed the rest of the file the moment an array spanned lines — we
# let bash evaluate it. A PKGBUILD's top level is only variable assignments;
# build() and package() are definitions that are never called.
emit_srcinfo() {
    local pkgbuild="$1" out="$2"

    (
        # shellcheck disable=SC1090
        source "$pkgbuild"

        printf 'pkgbase = %s\n' "$pkgname"

        # `if` rather than `[ ... ] && ...`: under `set -e` a false test makes
        # the `&&` list return 1, which killed the subshell at the first empty
        # optional field (epoch) and truncated .SRCINFO after pkgrel.
        field() { if [ -n "${1:-}" ]; then printf '\t%s = %s\n' "$2" "$1"; fi; }

        field "$pkgdesc" pkgdesc
        field "$pkgver"  pkgver
        field "$pkgrel"  pkgrel
        field "${epoch:-}" epoch
        field "$url"     url
        field "${install:-}" install
        field "${changelog:-}" changelog
        for a in "${arch[@]}"; do        printf '\tarch = %s\n' "$a"; done
        for g in "${groups[@]}"; do      printf '\tgroups = %s\n' "$g"; done
        for l in "${license[@]}"; do     printf '\tlicense = %s\n' "$l"; done
        for p in "${provides[@]}"; do    printf '\tprovides = %s\n' "$p"; done
        for c in "${conflicts[@]}"; do   printf '\tconflicts = %s\n' "$c"; done
        for r in "${replaces[@]}"; do    printf '\treplaces = %s\n' "$r"; done
        # makepkg --printsrcinfo emits makedepends before depends.
        for m in "${makedepends[@]}"; do printf '\tmakedepends = %s\n' "$m"; done
        for d in "${depends[@]}"; do     printf '\tdepends = %s\n' "$d"; done
        for c in "${checkdepends[@]}"; do printf '\tcheckdepends = %s\n' "$c"; done
        for o in "${optdepends[@]}"; do  printf '\toptdepends = %s\n' "$o"; done
        for s in "${source[@]}"; do      printf '\tsource = %s\n' "$s"; done
        for h in "${sha256sums[@]}"; do  printf '\tsha256sums = %s\n' "$h"; done

        printf '\npkgname = %s\n' "$pkgname"
    ) >"$out"
}

emit_srcinfo "$PKGBUILD" "$OUT_DIR/.SRCINFO"

echo "==> Wrote $PKGBUILD and $OUT_DIR/.SRCINFO"

# Cross-check against the canonical tool when it exists, so a template change
# that the generator mishandles cannot slip through on a machine that has
# makepkg.
if command -v makepkg >/dev/null; then
    if ( cd "$OUT_DIR" && makepkg --printsrcinfo >.SRCINFO.makepkg 2>/dev/null ); then
        if ! diff -q "$OUT_DIR/.SRCINFO" "$OUT_DIR/.SRCINFO.makepkg" >/dev/null; then
            echo "warning: generated .SRCINFO differs from makepkg --printsrcinfo" >&2
            diff "$OUT_DIR/.SRCINFO" "$OUT_DIR/.SRCINFO.makepkg" >&2 || true
        fi
    fi
    rm -f "$OUT_DIR/.SRCINFO.makepkg"
fi
