#!/bin/bash

# Cavabg Release Builder
set -e

VERSION="0.1.0"
RELEASE_DIR="target/release"
DIST_DIR="dist"
ARCH="x86_64-unknown-linux-gnu"

echo "Building Cavabg Release v$VERSION..."

# Clean previous builds
echo "Cleaning previous builds..."
cargo clean

# Build release binary
echo "Building release binary..."
cargo build --release

# Create distribution directory
echo "Creating distribution directory..."
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

# Copy binary
echo "Copying binary..."
cp "$RELEASE_DIR/cavabg" "$DIST_DIR/"

# Create archive
echo "Creating archive..."
cd "$DIST_DIR"
tar -czf "../cavabg-v$VERSION-$ARCH.tar.gz" cavabg
cd ..

# Create checksum
echo "Creating checksum..."
sha256sum "cavabg-v$VERSION-$ARCH.tar.gz" > "cavabg-v$VERSION-$ARCH.tar.gz.sha256"

echo ""
echo "Release built successfully!"
echo "Files created:"
echo "  - dist/cavabg (binary)"
echo "  - cavabg-v$VERSION-$ARCH.tar.gz (archive)"
echo "  - cavabg-v$VERSION-$ARCH.tar.gz.sha256 (checksum)"
echo ""
echo "To create a GitHub release:"
echo "1. Create a new release on GitHub"
echo "2. Upload the .tar.gz and .sha256 files"
echo "3. Copy the install instructions from INSTALL.md"