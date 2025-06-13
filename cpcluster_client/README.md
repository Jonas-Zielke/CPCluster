# cpcluster_client

This crate provides a simple example client that submits compute tasks to the
master node and waits for the result. It demonstrates how compute tasks can
borrow string slices using `Cow`.

## Running

1. Copy the `join.json` file created by the master node into this directory.
2. Build the crate:

   ```bash
   cargo build --release
   ```

3. Run the client:

   ```bash
   cargo run --release
   ```

The client reads `join.json`, connects to the master and submits a `Compute`
task with the expression `1+2`. After receiving the result it submits another
task multiplying the number by three. Example output looks like:

```
first result: Number(3.0)
chained result: Number(9.0)
```

The program exits once the second task completes.
