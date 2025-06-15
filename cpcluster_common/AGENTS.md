# cpcluster_common

Shared types used across the CPCluster crates. The `Config` helper reads the config file from each crate's `config` directory and defaults when the file is missing.

- Keep this crate lightweight as it is depended on by all others.
- Any change here requires `cargo test` from the repository root.

