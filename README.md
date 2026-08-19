<p align="center">
  <img src="assets/logo.svg" width="140" alt="ForgeIndex logo">
</p>

<h1 align="center">ForgeIndex</h1>

<p align="center"><b>Give your coding agent a pre-built map of your repo.</b><br>
Symbol search, file skeletons, and blast-radius analysis over MCP — instant, offline, no language servers to babysit.</p>

<p align="center">
  <a href="https://github.com/chrismicah/forgeindex/actions/workflows/ci.yml"><img src="https://github.com/chrismicah/forgeindex/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <img src="https://img.shields.io/badge/price-free%20forever-success" alt="Free">
  <img src="https://img.shields.io/badge/rust-1.75%2B-orange" alt="Rust 1.75+">
</p>

## Quickstart

```bash
# 1. Install
curl -fsSL https://raw.githubusercontent.com/chrismicah/forgeindex/main/install.sh | sh

# 2. Hook it into Claude Code — once, works in every project forever
forgeindex register

# 3. That's it. Open any repo and ask your agent something.
```

There is no step where you configure anything. The first time an agent calls a
ForgeIndex tool in a project, the index builds itself (a 60k-line Swift app
indexes in ~0.4 s). Edits are picked up incrementally via content hashing and
git hooks.

Something off? `forgeindex doctor` diagnoses the usual suspects (registration,
PATH, index health) and tells you the fix.

## Why not just grep?

Agents that navigate by grep burn context: searching a common name in a big
SwiftUI app returns thousands of raw lines. ForgeIndex answers from a symbol
graph instead.

|  | grep / raw reads | embeddings + RAG | LSP bridge (serena etc.) | **ForgeIndex** |
|---|---|---|---|---|
| Setup | none | API keys, vector DB | per-language servers | one binary |
| Cold start | instant | minutes (embedding) | slow, can hang MCP | **instant (~0.4 s / 60 kLOC)** |
| Tokens per answer | huge | medium | small | **small (85–95 % less than raw reads)** |
| Offline / free | ✓ | ✗ | ✓ | ✓ |
| Ranking | none | similarity | none | **PageRank over the reference graph** |
| Crashes to babysit | none | service outages | language servers | **none** |

## What your agent gets

14 MCP tools. The ones that matter most:

| Tool | What it answers |
|------|-----------------|
| `map_overview` | "What is this codebase?" — tiered map, ~2K tokens for any repo |
| `search_symbols` | "Where is the login logic?" — fuzzy, ranked, multi-word |
| `find_symbol` / `read_source` | "Show me exactly this function" — body or skeleton, never the whole file |
| `get_skeleton` | "What's in this 8,000-line file?" — signatures only |
| `get_impact` | "What breaks if I change this?" — transitive blast radius |
| `trace_data_flow` | "Who calls this, and what does it call?" |
| `get_ranked_symbols` | "What are the load-bearing symbols here?" — PageRank |
| `compress_context` / `pack_repo` | Optimal context under a token budget |

Full reference: [docs/MCP_TOOLS.md](docs/MCP_TOOLS.md)

**Languages:** Python · TypeScript/TSX · JavaScript · Rust · Go · Java · C/C++ · Ruby · **Swift** (incl. SwiftUI: actors, extensions, property wrappers, computed vars)

## Install options

<details>
<summary><b>Homebrew</b></summary>

```bash
brew install --build-from-source ./Formula/forgeindex.rb
```
</details>

<details>
<summary><b>Cargo (from source)</b></summary>

```bash
cargo install --git https://github.com/chrismicah/forgeindex.git
# or from a local clone:
cargo install --path . --force
```
</details>

<details>
<summary><b>Prebuilt binaries</b></summary>

macOS (Apple Silicon + Intel), Linux (x86_64 + arm64), and Windows binaries are
attached to each [GitHub release](https://github.com/chrismicah/forgeindex/releases).
</details>

<details>
<summary><b>Updating</b></summary>

```bash
cd /path/to/forgeindex && git pull && cargo install --path . --force
forgeindex register   # re-point the MCP registration at the fresh binary
```

Project indexes migrate themselves. Details: [docs/UPDATING.md](docs/UPDATING.md)
</details>

## Add to other clients

`forgeindex register` covers Claude Code. For anything else, the one rule is:
**use the absolute binary path** — GUI-launched apps don't have your shell's
PATH, and a bare `"command": "forgeindex"` is the #1 cause of "server failed
to connect". Find yours with `which forgeindex`.

<details>
<summary><b>Claude Desktop</b> (<code>claude_desktop_config.json</code>)</summary>

```json
{
  "mcpServers": {
    "forgeindex": {
      "command": "/Users/you/.cargo/bin/forgeindex",
      "args": ["serve", "--root", "/path/to/your/project"]
    }
  }
}
```
Fully quit and reopen Claude Desktop after editing.
</details>

<details>
<summary><b>Cursor / Windsurf / any MCP client</b> (<code>.mcp.json</code> style)</summary>

```json
{
  "mcpServers": {
    "forgeindex": {
      "type": "stdio",
      "command": "/Users/you/.cargo/bin/forgeindex",
      "args": ["serve"]
    }
  }
}
```
The server uses its working directory as the project root; MCP clients launch
it in the workspace folder.
</details>

## Troubleshooting

- **"Failed to connect" in Claude Code** → `forgeindex register`, restart the
  session. (`claude mcp list` shows connection status.)
- **Tools error or return nothing in a fresh checkout/worktree** → they
  shouldn't: the index auto-builds on first call. If it's empty, run
  `forgeindex status` in that directory — it reports exactly what was skipped
  and why (including files over the size cap, listed by name).
- **Anything else** → `forgeindex doctor`, then
  `tail -F ~/Library/Logs/Claude/mcp*.log` while reproducing.

## CLI

```
forgeindex register    Register with Claude Code (absolute path, user scope)
forgeindex doctor      Diagnose registration / PATH / index problems
forgeindex init        Build the index for the current directory
forgeindex status      Index stats + skipped-file warnings
forgeindex query "x"   Search symbols from the terminal
forgeindex map         Codebase overview map
forgeindex reindex     Force re-index (all files or one path)
forgeindex hooks       Install/uninstall git auto-reindex hooks
forgeindex config      Show/init configuration
```

## Configuration

`.forgeindex/config.toml` — created on first index, safe to leave untouched:

```toml
[index]
languages = ["python", "typescript", "tsx", "javascript", "rust", "go", "java", "c", "cpp", "ruby", "swift"]
exclude_patterns = ["**/node_modules/**", "**/dist/**", "**/*.min.js"]
include_tests = false
max_file_size_kb = 2048   # files above this are skipped — and loudly reported

[compression]
default_token_budget = 32000

[git_hooks]
auto_install = true
hook_types = ["post-commit", "post-checkout"]
```

## How it works

Tree-sitter parses every source file into an AST → symbols, imports, and
references land in a WAL-mode SQLite DB (`.forgeindex/index.db`) with xxh3
content hashing for incremental updates → an import/reference graph with
PageRank scoring powers ranking and impact analysis → 14 MCP tools serve it
all over stdio JSON-RPC. Local-first; nothing leaves your machine.

## Documentation

- [Design Document](DESIGN.md) · [MCP Tool Reference](docs/MCP_TOOLS.md) · [Roadmap](docs/ROADMAP.md) · [Updating](docs/UPDATING.md) · [Changelog](CHANGELOG.md)

## License

MIT — free forever.
