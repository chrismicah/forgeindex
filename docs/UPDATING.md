# Updating ForgeIndex

This document covers two common update paths:

1. Replacing the globally installed `forgeindex` binary in `~/.cargo/bin`
2. Pulling MCP changes into a local clone and refreshing any generated indexes

## How global reinstall works

If you have already installed ForgeIndex with Cargo, the command below rebuilds the
binary from your current checkout and replaces the copy on your PATH:

```bash
cd /path/to/forgeindex
cargo install --path . --force
```

What this does:

- Builds the current checkout in release mode
- Replaces the installed binary, usually at `~/.cargo/bin/forgeindex`
- Makes the updated binary available to any MCP client or shell session that resolves
  `forgeindex` from your PATH

Useful checks:

```bash
which forgeindex
forgeindex --help
```

If `which forgeindex` does not point at `~/.cargo/bin/forgeindex`, you may be using a
different installation source.

## If you only cloned the repo

If you only cloned ForgeIndex and have not installed it globally, what you need to do
depends on how you run it.

### Case 1: You run the installed global command

If you launch ForgeIndex as:

```bash
forgeindex serve
```

then pulling new commits into your clone is not enough. You also need to reinstall the
binary from that clone:

```bash
cd /path/to/forgeindex
git pull
cargo install --path . --force
```

### Case 2: You run directly from the clone

If you launch ForgeIndex as:

```bash
cargo run -- serve
```

or:

```bash
cargo run -- query UserService
```

then you do not need a global reinstall. Pull the latest commits and keep running from
that checkout:

```bash
cd /path/to/forgeindex
git pull
```

## After the MCP has been updated

Updating the ForgeIndex code and updating your per-project index are separate steps.

After pulling a new version of ForgeIndex, you should refresh the generated
`.forgeindex` index in any project you care about.

### Normal refresh

If the update was small and only changed query behavior, try:

```bash
cd /path/to/your/project
forgeindex reindex
```

### Full index rebuild

If the update changes parsing, symbol storage, reference extraction, graph building, or
SQLite schema, the safest approach is to rebuild the generated index from scratch:

```bash
cd /path/to/your/project
rm -f .forgeindex/index.db .forgeindex/index.db-shm .forgeindex/index.db-wal
forgeindex init
```

This is safe because `.forgeindex` is generated data, not source code.

Use a full rebuild when updates affect:

- symbol keys or canonical names
- reference extraction
- dependency graph logic
- index schema or migrations

## Recommended update flow

For someone using the global `forgeindex` binary:

```bash
cd /path/to/forgeindex
git pull
cargo install --path . --force

cd /path/to/your/project
rm -f .forgeindex/index.db .forgeindex/index.db-shm .forgeindex/index.db-wal
forgeindex init
```

For someone running from the clone without a global install:

```bash
cd /path/to/forgeindex
git pull

cd /path/to/your/project
rm -f .forgeindex/index.db .forgeindex/index.db-shm .forgeindex/index.db-wal
cargo run --manifest-path /path/to/forgeindex/Cargo.toml -- init
```
