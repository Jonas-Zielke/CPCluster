#!/usr/bin/env bash
# Setup script for CPCluster development container
set -e

# Install system packages
sudo apt-get update
sudo apt-get install -y curl git build-essential pkg-config libssl-dev

# Install Rust if not already installed
if ! command -v cargo >/dev/null 2>&1; then
    echo "Rust not found. Installing via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
else
    echo "Rust is already installed"
fi

# Build master and node crates
for dir in CPCluster_masterNode CPCluster_node; do
    echo "Building $dir..."
    (cd "$dir" && cargo build --release)
    echo "Finished building $dir"
done

echo "Setup complete. Use 'cargo run' in each directory to start the master or node."
