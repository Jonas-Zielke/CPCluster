# cpcluster_node

`cpcluster_node` is a simple client used to join the CPCluster network. It reads the `join.json` file created by the master, connects over TCP and exchanges `NodeMessage` requests.

## Workflow

1. Parse `join.json` to obtain the authentication token and master address.
2. Connect to the master and send the token for authentication.
3. Request the list of currently connected nodes using `NodeMessage::GetConnectedNodes`.
4. Optionally send further requests, such as `RequestConnection` to another node.
5. Gracefully disconnect from the master with `NodeMessage::Disconnect`.

The node also uses `tokio` for asynchronous I/O and relies on `cpcluster_common` for shared message definitions.
