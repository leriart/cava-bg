#!/usr/bin/env bash
# cava-bg — shared helpers for the build/packaging scripts.
#
# The canonical version lives in Cargo.toml. Nothing else in the tree may
# hardcode it; every script and workflow sources `version.sh` instead.
# Previously the version was duplicated in 7 files and a release required
# editing all of them by hand.
#
# Usage:  source "$(dirname "$0")/version.sh"    # or  scripts/version.sh

set -euo pipefail

CAVA_BG_ROOT="${CAVA_BG_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
CAVA_BG_NAME="cava-bg"

# Reads `version = "x.y.z"` out of Cargo.toml without requiring cargo.
cava_bg_version() {
    local cargo_toml="$CAVA_BG_ROOT/Cargo.toml"
    if [ ! -f "$cargo_toml" ]; then
        echo "error: Cargo.toml not found at $cargo_toml" >&2
        return 1
    fi
    sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$cargo_toml" | head -n1
}

# Derives the release tag (`v0.2.6`) from Cargo.toml.
cava_bg_tag() {
    printf 'v%s\n' "$(cava_bg_version)"
}

# `false` when this tree carries modifications that should not be packaged.
cava_bg_is_dirty() {
    [ -n "$(git -C "$CAVA_BG_ROOT" status --porcelain 2>/dev/null || true)" ]
}

# Build the release binary unless the caller already built a specific target.
# Honours CARGO_BUILD_ARGS so CI can pass `--locked --offline` etc.
cava_bg_build_release() {
    echo "==> cargo build --release (cava-bg $(cava_bg_version))"
    ( cd "$CAVA_BG_ROOT" && cargo build --release ${CARGO_BUILD_ARGS:-} )
}

# Refuse to continue on a dirty tree: a tarball built from uncommitted files
# cannot be reproduced or audited from the release tag.
cava_bg_require_clean_tree() {
    if cava_bg_is_dirty; then
        echo "error: working tree has uncommitted changes; commit or stash first." >&2
        git -C "$CAVA_BG_ROOT" status --short >&2
        return 1
    fi
}

# Emit a sha256 file next to an artifact (portable, GNU/BSD compatible).
cava_bg_sha256() {
    local artifact="$1"
    local dir base
    dir="$(dirname "$artifact")"
    base="$(basename "$artifact")"
    ( cd "$dir" && sha256sum "$base" >"$base.sha256" )
}

# Only usable when executed directly (not sourced): print the version.
if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
    cava_bg_version
fi
