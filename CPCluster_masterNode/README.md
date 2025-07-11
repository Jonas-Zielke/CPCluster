# cpcluster_masternode

This crate implements the CPCluster master connection manager. It listens for TCP connections (by default on `127.0.0.1:55000`), authenticates nodes via a token and coordinates direct connections between them.

On startup the master generates a `join.json` file in the `CPCluster_masterNode` directory containing the token, its IP address and port. Nodes must provide this token when connecting. Set `CPCLUSTER_JOIN` before starting the master to control where this file is written.

## Running

```bash
cargo run -- <config-path>
```

Replace `<config-path>` with the path to your configuration file if it is not `config/config.json`. The master listens on port **55000** and manages connection requests from nodes.
If `peer_masters` is set in the configuration, the master will connect to those addresses and exchange pending tasks and node states.

When starting, the master writes `join.json` with the authentication token to `CPCluster_masterNode/join.json`. Restrict access to this file (for example `chmod 600`) and consider encrypting it before copying to nodes. You can also provide the token via the `CPCLUSTER_TOKEN` environment variable instead of copying the file.

### Master Shell

After startup the master opens an interactive shell. Useful commands include:

```
nodes             # list connected nodes with their role
tasks             # list queued tasks
task <id>         # inspect a specific task
addtask <type> <args>  # queue a new task
```

### Example: Multiple Masters

Two masters configured with each other in `peer_masters` will automatically sync state:

```json
{
  "peer_masters": ["192.168.1.2:55000", "192.168.1.3:55000"]
}
```

## Responsibilities

- Maintain a map of currently connected nodes along with their addresses.
- Allocate free ports in the range `55001-55999` so nodes can communicate directly.
- Handle `NodeMessage` requests: return the list of connected nodes, assign connection info, and clean up when nodes disconnect.

The server uses `tokio` for asynchronous networking and stores node information in `HashMap`/`HashSet` protected by `Mutex`.
