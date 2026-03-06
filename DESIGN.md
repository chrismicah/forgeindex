# FORGEINDEX — AST-Driven Codebase Intelligence for Agentic Workflows

## Comprehensive Design Document

Version 1.0 • March 2026
Target Platform: macOS (Apple Silicon) + Conductor App

---

## 1. Executive Summary

ForgeIndex is a local-first, single-binary MCP server for macOS that provides AST-driven codebase intelligence to autonomous coding agents. It combines structural indexing via Tree-sitter, algorithmic token compression, dependency graph analysis, and automatic re-indexing into one tool that installs in a single command and integrates with the Conductor app out of the box.

The tool exists to solve a specific crisis: autonomous agents running inside Conductor (or similar orchestrators) burn through token budgets at catastrophic rates when they rely on grep, cat, and naive RAG pipelines to navigate codebases. Empirical data shows that structural indexing reduces per-session token usage from ~150,000 tokens down to 2,000–5,000 tokens — a reduction of 95% or more.

## 2. Problem Statement

### 2.1 The Token Exhaustion Crisis
When an autonomous agent inside Conductor issues a grep or cat command to understand a codebase, it ingests entire files — import blocks, boilerplate, comments, whitespace, and irrelevant implementations. A single 500-line file read consumes ~2,000 tokens. Over a 10-turn debugging session with multiple file reads, token consumption compounds to 100,000–200,000 tokens.

### 2.2 Why Existing Solutions Fail
- **grep/ripgrep**: Returns raw text matches with no structural awareness.
- **Cloud RAG/MCP**: Introduces latency, API costs, data privacy concerns.
- **Sourcegraph**: Enterprise-grade, heavyweight.
- **Individual AST MCP servers**: CortexAST, Srclight, RepoMapper each solve part of the problem but require separate installation and lack a unified compression pipeline.

### 2.3 What's Missing
A single, cohesive tool that: (a) parses code structurally via AST, (b) compresses token payloads algorithmically, (c) maps dependency graphs, (d) re-indexes automatically, (e) exposes everything via MCP, and (f) installs in one command on macOS.

## 3. Goals and Non-Goals

### 3.1 Goals
- Single-binary distribution for macOS (Apple Silicon aarch64)
- Sub-100ms response latency for all MCP tool calls on repos up to 500K LOC
- 90%+ token reduction vs. raw file reads
- Zero-configuration auto-reindexing via filesystem watching and git hook injection
- Native MCP server for Conductor, Claude Desktop, and any MCP-compatible client
- Multi-language: Python, TypeScript/JavaScript, Rust, Go, Java, C/C++, Swift, Ruby
- Local-only operation. Zero network calls. Zero data exfiltration.
- Dependency graph with impact analysis (blast radius computation)
- Configurable token budgets per query

### 3.2 Non-Goals
- NOT a code editor, IDE plugin, or language server
- NOT a code generator. Read-only intelligence.
- NOT cross-platform in v1. macOS Apple Silicon only (Linux/Windows future).

## 4. Architecture Overview

```
┌─────────────────────────────────────────────┐
│     Conductor / Claude Desktop / MCP Client │
└─────────────────────┬───────────────────────┘
                      │ MCP (stdio/SSE)
┌─────────────────────┴───────────────────────┐
│              FORGEINDEX BINARY              │
│                                             │
│  ┌───────────┐ ┌────────────┐ ┌─────────┐  │
│  │ AST Engine│ │ Compressor │ │ Dep Graph│  │
│  └───────────┘ └────────────┘ └─────────┘  │
│  ┌─────────────────────────────────────┐    │
│  │   File Watcher + Git Hooks          │    │
│  └─────────────────────────────────────┘    │
│  ┌─────────────────────────────────────┐    │
│  │   SQLite AST Store (mmap'd)         │    │
│  └─────────────────────────────────────┘    │
└─────────────────────────────────────────────┘
```

### Subsystem Responsibilities

