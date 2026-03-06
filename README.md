# ForgeIndex

**AST-driven codebase intelligence for agentic workflows.**

ForgeIndex is a local-first MCP server that provides structural code indexing, token compression, and dependency analysis to autonomous coding agents. It reduces per-session token usage by 90%+ compared to raw file reads.

## Features

- **AST Parsing** — Tree-sitter-based parsing for Python, TypeScript, JavaScript, Rust, Go, Java, C/C++, Ruby
- **Token Compression** — Skeleton views, TF-IDF ranking, and greedy knapsack packing reduce token payloads by 85–95%
- **Dependency Graph** — Import-based graph with PageRank scoring and blast radius analysis
- **MCP Server** — 12 tools exposed via JSON-RPC over stdio, compatible with Claude Desktop, Conductor, and any MCP client
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
| `map_overview` | Hierarchical tree of all public symbols |
| `find_symbol` | Exact symbol lookup with signature and location |
| `read_source` | Full source code of a specific symbol |
| `search_symbols` | Fuzzy symbol search ranked by relevance |
| `get_skeleton` | Skeletonized file view — signatures only |
| `get_dependencies` | Direct dependencies or dependents |
| `get_impact` | Transitive blast radius analysis |
| `get_ranked_symbols` | Top symbols by PageRank importance |
| `compress_context` | Optimal context within token budget |
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
default_token_budget = 4000
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

## License

MIT
