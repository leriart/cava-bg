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

# Highest released tag, as a bare version (no leading `v`).
# Empty when nothing has been released yet.
#
# The repository's existing tags are bare (`0.1.7` ... `0.2.5`), so that is the
# form we cut. The `v` prefix is accepted when reading so links written against
# a differently-tagged release keep working.
cava_bg_latest_tag_version() {
    # `|| true` matters: with `set -o pipefail` the `grep` that finds nothing
    # returns 1, which under `set -e` would abort the caller. "No tags yet" is a
    # perfectly valid answer, not an error.
    git -C "$CAVA_BG_ROOT" tag --list --sort=-v:refname 2>/dev/null \
        | grep -E '^v?[0-9]+\.[0-9]+\.[0-9]+$' \
        | head -n1 | sed 's/^v//' || true
}

# Version comparison using version-sort semantics, so 0.2.10 > 0.2.9.
cava_bg_version_gt() {
    [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n1)" = "$1" ]
}

# Next patch release after `$1`, e.g. 0.2.6 -> 0.2.7.
cava_bg_next_patch() {
    local current="$1"
    if [[ "$current" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
        printf '%s.%s.%s\n' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "$((BASH_REMATCH[3] + 1))"
    else
        # Pre-release or unparsable version: fall back to a literal patch bump
        # rather than silently producing something unbuildable.
        printf '%s.1\n' "$current"
    fi
}

# Rewrite the [package] version in Cargo.toml in place.
# Cargo.toml carries exactly one top-level `version = "..."` line, so this is
# unambiguous.
cava_bg_write_cargo_version() {
    local new="$1"
    local file="$CAVA_BG_ROOT/Cargo.toml"
    grep -q '^version = ' "$file" || {
        echo "error: no top-level version line in $file" >&2
        return 1
    }
    # Match the whole `version = "x.y.z"` including its closing quote. Reusing
    # just the leading pattern with `s//` would only replace `version = "` and
    # leave the old value dangling as: version = "0.2.7"0.2.6"
    sed -i "0,/^version = \"[^\"]*\"/s//version = \"$new\"/" "$file"
    grep -q "^version = \"$new\"$" "$file" || {
        echo "error: failed to set version to $new" >&2
        return 1
    }
}

# Decide the version to release, and bump Cargo.toml when needed.
#
# Rules, in order:
#   force     -> reuse the newest tag as-is (re-running a broken release)
#   explicit  -> dev already bumped Cargo.toml past the newest tag; honour it
#   same      -> nothing was bumped, so cut the next patch release
#   first     -> no tags at all; release whatever Cargo.toml says
#
# Echoes the version on stdout and a human-readable reason on stderr.
cava_bg_resolve_release_version() {
    local force="${1:-false}"
    local cargo_version latest

    cargo_version="$(cava_bg_version)"
    latest="$(cava_bg_latest_tag_version)"

    if [ "$force" = "true" ] && [ -n "$latest" ]; then
        echo "forced re-run; reusing already-released version $latest" >&2
        echo "$latest"
        return 0
    fi

    if [ -z "$latest" ]; then
        echo "no tags yet; releasing Cargo.toml version $cargo_version" >&2
        echo "$cargo_version"
        return 0
    fi

    if cava_bg_version_gt "$cargo_version" "$latest"; then
        echo "dev bumped $latest -> $cargo_version explicitly" >&2
        echo "$cargo_version"
        return 0
    fi

    if [ "$cargo_version" = "$latest" ]; then
        local next
        next="$(cava_bg_next_patch "$latest")"
        # Deliberately does NOT write Cargo.toml here.
        #
        # It used to, and because GitHub Actions steps share a workspace, the
        # later "commit the version bump" step found the file already updated,
        # took its `already at $WANTED` early exit, and never committed or
        # pushed anything. The tag then landed on a commit whose Cargo.toml
        # still held the previous version. Deciding and mutating are separate
        # steps on purpose; the caller writes the file when it is ready.
        echo "no bump on dev; cutting $latest -> $next" >&2
        echo "$next"
        return 0
    fi

    # Cargo.toml is *behind* the newest tag: someone force-pushed or reverted.
    # Trusting it would re-release an older version as if it were new.
    echo "error: Cargo.toml ($cargo_version) is behind the newest tag ($latest)." >&2
    echo "error: Refusing to release; bump version in Cargo.toml on dev first." >&2
    return 1
}
# ── Tests ──────────────────────────────────────────────────────────────────
# Run with `scripts/version.sh --test`.
# Deliberately a function, not inline code: the harness sources this file in a
# subshell that still sees the caller's `--test`, so testing only
# `[ "$1" = --test ]` recursed forever. Guarding on "executed directly" too
# means sourcing never re-enters anything.
cava_bg_run_tests() {
    set -uo pipefail
    ORIG_ROOT="$CAVA_BG_ROOT"
    cd "$CAVA_BG_ROOT"
    failures=0
    check() { # check <description> <expected> <actual>
        if [ "$2" = "$3" ]; then
            printf '  ok    %s\n' "$1"
        else
            printf '  FAIL  %s (expected %s, got %s)\n' "$1" "$2" "$3"
            failures=$((failures + 1))
        fi
    }

    check "next patch from 0.2.6"      "0.2.7"  "$(cava_bg_next_patch 0.2.6)"
    check "next patch carries digits"  "0.2.10" "$(cava_bg_next_patch 0.2.9)"
    check "next patch past 99"         "0.2.100" "$(cava_bg_next_patch 0.2.99)"
    check "next patch from 1.0.0"      "1.0.1"  "$(cava_bg_next_patch 1.0.0)"
    check "unparsable falls back"      "0.3-rc.1.1" "$(cava_bg_next_patch 0.3-rc.1)"

    # Version sort, not string sort: 0.2.10 must beat 0.2.9.
    check "gt 0.2.10 vs 0.2.9"   "yes" "$(cava_bg_version_gt 0.2.10 0.2.9 && echo yes || echo no)"
    check "gt 0.2.9 vs 0.2.10"   "no"  "$(cava_bg_version_gt 0.2.9 0.2.10 && echo yes || echo no)"
    check "gt equal"             "no"  "$(cava_bg_version_gt 0.2.6 0.2.6 && echo yes || echo no)"
    check "gt 1.0.0 vs 0.9.9"    "yes" "$(cava_bg_version_gt 1.0.0 0.9.9 && echo yes || echo no)"

    # Tag parsing is exercised against a throwaway repo with known tags rather
    # than this repository's own history: a real release cuts a new tag, which
    # would make an assertion like "the newest tag is 0.2.5" fail the next time
    # anyone publishes. Asserting on live state is what made this test fragile.
    tagged="$(mktemp -d)"
    git -C "$tagged" init -q .
    git -C "$tagged" -c user.email=a@b -c user.name=c commit -q --allow-empty -m init
    for t in 0.1.7 v0.2.5 0.2.6 v0.10.0 not-a-version; do
        git -C "$tagged" tag "$t"
    done
    saved_root="$CAVA_BG_ROOT"
    CAVA_BG_ROOT="$tagged"
    # 0.10.0 must beat 0.2.6 (version sort, not string sort) and come back
    # bare even though the tag on disk is `v0.10.0`.
    check "highest tag, v stripped"      "0.10.0" "$(cava_bg_latest_tag_version)"
    rm -rf "$tagged"
    CAVA_BG_ROOT="$saved_root"

    # "no tags yet" must exit 0. Under `set -e -o pipefail` the grep that finds
    # nothing returns 1, and that used to abort the very first release of a
    # repository.
    empty="$(mktemp -d)"
    git -C "$empty" init -q .
    CAVA_BG_ROOT="$empty"
    set -e -o pipefail
    got="$(cava_bg_latest_tag_version)"
    set +e
    CAVA_BG_ROOT="$ORIG_ROOT"
    check "no tags is not an error" "" "$got"
    rm -rf "$empty"

    # Rewriting Cargo.toml is the mechanism the release workflow uses to cut a
    # version, so make sure it is idempotent and touches the right line.
    tmp="$(mktemp -d)"
    mkdir -p "$tmp/scripts"
    cp "$CAVA_BG_ROOT/scripts/version.sh" "$tmp/scripts/version.sh"
    printf '[package]\nname = "x"\nversion = "0.2.6"\n\n[dependencies]\nserde = { version = "1.0" }\n' >"$tmp/Cargo.toml"
    (
      CAVA_BG_ROOT="$tmp"; export CAVA_BG_ROOT
      cd "$tmp" || exit 1
      source scripts/version.sh
      cava_bg_write_cargo_version 0.2.7
      cava_bg_write_cargo_version 0.2.7
    )
    check "bumped package version" "0.2.7"         "$(sed -n 's/^version = "\(.*\)"$/\1/p' "$tmp/Cargo.toml" | head -n1)"
    check "dependency untouched"   "yes"         "$(grep -q 'serde = { version = "1.0" }' "$tmp/Cargo.toml" && echo yes || echo no)"
    rm -rf "$tmp"

    if [ "$failures" -eq 0 ]; then
        echo "all version.sh tests passed"
    else
        echo "$failures test(s) failed"
        exit 1
    fi
    exit 0
}

# ── Entry point ────────────────────────────────────────────────────────────
# Run the entry point only when this file was *executed*, not sourced.
#
# `${BASH_SOURCE[0]}` vs `$0` is the usual check but compares strings, and both
# end up identical whenever the file is sourced from a different directory using
# a relative path -- which is what the test harness below does. `(return 0)`
# is the canonical bash probe instead: `return` is only legal inside a sourced
# script, so the subshell succeeds there and fails when executed directly.
if ! (return 0 2>/dev/null); then
    case "${1:-}" in
        --test) cava_bg_run_tests ;;
        *)      cava_bg_version ;;
    esac
fi
