#!/usr/bin/env bash
# Setup script for CPCluster development container
set -e

# Install system packages
# Detect package manager and install system packages if possible
if command -v apt-get >/dev/null 2>&1; then
    pkg_cmd="apt-get"
    install_cmd="install -y"
elif command -v dnf >/dev/null 2>&1; then
    pkg_cmd="dnf"
    install_cmd="install -y"
elif command -v yum >/dev/null 2>&1; then
    pkg_cmd="yum"
    install_cmd="install -y"
else
    echo "No supported package manager found. Please install curl, git, build-essential, pkg-config and libssl-dev manually." >&2
    pkg_cmd=""
fi

if [ -n "$pkg_cmd" ]; then
    if [ "$(id -u)" -ne 0 ]; then
        sudo $pkg_cmd update
        sudo $pkg_cmd $install_cmd curl git build-essential pkg-config libssl-dev
    else
        $pkg_cmd update
        $pkg_cmd $install_cmd curl git build-essential pkg-config libssl-dev
    fi
fi

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