**AST Engine (Tree-sitter)**: Parses source files into ASTs. Extracts symbols (functions, classes, methods, types, constants, interfaces), their signatures, visibility modifiers, docstrings, and byte ranges. Uses xxh3 content hashing for JIT invalidation.

**Compressor**: Implements nuclear skeletonization (collapse function bodies to signatures, aggregate imports, strip whitespace/comments). Implements TF-IDF relevance ranking with greedy knapsack for optimal token-budget allocation.

**Dependency Graph**: Builds in-memory directed graph of symbol references. Exposes PageRank-weighted importance scores and impact analysis (blast radius).

**File Watcher + Git Hooks**: FSEvents watcher for macOS. Git post-commit/post-checkout hooks for index reconciliation.

## 5. Core Modules

### 5.1 Module: parser
Key struct: `ParsedFile { path, lang, hash (xxh3), symbols: Vec<Symbol>, raw_bytes_range_map }`
Key struct: `Symbol { name, kind (Function|Class|Method|Type|Const|Interface), visibility (Public|Private|Internal), signature_text, docstring, byte_start, byte_end, children: Vec<Symbol> }`

### 5.2 Module: compressor
- `skeleton(file)`: Returns signatures only. Bodies replaced with `...`. Imports aggregated.
- `compress(files, query, token_budget)`: TF-IDF ranking + greedy knapsack.
- `pack(repo_root, token_budget)`: Repomix-style single-file XML representation.

### 5.3 Module: graph
- `build(parsed_files)`: Constructs directed adjacency list from import/reference analysis.
- `rank()`: Computes PageRank scores.
- `impact(symbol_name)`: Returns all transitive dependents.
- `related(symbol_name, depth)`: Returns symbols within N hops.

### 5.4 Module: watcher
- `watch(root_path)`: Starts FSEvents listener.
- `install_hooks(repo_path)`: Writes post-commit and post-checkout hooks.
- `uninstall_hooks(repo_path)`: Cleanly removes ForgeIndex hooks.

### 5.5 Module: store
SQLite-backed persistent store using WAL mode. Memory-mapped for near-zero I/O latency.

## 6. MCP Server Interface

### Tool Registry

| Tool Name | Parameters | Returns | Token Cost |
|-----------|-----------|---------|------------|
| map_overview | max_chars: int (default 8000) | Hierarchical text tree of all public symbols | ~2,000 |
| find_symbol | name: string, kind?: string | Exact symbol match with signature, file, line, docstring | ~50–200 |
| read_source | symbol: string | Full source code of a specific symbol only | ~100–1,000 |
| search_symbols | query: string, max_results: int, max_tokens: int | Fuzzy symbol search ranked by relevance | ~200–2,000 |
| get_skeleton | file_path: string | Skeletonized file: signatures only, bodies collapsed | ~100–500 |
| get_dependencies | symbol: string, direction: in\|out\|both | Direct dependencies or dependents | ~100–300 |
| get_impact | symbol: string | Full transitive blast radius | ~100–500 |
| get_ranked_symbols | top_n: int, kind?: string | Top N symbols by PageRank importance | ~200–800 |
| compress_context | query: string, token_budget: int | Optimized context payload within budget | = token_budget |
| pack_repo | token_budget: int, format: xml\|json | Full repo packed into single artifact | = token_budget |
| index_status | (none) | Index health: file count, symbol count, last update | ~50 |
| reindex | path?: string | Force re-index of specific file or entire repo | ~50 |

### MCP Protocol Details
- Transport: stdio (default), optional SSE (`--transport sse --port 3945`)
- Error codes: SYMBOL_NOT_FOUND, INDEX_STALE, BUDGET_EXCEEDED, PARSE_ERROR

## 7. Token Compression Pipeline

### Stage 1: Structural Pruning (AST Skeleton)
Preserves: function/method signatures with types, class/struct/interface declarations, module-level constants, single-line import summary.
Discards: function bodies, inline comments, docstrings beyond first line, whitespace, test functions.
**Measured reduction: 60–85% per file.**

