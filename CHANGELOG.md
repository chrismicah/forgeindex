# Changelog

All notable changes to ForgeIndex are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-06-10

Initial release.

### Added
- Tree-sitter AST parsing for Python, TypeScript, JavaScript, Rust, Go, Java, C/C++, and Ruby
- MCP server (JSON-RPC over stdio) with 14 tools: `map_overview`, `find_symbol`, `read_source`, `search_symbols`, `search_imports`, `get_skeleton`, `get_dependencies`, `get_impact`, `trace_data_flow`, `get_ranked_symbols`, `compress_context`, `pack_repo`, `index_status`, `reindex`
- Token compression: skeleton views, TF-IDF ranking, greedy knapsack packing (85–95% reduction)
- Import-based dependency graph with PageRank scoring and blast-radius analysis
- SQLite store (WAL mode) with xxh3 content hashing for JIT invalidation
- File watcher and git hooks for automatic re-indexing
- CLI: `init`, `serve`, `status`, `reindex`, `query`, `map`, `hooks`, `config`
- One-line installer (`install.sh`) with prebuilt-binary download and source-build fallback
- Homebrew formula

### Security
- Parameterized all SQL queries (no string interpolation of user input)
- MCP file-read tools reject paths that escape the project root
- Stale-index byte ranges are detected and reported instead of slicing blindly
