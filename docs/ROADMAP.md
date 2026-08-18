# Roadmap

Research-driven backlog, distilled from a competitive audit of serena, aider's
repo map, codanna, probe, claude-context, octocode, and the Swift agent-tooling
landscape (Aug 2026). Ordered by impact.

## Near term

- **Symbol-anchored editing** (serena's most-loved feature, trivially portable
  to tree-sitter): `replace_symbol_body`, `insert_after_symbol`,
  `insert_before_symbol` addressed by name path (`Type/method`), using node
  byte extents. Plus `safe_delete_symbol` that refuses while the reference
  graph shows remaining usages.
- **Graph caching**: every graph tool currently rebuilds all symbols + edges +
  20 PageRank iterations per call. Cache in `McpServer` keyed on
  `PRAGMA data_version`; persist PageRank at index time.
- **Parallel indexing**: rayon over files, single writer thread. ~10x on cold
  index of large repos.
- **mtime+size fast path**: skip reading file contents entirely when metadata
  is unchanged; load all stored hashes in one query per run.
- **Aider-grade ranking**: adopt aider's battle-tested edge multipliers
  (mentioned-ident ×10, long snake/camel idents ×10, `_`-prefixed ×0.1,
  defined-in->5-files ×0.1, sqrt(ref-count) weight) and personalized PageRank
  with the dangling vector set to the personalization vector — but keyed on
  resolved symbol IDs instead of bare ident strings (kills aider's aliasing
  problem).

## Swift depth

- **Property-wrapper synthesis**: for `@State var foo`, also emit `_foo` and
  `$foo` aliased to the declaration so references resolve.
- **Extension merging**: unify all `extension Foo` blocks into one logical
  symbol with multi-file locations; record conformances declared in extensions.
- **Known-macro semantics**: hard-code the ~10 common macros (`@Observable` ⇒
  conforms to Observable, `#Preview` ⇒ ignore for call graph, `@Model`,
  `@Test`, …) as attribute metadata instead of expansion.
- **IndexStore enrichment (frontier)**: optional FFI to `libIndexStore.dylib`
  to overlay compiler-truth references/conformances from an existing
  DerivedData/`.build` index onto the tree-sitter graph; degrade gracefully
  when absent. No Rust crate exists for this today — greenfield advantage.

## Agent UX

- **Fused responses** (codanna's `semantic_search_with_context`): one call
  returning match + callers + callees + impact radius.
- **Session dedup** (probe): don't re-emit code blocks already returned this
  session.
- **Memories + onboarding** (serena): `.forgeindex/memories/*.md` with
  list-names-first progressive disclosure and an auto-onboarding tool.
- **Secret redaction** in `read_source`/`pack_repo` output (octocode ships
  300+ patterns).
- **`--context claude-code` flag**: hide tools that duplicate the host agent's
  own read/edit/shell to keep tool lists lean.

## Infrastructure

- **Merkle/content-hash tree incremental reindex** (claude-context): correct,
  fast re-index on branch switches.
- **MCP `roots` capability**: accept the client-provided workspace root instead
  of relying on spawn cwd.
- **VHS demo GIF** (`demo.tape` checked in) + comparison table (vs grep /
  embeddings / LSP) in the README.