### Stage 2: Relevance Ranking (TF-IDF Knapsack)
Tokenize query → TF-IDF score all symbols → greedy 0/1 knapsack → optimal set within token_budget.
**Additional 40–70% on top of Stage 1. Net: 85–95% total reduction.**

### Stage 3: Output Formatting
Clean pseudo-code with 2-space indent, no blank lines between symbols, file paths as section headers, dependency annotations as inline comments.

## 8. Auto-Reindexing System

1. **JIT Hash Validation**: xxh3 hash check on every tool call. Re-parse in <5ms if changed.
2. **FSEvents Background Watcher**: Kernel-level, no polling. Respects .gitignore and .forgeindexignore.
3. **Git Hook Integration**: SIGUSR1 to ForgeIndex process on commit/checkout.

## 9. Configuration Schema

```toml
# .forgeindex/config.toml
[index]
languages = ["python", "typescript", "rust", "go", "java", "swift"]
exclude_patterns = ["**/node_modules/**", "**/dist/**", "**/*.min.js"]
include_tests = false
max_file_size_kb = 512

[compression]
default_token_budget = 4000
skeleton_collapse_threshold_lines = 3
aggregate_imports = true
strip_comments = true
strip_docstrings_beyond_first_line = true

[watcher]
enabled = true
debounce_ms = 200
respect_gitignore = true

[git_hooks]
auto_install = true
hook_types = ["post-commit", "post-checkout"]

[server]
transport = "stdio"
sse_port = 3945
log_level = "warn"
log_file = ".forgeindex/forge.log"
```

## 10. Performance Targets

| Metric | Target |
|--------|--------|
| Initial full index (100K LOC) | < 10 seconds |
| Incremental re-index (single file) | < 5 ms |
| find_symbol latency | < 10 ms |
| map_overview latency | < 50 ms |
| compress_context latency | < 100 ms |
| get_impact latency | < 20 ms |
| Token reduction (skeleton) | > 60% per file |
| Token reduction (compress_context) | > 90% vs raw grep |
| Memory usage (100K LOC indexed) | < 200 MB RSS |
| Binary size | < 30 MB |

## 11. Technology Stack

| Component | Technology |
|-----------|-----------|
| Language | Rust |
| AST Parser | Tree-sitter (tree-sitter crate) |
| Content Hashing | xxh3 (xxhash-rust crate) |
| Storage | SQLite (rusqlite crate, WAL mode) |
| Async Runtime | Tokio |
| MCP Protocol | Custom JSON-RPC over stdio/SSE |
| File Watching | notify crate (FSEvents backend) |
| TF-IDF | Custom implementation |
| CLI Framework | clap crate |
| Serialization | serde + serde_json |
| Logging | tracing crate |

## 12. MCP Tool JSON Schemas

### find_symbol
```json
{
  "name": "find_symbol",
  "description": "Find a symbol by exact name. Returns signature, location, and docstring.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "name": { "type": "string", "description": "Exact symbol name to find" },
      "kind": { "type": "string", "enum": ["function","class","method","type","const","interface"], "description": "Optional: filter by symbol kind" }
    },
    "required": ["name"]
  }
}
```

### compress_context
```json
{
  "name": "compress_context",
  "description": "Return maximally relevant compressed code for a query within a strict token budget.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "Natural language or identifier query" },
      "token_budget": { "type": "integer", "default": 4000, "description": "Max tokens in response" }
    },
    "required": ["query"]
  }
}
```

### get_impact
```json
{
  "name": "get_impact",
  "description": "Compute the blast radius of changing a symbol. Returns all transitive dependents.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "symbol": { "type": "string", "description": "Symbol name to analyze" }
    },
    "required": ["symbol"]
  }
}
```

### get_skeleton
```json
{
  "name": "get_skeleton",
  "description": "Return a skeletonized view of a file: signatures only, bodies collapsed, imports aggregated.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file_path": { "type": "string", "description": "Relative path to the source file" }
    },
    "required": ["file_path"]
  }
}
```
