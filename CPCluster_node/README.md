# cpcluster_node

`cpcluster_node` is a simple client used to join the CPCluster network. It reads the `join.json` file created by the master (written to `CPCluster_masterNode/join.json` by default). Use `CPCLUSTER_JOIN` to set a custom path. The token can also be supplied via the `CPCLUSTER_TOKEN` environment variable.

## Configuration

The node loads runtime options from `config/config.json` in the crate directory. Pass a different file path as the first command line argument to `cargo run` to override it.

Key fields include:

- **`role`** – choose `Worker` (default), `Disk` or `Internet`.
- **`storage_dir`** – directory used for on-disk tasks or RAM-disk storage.
- **`internet_ports`** – list of ports Internet nodes open for external connections.

`join.json` is expected in the `CPCluster_node` directory by default but can be overridden with the `CPCLUSTER_JOIN` environment variable. Likewise the connection token may be supplied via `CPCLUSTER_TOKEN`.

## Workflow

1. Parse `join.json` to obtain the authentication token and master address. Set the `CPCLUSTER_TOKEN` environment variable to override the token at runtime. Use `CPCLUSTER_JOIN` if the file is stored elsewhere.
2. Connect to the master and send the token for authentication.
3. Request the list of currently connected nodes using `NodeMessage::GetConnectedNodes`.
4. Optionally send further requests, such as `RequestConnection` to another node.
5. Gracefully disconnect from the master with `NodeMessage::Disconnect`.

The node also uses `tokio` for asynchronous I/O and relies on `cpcluster_common` for shared message definitions.

## Heartbeat and Failover

Nodes send a `Heartbeat` message to the master every `failover_timeout_ms` milliseconds. If the master does not receive a heartbeat within twice this period it removes the node and re-queues its tasks. Nodes attempt to reconnect using all addresses in `master_addresses` until one responds.

## Task Examples

Nodes can execute compute expressions or HTTP requests. For example:

```rust
use cpcluster_common::Task;

let compute = Task::Compute { expression: "1 + 2".into() };
let request = Task::HttpRequest { url: "https://example.com".into() };
```

`TaskResult::Number(3.0)` or `TaskResult::Response` will be returned respectively.

## In-memory Storage

The node exposes a simple volatile key/value store implemented in
`memory_store.rs`. `MemoryStore` is created in `main` and shared with all task
handlers. Tasks can store and load binary blobs using `Task::StoreData` and
`Task::RetrieveData` which internally call `MemoryStore::store` and
`MemoryStore::load`.

```rust
use cpcluster_node::memory_store::MemoryStore;

let store = MemoryStore::new();
store.store("example".into(), b"data".to_vec()).await;
let data = store.load("example").await;
```

### Shared-memory storage

When `storage_dir` points to a tmpfs or RAM-disk (for example `/dev/shm/cpcluster`)
and the node role is `Worker`, tasks can use `DiskWrite` and `DiskRead` as a
lightweight shared-memory channel between processes. Disk nodes apply the same
mechanism but persist the files on disk while respecting `disk_space_mb`.

The node also exposes `GetGlobalRam` and `GetStorage` tasks. `GetGlobalRam` lists all in-memory keys with their sizes while `GetStorage` reports disk usage when a disk store is configured.
