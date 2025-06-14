# CPCluster Roadmap

## Combined task: Node-to-node communication, job distribution and failover

This task combines the work for direct node connections, simple job execution and task failover into a single milestone.

1. **Direct Node Connections**
   - Extend `cpcluster_common::NodeMessage` with `DirectMessage`, `AssignTask`, and `TaskResult` variants.
   - After receiving `ConnectedNodes`, each node selects a peer and sends `RequestConnection` to the master.
   - On `ConnectionInfo` both nodes establish a TCP/TLS link and can exchange messages directly.

2. **Simple Job Execution**
   - Introduce a `Task` enum with variants like `Compute { expression }` and `HttpRequest { url }`.
   - Nodes execute received tasks using `meval` for computations or `reqwest` for HTTP GETs.
   - Results are returned with `TaskResult` over the direct connection or via the master as fallback.

3. **Task Failover**
   - The master tracks which node is working on which task.
   - If a node disconnects or times out, its active tasks go back into a queue.
   - Pending tasks are re-assigned to available nodes to ensure completion.

Implementing this combined task provides functional node-to-node messaging, initial distributed computation and basic robustness against node failures.

## Status

- [x] Direct node connections
- [x] Simple job execution
- [x] Task failover

All milestones above are implemented in the current code base. Further
enhancements will be added as new roadmap items.
