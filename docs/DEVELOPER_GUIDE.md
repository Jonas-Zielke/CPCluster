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

The master stores connected nodes together with the timestamp of their last heartbeat. A background task periodically cleans up nodes that stop sending heartbeats. Available ports are tracked in a `HashSet`, both collections protected by `Mutex`.

`generate_tls_config()` in the master builds a self-signed certificate used when clients connect from outside a local network. Nodes rely on `is_local_ip()` from `cpcluster_common` to decide whether TLS should be enabled. Alternatively you can supply your own certificate using `cert_path` and `key_path` in the configuration file and distribute the public certificate to nodes via `ca_cert_path`.

Nodes periodically send heartbeats. If one fails, the failover timeout defined in the configuration determines when the master removes it. Nodes try each configured master address when reconnecting so another master can take over.

## Node overview

`CPCluster_node/src/main.rs` shows how a node authenticates with the master and requests the list of connected nodes. After connecting it periodically sends heartbeat messages until the connection is closed.

- `send_message()` – helper to serialize a `NodeMessage` and send it via `TcpStream`.

### Node roles and configuration

Nodes can operate in different roles defined in the configuration file (default `config.json`):

- `Worker` – default mode executing tasks purely in memory.
- `Disk` – persists data in `storage_dir` with a `disk_space_mb` quota.
- `Internet` – reachable from public networks, uses TLS and opens ports listed in `internet_ports`.

Important configuration fields include:

- `storage_dir` – directory used for disk tasks or shared RAM disks.
- `disk_space_mb` – quota for disk nodes.
- `role` – choose `Worker`, `Disk` or `Internet`.
- `failover_timeout_ms` and `master_addresses` – reconnection behaviour.
- `max_retries` – limit for connection attempts when (re)joining the master.
- `internet_ports` – list of ports bound by Internet nodes.
- `state_file` – path where the master node persists its state.

`cpcluster_common::Task` includes variants such as `Tcp`, `Udp`, `ComplexMath`, `StoreData`, `RetrieveData`, `DiskWrite`, `DiskRead`, `GetGlobalRam` and `GetStorage` in addition to compute and HTTP requests.

### Secure token distribution

The master writes a `join.json` file containing the authentication token. Because any node holding this token can join the cluster, distribute it carefully:

- Restrict read permissions so only the intended user can access the file.
- Copy it over an encrypted channel or encrypt it before transfer (e.g. with `gpg`).
- Alternatively set the token via the `CPCLUSTER_TOKEN` environment variable or use a secrets manager, keeping only the IP and port in `join.json`.
- Set `CPCLUSTER_JOIN` on both master and nodes to change where `join.json` is written and read.
## Contribution hints


1. Make sure [Rust](https://www.rust-lang.org/) is installed. You can run `./setup_container.sh` from the repository root to install all dependencies and build the project.
2. Install the `rustfmt` and `clippy` components with:
   ```bash
   rustup component add rustfmt clippy
   ```
3. Before committing, format the code and run lints:
   ```bash
   cargo fmt
   cargo clippy
   ```
4. Build the entire workspace once using `cargo build --workspace`.
5. Run nodes separately in different terminals with `cargo run`.
6. Runtime options such as port ranges or master addresses are loaded from a configuration file via the `Config` helper in `cpcluster_common`. The path defaults to `config.json` but can be supplied as the first command line argument when running the binaries.

## Additional documentation

Check `README.md` for instructions on running the master and node binaries. The `docs` directory can hold further notes as the project evolves.
