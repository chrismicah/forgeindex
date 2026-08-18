# Changelog

All notable changes to ForgeIndex are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- Swift language support: AST parsing (classes, structs, enums, actors, protocols,
  extensions, functions, initializers, deinitializers, subscripts, operators,
  properties, typealiases, associatedtypes), import resolution, and call-reference
  extraction via `tree-sitter-swift`. Existing `.forgeindex/config.toml` files are
  migrated automatically to enable Swift.
- `forgeindex register` — one-shot registration with Claude Code using the
  binary's absolute path (immune to GUI-app PATH issues).
- `forgeindex doctor` — diagnoses registration, PATH, and index-health problems.
- Auto-indexing: the MCP server now builds the index on the first tool call in a
  project with no index (fresh checkouts and git worktrees just work); `serve`
  no longer exits when no index exists.
- New `property` symbol kind: Swift `var` (including computed vars and wrapped
  SwiftUI state) is distinguished from `let` constants.
- Oversized-file visibility: files skipped for exceeding `max_file_size_kb` are
  now listed by name in `forgeindex status`, the `index_status` MCP tool, and
  index-time warnings — never silently invisible.
- Stale-file pruning: deleted files are removed from the index on reindex.
- Next-step hints in `search_symbols` responses guide agents to the right
  follow-up tool.

### Fixed
- **Parse-error recovery**: one construct the grammar doesn't understand no
  longer erases every symbol after it in the file — declarations are now
  recovered from inside ERROR subtrees (a 110 KB real-world SwiftUI file went
  from 123 to 211 extracted symbols).
- **Incremental reindexing no longer destroys cross-file graph edges.**
  Re-indexing a file used to cascade-delete every edge targeting its symbols;
  impact analysis and ranking silently degraded with every edit. Inbound edges
  are now rebuilt from persisted references in the same transaction.
- MCP `reindex` with a path now resolves against the project root (was: the
  server's working directory) and is sandboxed to the project.
- Single-file indexing now honors config filters (language list, exclude
  patterns, size cap, test-file filter).
- Git hooks are now installed into the hooks directory git actually reads in
  linked worktrees (`$GIT_COMMON_DIR/hooks` via `commondir`), and embed the
  binary's absolute path so they work from GUI git clients.
- SQLite busy timeout (5 s) prevents "database is locked" errors when the
  git-hook reindex races the MCP server.
- Swift: `deinit` declarations are captured; all extension members (nested
  types, computed vars, static funcs) keep their `Type.` prefix, not just
  methods; `.swift` files participate in import-path edge resolution; Swift
  test files (`*Tests.swift`, `*Test.swift`) respect `include_tests = false`.
- `find_symbol` caps responses at 25 matches with guidance for narrowing;
  `read_source` reports ambiguity when several symbols share a name instead of
  silently picking the first.
- Removed the phantom `max_tokens` parameter from the `search_symbols` schema.
- A malformed `config.toml` now logs a warning instead of silently falling
  back to defaults.

### Changed
- Default `max_file_size_kb` raised 512 → 2048 (the old cap silently excluded
  large real-world files, e.g. monolithic SwiftUI views); existing configs at
  the old default are migrated automatically.

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
