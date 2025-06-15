# CPCluster - Compute Power Cluster - Distributed Node Network with Master Connection Manager

CPCluster is a distributed network of nodes that communicate with each other for task distribution. The master node in CPCluster serves as a connection manager, coordinating direct connections between nodes without routing their tasks through the master. This approach minimizes latency, improves task performance, and centralizes connection management.

For an overview of the repository structure see `docs/PROJECT_OVERVIEW.md`.
The development roadmap lives in `docs/ROADMAP.md` and changes are tracked in
`docs/CHANGELOG.md`.

## Features

- **Centralized Connection Management**: The master node manages node connections and assigns direct ports for inter-node communication.
- **Dynamic Port Assignment**: Nodes request connections to other nodes through the master, which assigns available ports in the configurable range defined in the configuration file (`CPCluster_masterNode/config/config.json` by default).
- **Direct Node-to-Node Communication**: Nodes establish direct communication channels after being connected, allowing efficient data transfer with minimal latency.
- **Token-based Authentication**: Nodes authenticate with the master using a unique token stored in a `join.json` file.
 - **Disconnect Handling**: The master node manages disconnections and releases ports when nodes leave the network.
- **Optional TLS Encryption**: When nodes communicate across the internet, connections to the master node are automatically upgraded to TLS using `tokio-rustls`.
- **Redundant Masters**: Clients can specify multiple master addresses and automatically fail over if one becomes unavailable.
- **Heartbeat Monitoring**: Nodes periodically send heartbeats and the master removes entries if a node stops responding.
- **Node Roles**: Nodes can run as `Worker` (default), `Disk` or `Internet`. Disk nodes persist data in `storage_dir` up to `disk_space_mb`. Internet nodes expose ports from `internet_ports` for network tasks and always use TLS.

## Project Structure

- **Master Node** (`CPCluster_masterNode`): Acts as the connection manager, handles authentication, manages available ports, and facilitates direct connections between nodes.
- **Normal Node** (`CPCluster_node`): Connects to the master node, requests connections to other nodes, and handles direct communication for task exchange.

## Getting Started

### Supported Environments

The project is primarily developed on Linux and macOS. The provided
`setup_container.sh` script detects common package managers (`apt`, `dnf`, `yum`,
`pacman` and `brew`) to install prerequisites automatically. On other platforms
or when detection fails, install `curl`, `git`, a C toolchain, `pkg-config` and
OpenSSL development libraries manually before building.

### Prerequisites

