# ForgeIndex

**AST-driven codebase intelligence for agentic workflows.**

ForgeIndex is a local-first MCP server that provides structural code indexing, token compression, and dependency analysis to autonomous coding agents. It reduces per-session token usage by 90%+ compared to raw file reads.

## Features

- **AST Parsing** — Tree-sitter-based parsing for Python, TypeScript, JavaScript, Rust, Go, Java, C/C++, Ruby
- **Token Compression** — Skeleton views, TF-IDF ranking, and greedy knapsack packing reduce token payloads by 85–95%
- **Dependency Graph** — Import-based graph with PageRank scoring and blast radius analysis
- **MCP Server** — 14 tools exposed via JSON-RPC over stdio, compatible with Claude Desktop, Conductor, and any MCP client
- **Auto-Reindexing** — File watcher + git hooks keep the index fresh
- **SQLite Store** — WAL-mode database with xxh3 content hashing for JIT invalidation
- **Zero Config** — Works out of the box with sensible defaults

## Install

**One line:**
```bash
curl -fsSL https://raw.githubusercontent.com/chrismicah/forgeindex/main/install.sh | sh
```

**From source:**
```bash
cargo install --git https://github.com/chrismicah/forgeindex.git
```

**From a local clone (replaces the global binary in `~/.cargo/bin`):**
```bash
git clone https://github.com/chrismicah/forgeindex.git
cd forgeindex
cargo install --path . --force
```

If you already have ForgeIndex installed globally and want to replace it with the current
checkout, run:

```bash
cd /path/to/forgeindex
cargo install --path . --force
```

This updates the single `forgeindex` binary on your PATH, usually
`~/.cargo/bin/forgeindex`.

## Updating

```bash
# Pull latest and reinstall
cd /path/to/forgeindex
git pull
cargo install --path . --force

# Then rebuild your project index
cd /path/to/your/project
forgeindex reindex
```

See [Updating ForgeIndex](docs/UPDATING.md) for full details including when a clean rebuild is needed.

## Quick Start

```bash
# Initialize in a project directory
cd /path/to/your/project
forgeindex init

# Check index status
forgeindex status

# Search for symbols
forgeindex query "UserService"

# Show codebase map
forgeindex map

# Start the MCP server
forgeindex serve
```

## MCP Tools

| Tool | Description |
|------|-------------|
| `map_overview` | Codebase map with tiered detail: `tree` (~5K tokens), `summary` (~15K), or `full` (signatures) |
| `find_symbol` | Exact symbol lookup with signature and location |
| `read_source` | Source code of a symbol. `mode="skeleton"` for signatures only (~100 tokens), `mode="full"` with head+tail truncation for large symbols |
| `search_symbols` | Fuzzy search by name, signature, file path, or docstring. Short terms use prefix matching to avoid noise |
| `search_imports` | Search import/require/use statements across all files |
| `get_skeleton` | Skeletonized file view — signatures only |
| `get_dependencies` | Direct dependencies or dependents |
| `get_impact` | Transitive blast radius analysis with `max_depth` and `max_results` limits |
| `trace_data_flow` | Follow a symbol upstream (callers) and downstream (callees) across files |
| `get_ranked_symbols` | Top symbols by PageRank importance. Optional `path` filter to scope to a directory |
| `compress_context` | Optimal context within token budget (default 32K tokens) |
| `pack_repo` | Full repo packed into single artifact |
| `index_status` | Index health metrics |
| `reindex` | Force re-index |

## Claude MCP Integration

For Claude Code, add ForgeIndex to your user-level MCP config:

```bash
claude mcp add -s user forgeindex forgeindex serve
```

ForgeIndex uses the current working directory as the project root unless you pin a `cwd`
in Claude's MCP config. The safest workflow is:

```bash
cd /path/to/your/project
forgeindex init
claude
```

For Claude Desktop, add it to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "forgeindex": {
      "command": "forgeindex",
      "args": ["serve", "--root", "/path/to/your/project"]
    }
  }
}
```

## CLI Commands

```
forgeindex init        Initialize index in current directory
forgeindex serve       Start MCP server (stdio)
forgeindex status      Show index statistics
forgeindex reindex     Re-index all files (or a specific path)
forgeindex query       Search for symbols
forgeindex map         Show codebase overview map
forgeindex hooks       Install/uninstall git hooks
forgeindex config      Show/init configuration
```

## Configuration

Configuration is stored in `.forgeindex/config.toml`:

```toml
[index]
languages = ["python", "typescript", "tsx", "javascript", "rust", "go", "java", "c", "cpp", "ruby"]
exclude_patterns = ["**/node_modules/**", "**/dist/**", "**/*.min.js"]
include_tests = false
max_file_size_kb = 512

[compression]
default_token_budget = 32000
skeleton_collapse_threshold_lines = 3
aggregate_imports = true
strip_comments = true

[watcher]
enabled = true
debounce_ms = 200
respect_gitignore = true

[git_hooks]
auto_install = true
hook_types = ["post-commit", "post-checkout"]

[server]
transport = "stdio"
log_level = "warn"
```

## Architecture

```
┌─────────────────────────────────────────┐
│     MCP Client (Claude Desktop, etc.)   │
└──────────────────┬──────────────────────┘
                   │ JSON-RPC (stdio)
┌──────────────────┴──────────────────────┐
│            FORGEINDEX                    │
│                                          │
│  ┌──────────┐ ┌───────────┐ ┌────────┐  │
│  │ Parser   │ │Compressor │ │ Graph  │  │
│  │(TS grammars)│(TF-IDF)  │ │(PageRank)│ │
│  └──────────┘ └───────────┘ └────────┘  │
│  ┌──────────────────────────────────┐    │
│  │  File Watcher + Git Hooks       │    │
│  └──────────────────────────────────┘    │
│  ┌──────────────────────────────────┐    │
│  │  SQLite Store (WAL mode)        │    │
│  └──────────────────────────────────┘    │
└──────────────────────────────────────────┘
```

## Building

Requires Rust 1.70+ and a C compiler (for tree-sitter grammars).

```bash
cargo build --release
```

The binary will be at `target/release/forgeindex`.

## Documentation

- [Design Document](DESIGN.md) — full architecture and specification
- [MCP Tool Reference](docs/MCP_TOOLS.md) — complete tool docs for agent authors
- [Updating ForgeIndex](docs/UPDATING.md) — how to reinstall the global binary and what to do after pulling MCP updates

## License

MIT
