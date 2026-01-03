#!/bin/bash
set -e

echo "Updating server..."

git pull && cargo build --release && sudo systemctl restart rubato

echo "Update complete!"
