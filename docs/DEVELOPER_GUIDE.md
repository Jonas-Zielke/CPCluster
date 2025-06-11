# Developer Guide

This guide provides a brief overview of the project structure and some hints for getting familiar with the code base.

## Repository layout

- **CPCluster_masterNode** – source code of the master node connection manager.
- **CPCluster_node** – source code for a normal node that connects to the master.
- **README.md** – high level usage instructions.

Both projects are written in Rust and use `tokio` for asynchronous networking and `serde` for JSON serialization.

## Master node overview

The main logic is implemented in `CPCluster_masterNode/src/main.rs`.
Key components:

- `generate_token()` – creates a UUID used to authenticate nodes.
- `handle_connection()` – accepts a TCP connection from a node, validates the token and handles requests (`RequestConnection`, `GetConnectedNodes`, `Disconnect`).
- `allocate_port()` / `release_port()` – manage available ports for direct node communication.

The master stores connected nodes in a `HashMap` and available ports in a `HashSet`, both protected by `Mutex`.

## Node overview

`CPCluster_node/src/main.rs` shows how a node authenticates with the master and requests the list of connected nodes. It demonstrates a minimal workflow for joining the network and disconnecting.

- `send_message()` – helper to serialize a `NodeMessage` and send it via `TcpStream`.

## Contribution hints

1. Make sure [Rust](https://www.rust-lang.org/) is installed. Use the provided `scripts/install.sh` for a quick setup.
2. Build each crate individually using `cargo build` inside `CPCluster_masterNode` and `CPCluster_node`.
3. Run nodes separately in different terminals with `cargo run`.
4. The project currently uses TCP without TLS. Future improvements could include adding encrypted communication and more robust error handling.

## Additional documentation

Check `README.md` for instructions on running the master and node binaries. The `docs` directory can hold further notes as the project evolves.
