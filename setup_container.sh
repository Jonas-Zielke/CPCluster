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
elif command -v pacman >/dev/null 2>&1; then
    pkg_cmd="pacman"
    install_cmd="-Syu --noconfirm"
elif command -v brew >/dev/null 2>&1; then
    pkg_cmd="brew"
    install_cmd="install"
else
    echo "No supported package manager found. Please install curl, git, pkg-config, openssl development libraries and build tools manually." >&2
    pkg_cmd=""
fi

if [ -n "$pkg_cmd" ]; then
    if [ "$pkg_cmd" = "pacman" ]; then
        update_cmd="-Sy"
        deps="base-devel git curl pkgconf openssl"
    elif [ "$pkg_cmd" = "brew" ]; then
        update_cmd="update"
        deps="git curl pkg-config openssl@3"
    else
        update_cmd="update"
        deps="curl git build-essential pkg-config libssl-dev"
    fi

    if [ "$(id -u)" -ne 0 ] && [ "$pkg_cmd" != "brew" ]; then
        sudo $pkg_cmd $update_cmd
        sudo $pkg_cmd $install_cmd $deps
    else
        $pkg_cmd $update_cmd
        $pkg_cmd $install_cmd $deps
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

# Build all crates so they are ready for use
for dir in cpcluster_common cpcluster_client CPCluster_masterNode CPCluster_node; do
    echo "Building $dir..."
    (cd "$dir" && cargo build --release)
    echo "Finished building $dir"
done

echo "Setup complete. Use 'cargo run' in each directory to start the master or node."
