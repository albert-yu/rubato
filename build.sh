#!/bin/bash
set -e

echo "Building rubato..."
cargo build --release

echo "Creating dist directory..."
mkdir -p dist

echo "Copying binary..."
cp target/release/rubato dist/

echo "Copying assets..."
cp -r assets dist/

echo "Build complete! Artifacts are in dist/"
