# cpcluster_masternode

This crate implements the CPCluster master connection manager. It listens for TCP connections (by default on `127.0.0.1:55000`), authenticates nodes via a token and coordinates direct connections between them.

On startup the master generates a `join.json` file containing the token, its IP address and port. Nodes must provide this token when connecting. Set `CPCLUSTER_JOIN` before starting the master to control where this file is written.

## Responsibilities

- Maintain a map of currently connected nodes along with their addresses.
- Allocate free ports in the range `55001-55999` so nodes can communicate directly.
- Handle `NodeMessage` requests: return the list of connected nodes, assign connection info, and clean up when nodes disconnect.

The server uses `tokio` for asynchronous networking and stores node information in `HashMap`/`HashSet` protected by `Mutex`.
