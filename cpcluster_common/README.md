# cpcluster_common

`cpcluster_common` provides shared data types used by the CPCluster master node and normal nodes. It defines the structures for authentication and the messages exchanged between peers.

## Provided Types

- `JoinInfo` – contains the authentication token and address information published by the master in `join.json`.
- `NodeMessage` – enum describing messages exchanged over TCP:
  - `RequestConnection(String)` – ask the master to connect to another node.
  - `ConnectionInfo(String, u16)` – master response giving target IP and port.
  - `GetConnectedNodes` and `ConnectedNodes(Vec<String>)` – request and response for the list of nodes currently in the cluster.
  - `Disconnect` – tells the master a node is leaving.

These types are `serde` serializable and are used by both the `cpcluster_masternode` and `cpcluster_node` crates.
