# CPCluster - Compute Power Cluster - Distributed Node Network with Master Connection Manager

CPCluster is a distributed network of nodes that communicate with each other for task distribution. The master node in CPCluster serves as a connection manager, coordinating direct connections between nodes without routing their tasks through the master. This approach minimizes latency, improves task performance, and centralizes connection management.

## Features

- **Centralized Connection Management**: The master node manages node connections and assigns direct ports for inter-node communication.
- **Dynamic Port Assignment**: Nodes request connections to other nodes through the master, which assigns available ports in the range 55001-55999.
- **Direct Node-to-Node Communication**: Nodes establish direct communication channels after being connected, allowing efficient data transfer with minimal latency.
- **Token-based Authentication**: Nodes authenticate with the master using a unique token stored in a `join.json` file.
- **Disconnect Handling**: The master node manages disconnections and releases ports when nodes leave the network.

## Project Structure

- **Master Node** (`CPCluster_masterNode`): Acts as the connection manager, handles authentication, manages available ports, and facilitates direct connections between nodes.
- **Normal Node** (`CPCluster_node`): Connects to the master node, requests connections to other nodes, and handles direct communication for task exchange.

## Getting Started

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

   You can run `scripts/install.sh` from the repository root to install Rust
   (if missing) and build both projects automatically.

   For a full container setup including system packages you can also run
   `./setup_container.sh` from the repository root. This installs required
   packages, Rust and builds both crates.

2. **Navigate to each project**:
   - Master Node:
     ```bash
     cd CPCluster_masterNode
     ```
   - Normal Node:
     ```bash
     cd CPCluster_node
     ```

3. **Build each project**:
   ```bash
   cargo build
   ```

### Configuration

1. **Generate join.json**: When the master node is started, it creates a `join.json` file with a unique token for network access.
2. **Copy `join.json` to nodes**: Each node must have a `join.json` file identical to the one in the master node directory. Copy this file to the `CPCluster_node` directory for each node that will join the network.

### Running the Project

1. **Start the Master Node**:
   ```bash
   cd CPCluster_masterNode
   cargo run
   ```
   The master node listens on port **55000** and manages all connection requests from nodes.

2. **Start Normal Nodes**:
   For each node instance:
   ```bash
   cd CPCluster_node
   cargo run
   ```
   The node connects to the master, requests the list of currently connected nodes, and can initiate direct connections to other nodes.

### Example Workflow

1. **Node Authentication**: Each node connects to the master node and authenticates using the token in `join.json`.
2. **Requesting Connected Nodes**: After successful authentication, the node requests a list of currently connected nodes from the master.
3. **Requesting a Direct Connection**:
   - Node A requests to connect to Node B.
   - The master node assigns an available port, notifies both Node A and Node B of the connection details.
   - Node A and Node B then establish a direct connection on the assigned port.
4. **Handling Disconnection**:
   - If a node disconnects, the master releases the assigned port for future connections and notifies the other node if necessary.

## Code Structure

### Master Node (`CPCluster_masterNode/src/main.rs`)

- `generateToken()`: Generates a unique token for authentication.
- `handleConnection()`: Handles incoming node requests, validates tokens, and manages connection requests.
- `allocatePort()` and `releasePort()`: Manage port allocation and release for node connections.

### Normal Node (`CPCluster_node/src/main.rs`)

- `connectToMaster()`: Connects to the master and authenticates using the token.
- `requestConnectedNodes()`: Requests the list of currently connected nodes from the master.
- `requestConnectionToNode()`: Sends a request to the master to establish a direct connection with another node.
- `disconnect()`: Sends a disconnect request to the master to release resources and notify other nodes.

## Example `join.json`

```json
{
  "token": "your-unique-token-here",
  "ip": "127.0.0.1",
  "port": "55000"
}
```

Ensure each node uses the same `join.json` for authentication with the master node.

For additional information on the code layout and contribution hints see
[`docs/DEVELOPER_GUIDE.md`](docs/DEVELOPER_GUIDE.md).

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
