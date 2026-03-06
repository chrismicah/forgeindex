# ForgeIndex MCP Tool Reference

Complete reference for all MCP tools exposed by ForgeIndex. Use these tools in Claude Desktop, Conductor, or any MCP-compatible client.

---

## map_overview

Get a hierarchical text tree of all public symbols in the indexed codebase.

**Parameters:**
| Name | Type | Default | Description |
|------|------|---------|-------------|
| `max_chars` | integer | 8000 | Maximum characters in the response |

**Example Response:**
```
src/auth/service.py
  class AuthService
    def login(username: str, password: str) -> Token
    def refresh(token: Token) -> Token
    def revoke(token: Token) -> None
  class TokenValidator
    def validate(token: str) -> bool
src/auth/models.py
  class Token
  class User
```

**Token Cost:** ~2,000

---

## find_symbol

Find a symbol by exact name. Returns signature, file path, line number, and docstring.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| `name` | string | ✅ | Exact symbol name to find |
| `kind` | string | ❌ | Filter by kind: `function`, `class`, `method`, `type`, `const`, `interface` |

**Example Response:**
```json
{
  "name": "AuthService",
  "kind": "class",
  "file": "src/auth/service.py",
  "line": 15,
  "signature": "class AuthService:",
  "docstring": "Handles user authentication and token management.",
  "visibility": "public"
}
```

**Token Cost:** ~50–200

---

## read_source

Read the full source code of a specific symbol. Uses byte ranges to extract only the symbol — does not read the entire file.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| `symbol` | string | ✅ | Symbol name to read |

**Token Cost:** ~100–1,000

---

## search_symbols

Fuzzy symbol search ranked by TF-IDF relevance. Returns compressed results within a token budget.

**Parameters:**
| Name | Type | Default | Description |
|------|------|---------|-------------|
| `query` | string | — | Search query (required) |
| `max_results` | integer | 10 | Maximum number of results |
| `max_tokens` | integer | 2000 | Token budget for results |

**Token Cost:** ~200–2,000

---

## get_skeleton

Return a skeletonized view of a file: signatures only, function bodies collapsed to `{ ... }`, imports aggregated.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| `file_path` | string | ✅ | Relative path to the source file |

**Example Response:**
```python
# src/auth/service.py
import os, hashlib, jwt

class AuthService:
    def __init__(self, db: Database, secret: str): ...
    def login(self, username: str, password: str) -> Token: ...
    def refresh(self, token: Token) -> Token: ...
    def revoke(self, token: Token) -> None: ...

class TokenValidator:
    def __init__(self, secret: str): ...
    def validate(self, token: str) -> bool: ...
```

**Token Cost:** ~100–500 (60–85% reduction from raw file)

---

## get_dependencies

Get direct dependencies or dependents of a symbol.

**Parameters:**
| Name | Type | Default | Description |
|------|------|---------|-------------|
| `symbol` | string | — | Symbol name (required) |
| `direction` | string | `both` | `in` (dependents), `out` (dependencies), or `both` |

**Token Cost:** ~100–300

---

## get_impact

Compute the blast radius of changing a symbol. Returns all transitive dependents — everything that would be affected by a change.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| `symbol` | string | ✅ | Symbol name to analyze |

**Example Response:**
```json
{
  "symbol": "Token",
  "direct_dependents": 4,
  "transitive_dependents": 12,
  "affected_files": [
    "src/auth/service.py",
    "src/api/handlers.py",
    "src/middleware/auth.py"
  ],
  "affected_symbols": [
    "AuthService.login",
    "AuthService.refresh",
    "TokenValidator.validate",
    "auth_middleware"
  ]
}
```

**Token Cost:** ~100–500

---

## get_ranked_symbols

Get the top N symbols by PageRank importance — the most structurally important symbols in the codebase.

**Parameters:**
| Name | Type | Default | Description |
|------|------|---------|-------------|
| `top_n` | integer | 10 | Number of symbols to return |
| `kind` | string | — | Optional kind filter |

**Token Cost:** ~200–800

---

## compress_context

The core power tool. Returns maximally relevant compressed code for a natural-language query, optimized to fit within a strict token budget. Uses TF-IDF ranking and greedy knapsack packing.

**Parameters:**
| Name | Type | Default | Description |
|------|------|---------|-------------|
| `query` | string | — | Natural language or identifier query (required) |
| `token_budget` | integer | 4000 | Maximum tokens in response |

**Example:**
```
Query: "authentication token refresh flow"
Budget: 2000 tokens

→ Returns the most relevant function signatures, class definitions,
  and dependency annotations that explain the auth token refresh flow,
  packed into exactly ≤2000 tokens.
```

**Token Cost:** = token_budget (exactly what you asked for)

---

## pack_repo

Pack the entire repository into a single compressed artifact (XML or JSON format), fitting within the specified token budget.

**Parameters:**
| Name | Type | Default | Description |
|------|------|---------|-------------|
| `token_budget` | integer | — | Maximum tokens (required) |
| `format` | string | `xml` | Output format: `xml` or `json` |

**Token Cost:** = token_budget

---

## index_status

Returns index health metrics: file count, symbol count, last update timestamp, number of stale files.

**Parameters:** None

**Token Cost:** ~50

---

## reindex

Force re-index of a specific file or the entire repository. Rarely needed due to auto-reindexing, but useful for debugging. Directory reindex responses report both files that were updated and files that were unchanged.

**Parameters:**
| Name | Type | Required | Description |
|------|------|----------|-------------|
| `path` | string | ❌ | Specific file to reindex (omit for full repo) |

**Token Cost:** ~50

---

## Agent Directive

Add this to your agent's system prompt for best results:

```
RULE: For all code exploration, use forgeindex MCP tools (map_overview, find_symbol,
read_source, get_skeleton, compress_context). NEVER use grep, cat, or find to read
source files. Only use cat when you need to edit a specific file you have already
identified via forgeindex.
```

---

## Error Codes

| Code | Description |
|------|-------------|
| `SYMBOL_NOT_FOUND` | The requested symbol does not exist in the index |
| `INDEX_STALE` | The index needs rebuilding (run `reindex`) |
| `BUDGET_EXCEEDED` | The minimum useful response exceeds the token budget |
| `PARSE_ERROR` | A source file could not be parsed |
