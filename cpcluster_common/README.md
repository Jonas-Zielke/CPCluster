# cpcluster_common

`cpcluster_common` provides shared data types used by the CPCluster master node and normal nodes. It defines the structures for authentication and the messages exchanged between peers.

## Provided Types

- `JoinInfo` – contains the authentication token and address information (IP and numerical port) published by the master in `join.json`.
- `NodeMessage` – enum describing messages exchanged over TCP:
  - `RequestConnection(String)` – ask the master to connect to another node.
  - `ConnectionInfo(String, u16)` – master response giving target IP and port.
  - `GetConnectedNodes` and `ConnectedNodes(Vec<String>)` – request and response for the list of nodes currently in the cluster.
  - `SubmitTask { id, task }` – queue a new task on the master node.
  - `GetTaskResult(id)` – fetch the result of a previously submitted task.
  - `Disconnect` – tells the master a node is leaving.
  - `Heartbeat` – periodic keep-alive message sent by the nodes.
  - `AssignTask { id, task }` – instructs a peer to execute a `Task`.
  - `TaskResult { id, result }` – result of a task execution.
  - `TaskAccepted(id)` – acknowledgement that a task was queued.
  - `DirectMessage(String)` – simple string message between peers.

- `Task` – represents work sent between nodes:
  - `Compute { expression }` – evaluate a mathematical expression. The
    expression uses `Cow<'static, str>` so it can borrow a string slice or own
    the data.
  - `HttpRequest { url }` – perform a HTTP GET request.

- `TaskResult` – returned from task execution:
  - `Number(f64)` – result of a computation.
  - `Response(String)` – body of an HTTP request.
  - `Error(String)` – task failed with this error.

These types are `serde` serializable and are used by both the `cpcluster_masternode` and `cpcluster_node` crates.

## Configuration

`Config` offers runtime configuration such as port ranges, failover timeout and the list of master nodes. A default configuration is returned when no `config.json` file is present.

## Helper Functions

- `is_local_ip(&str)` – returns `true` if the provided IP address is part of a private
  network. This allows the nodes to decide whether to enable TLS.
