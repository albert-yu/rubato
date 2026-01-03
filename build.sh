#!/bin/bash
set -e

echo "Building rubato for Linux..."
# Source - https://stackoverflow.com/a
# Posted by cafce25
# Retrieved 2026-01-02, License - CC BY-SA 4.0

rustup target add x86_64-unknown-linux-gnu
cargo build --target x86_64-unknown-linux-gnu --release

echo "Creating dist directory..."
mkdir -p dist

echo "Copying binary..."
cp target/release/rubato dist/

echo "Copying assets..."
cp -r assets dist/

echo "Build complete! Artifacts are in dist/"