- [Rust](https://www.rust-lang.org/) programming language
- Internet connection for fetching dependencies
- Ports **55000** (for the master node) and **55001-55999** (for direct inter-node connections) open on your network

### Installation

1. **Clone the repository**:
   ```bash
   git clone https://github.com/CPCluster/CPCluster.git
   cd CPCluster
   ```

   You can run `./setup_container.sh` from the repository root to install Rust
   and all required packages automatically. The script detects common package
   managers (`apt`, `dnf`, `yum`, `pacman`, `brew`), installs dependencies and
   builds the crates. If your system uses a different manager, install `curl`,
   `git`, a C toolchain, `pkg-config` and OpenSSL development libraries manually
   before running the script.

2. **Build all crates**:
   ```bash
   cargo build --workspace
   ```

3. *(Optional)* **Build individual projects**:
   - Master Node:
     ```bash
     cd CPCluster_masterNode
     cargo build
     ```
   - Normal Node:
     ```bash
     cd CPCluster_node
     cargo build
     ```

4. **Run tests**:
   ```bash
   cargo test --workspace
   ```

### Configuration

1. **Generate join.json**: When the master node is started, it creates a `join.json` file with a unique token for network access.
   Set the `CPCLUSTER_JOIN` environment variable before starting the master to write the file to a different location.
2. **Securely distribute `join.json`**: Restrict access and encrypt the file before transferring it. One approach is:
   ```bash
   chmod 600 join.json
   gpg --recipient alice@example.com --encrypt join.json
   scp join.json.gpg node:/tmp/join.json.gpg
   ssh node 'gpg --decrypt /tmp/join.json.gpg > ~/CPCluster_node/join.json && chmod 600 ~/CPCluster_node/join.json'
   ```
   Instead of copying the file, export the token on each node via the `CPCLUSTER_TOKEN` environment variable:
   ```bash
   export CPCLUSTER_TOKEN=<token-from-master>
   ```
3. **Copy `join.json` to nodes**: If not using the environment variable, each node must have a `join.json` file identical to the one in the master node directory. Copy this file to the `CPCluster_node` directory for each node that will join the network. Nodes read from `join.json` by default, or use `CPCLUSTER_JOIN` to specify a different path.
4. **Edit the configuration file**: Both master and nodes read runtime options from `CPCluster_masterNode/config/config.json` and `CPCluster_node/config/config.json` by default. You can pass a different file as the first command line argument. The configuration lets you tune the port range, failover timeout, master addresses and TLS certificates. Additional fields include `role` (`Worker`, `Disk`, `Internet`), `storage_dir`, `disk_space_mb`, `internet_ports` and `state_file`.
5. **Generate TLS certificates (optional)**: To secure traffic between nodes and the master, create a certificate for the master node and distribute it to all nodes:
   ```bash
   openssl req -x509 -newkey rsa:4096 -nodes -keyout master_key.pem \
       -out master_cert.pem -days 365 -subj "/CN=<master-ip>"
   ```
   Set `cert_path` and `key_path` in the configuration file to these files and copy
   `master_cert.pem` to each node, configuring the path in `ca_cert_path`.

### Running the Project

1. **Start the Master Node**:
   ```bash
    cd CPCluster_masterNode
    cargo run -- <config-path> [--log-level <level>]
    ```
    Replace `<config-path>` with the path to your configuration file if it is not `CPCluster_masterNode/config/config.json`. The master node listens on port **55000** and manages all connection requests from nodes.
   The `--log-level` flag controls verbosity (`error`, `warn`, `info`, `debug`, `trace`).

2. **Start Normal Nodes**:
   For each node instance:
   ```bash
   cd CPCluster_node
   cargo run -- <config-path> [--log-level <level>]
   ```
   Use `<config-path>` to specify a custom configuration file if needed. The node connects to the master, requests the list of currently connected nodes, and can initiate direct connections to other nodes.
   The `--log-level` flag controls verbosity in the same way as for the master.

### Example Workflow

1. **Node Authentication**: Each node connects to the master node and authenticates using the token in `join.json`.
2. **Requesting Connected Nodes**: After successful authentication, the node requests a list of currently connected nodes from the master.
3. **Requesting a Direct Connection**:
   - Node A requests to connect to Node B.
   - The master node assigns an available port, notifies both Node A and Node B of the connection details.
   - Node A and Node B then establish a direct connection on the assigned port.
   - Once connected they can exchange tasks. For example Node A can send `AssignTask` with `Compute { expression: "1+2" }` and Node B replies with `TaskResult::Number(3.0)`.
   - Another example is `HttpRequest { url: "https://example.com" }` which lets a node fetch the page body and return it as `TaskResult::Response`.
   - Additional task types include `Tcp` and `Udp` for raw socket communication,
     `ComplexMath` for complex numbers, in-memory `StoreData`/`RetrieveData`,
     `DiskWrite`/`DiskRead` for persistent storage, `GetGlobalRam` to list memory
     usage and `GetStorage` for disk statistics.
4. **Handling Disconnection**:
   - If a node disconnects, the master releases the assigned port for future connections and notifies the other node if necessary.

### Master Shell

When the master node starts it opens an interactive shell. Besides `nodes` and
`tasks` you can check the status of a specific job with `task <id>` or queue new
work with `addtask`:

```bash
addtask compute 1+2
addtask http https://example.com
```

## Code Structure

### Master Node (`CPCluster_masterNode/src/main.rs`)

- `generate_token()`: Generates a unique token for authentication.
- `handle_connection()`: Handles incoming node requests, validates tokens, and manages connection requests.
- `allocate_port()` and `release_port()`: Manage port allocation and release for node connections.

### Normal Node (`CPCluster_node/src/main.rs`)

- `connect_to_master()`: Connects to the master and authenticates using the token.
- `request_connected_nodes()`: Requests the list of currently connected nodes from the master.
- `request_connection_to_node()`: Sends a request to the master to establish a direct connection with another node.
- `disconnect()`: Sends a disconnect request to the master to release resources and notify other nodes.

## Example `join.json`

```json
{
  "token": "your-unique-token-here",
  "ip": "127.0.0.1",
  "port": 55000
}
```

Ensure each node uses the same `join.json` for authentication with the master node.

### Node Configuration Examples

Disk node reserving 2 GB for persistent tasks:

```json
{
  "role": "Disk",
  "storage_dir": "/var/cpcluster",
  "disk_space_mb": 2048
}
```

Worker node using a RAM disk for shared memory between tasks:

```json
{
  "role": "Worker",
  "storage_dir": "/dev/shm/cpcluster"
}
```

Internet node opening specific ports for network tasks:

```json
{
  "role": "Internet",
  "internet_ports": [8080, 8443]
}
```

For additional information on the code layout and contribution hints see
[`docs/DEVELOPER_GUIDE.md`](docs/DEVELOPER_GUIDE.md).

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
