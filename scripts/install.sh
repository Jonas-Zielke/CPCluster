#!/bin/bash
# Basic setup script for CPCluster
set -e

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
    (cd "$dir" && cargo build)
    echo "Finished building $dir"
done

echo "Installation complete. Use 'cargo run' inside each directory to start the master or node." 
