# Guidelines for Agents

This repository contains multiple Rust crates forming the CPCluster project. The workspace layout is described in `docs/PROJECT_OVERVIEW.md`.

## Conventions

- Format the code using `cargo fmt` before committing.
- Run `cargo clippy --all-targets` and `cargo test` from the repository root.
- Commit messages should be in English and briefly describe the change.

## Structure

- `CPCluster_masterNode` – master connection manager.
- `CPCluster_node` – node implementation.
- `cpcluster_common` – shared types and helpers.
- `cpcluster_client` – example client.
- `docs` – additional documentation.

When adding new files, place further AGENTS.md files in subdirectories if specific instructions are required.


