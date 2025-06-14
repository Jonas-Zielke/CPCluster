# cpcluster_masternode

This crate implements the CPCluster master connection manager. It listens for TCP connections (by default on `127.0.0.1:55000`), authenticates nodes via a token and coordinates direct connections between them.

On startup the master generates a `join.json` file containing the token, its IP address and port. Nodes must provide this token when connecting. Set `CPCLUSTER_JOIN` before starting the master to control where this file is written.

## Running

```bash
cargo run -- <config-path>
```

Replace `<config-path>` with the path to your configuration file if it is not `config.json`. The master listens on port **55000** and manages connection requests from nodes.

When starting, the master writes `join.json` with the authentication token. Restrict access to this file (for example `chmod 600`) and consider encrypting it before copying to nodes. You can also provide the token via the `CPCLUSTER_TOKEN` environment variable instead of copying the file.

### Master Shell

After startup the master opens an interactive shell. Useful commands include:

```
nodes             # list connected nodes
tasks             # list queued tasks
task <id>         # inspect a specific task
addtask <type> <args>  # queue a new task
```

## Responsibilities

- Maintain a map of currently connected nodes along with their addresses.
- Allocate free ports in the range `55001-55999` so nodes can communicate directly.
- Handle `NodeMessage` requests: return the list of connected nodes, assign connection info, and clean up when nodes disconnect.

The server uses `tokio` for asynchronous networking and stores node information in `HashMap`/`HashSet` protected by `Mutex`.
