# Project Overview

CPCluster is a collection of Rust crates implementing a simple distributed network. Nodes connect to a master for connection management and exchange tasks directly. The repository is organized as a Cargo workspace containing the following crates:

- **cpcluster_masternode** – master connection manager responsible for authenticating nodes and assigning ports.
- **cpcluster_node** – client node that connects to the master and communicates with peers.
- **cpcluster_common** – shared data types like `NodeMessage`, `Task` and `Config`.
- **cpcluster_client** – example client showing how to submit tasks through the master.

Additional documentation is located in `docs/DEVELOPER_GUIDE.md` and `docs/ROADMAP.md`. A sample configuration lives in `config.json`.

Use `setup_container.sh` to install Rust and build all crates on new systems.

