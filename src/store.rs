use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::parser::{Import, ParsedFile, Reference, Symbol};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolRecord {
    pub id: i64,
    pub file_path: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub visibility: String,
    pub signature: String,
    pub docstring: Option<String>,
    pub byte_start: usize,
    pub byte_end: usize,
    pub parent_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub id: i64,
    pub path: String,
    pub language: String,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRecord {
    pub file_path: String,
    pub raw_text: String,
    pub source_module: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceRecord {
    pub file_path: String,
    pub symbol_name: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeRecord {
    pub source_symbol_id: i64,
    pub target_symbol_id: i64,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub file_count: usize,
    pub symbol_count: usize,
    pub import_count: usize,
    pub reference_count: usize,
    pub edge_count: usize,
    pub languages: Vec<String>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

        // Enable WAL mode for concurrent reads
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;

        let store = Store { conn };
        store.create_tables()?;
        Ok(store)
    }

    fn create_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT UNIQUE NOT NULL,
                language TEXT NOT NULL,
                hash TEXT NOT NULL,
                last_indexed TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS symbols (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                qualified_name TEXT,
                kind TEXT NOT NULL,
                visibility TEXT NOT NULL,
                signature TEXT NOT NULL,
                docstring TEXT,
                byte_start INTEGER NOT NULL,
                byte_end INTEGER NOT NULL,
                parent_symbol_id INTEGER,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE,
                FOREIGN KEY (parent_symbol_id) REFERENCES symbols(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS imports (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL,
                raw_text TEXT NOT NULL,
                source_module TEXT,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS symbol_references (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL,
                symbol_name TEXT NOT NULL,
                byte_start INTEGER NOT NULL,
                byte_end INTEGER NOT NULL,
                context TEXT NOT NULL,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS edges (
                source_symbol_id INTEGER NOT NULL,
                target_symbol_id INTEGER NOT NULL,
                context TEXT NOT NULL,
                FOREIGN KEY (source_symbol_id) REFERENCES symbols(id) ON DELETE CASCADE,
                FOREIGN KEY (target_symbol_id) REFERENCES symbols(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
            CREATE INDEX IF NOT EXISTS idx_symbols_file_id ON symbols(file_id);
            CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);
            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            CREATE INDEX IF NOT EXISTS idx_imports_file_id ON imports(file_id);
            CREATE INDEX IF NOT EXISTS idx_refs_name ON symbol_references(symbol_name);
            CREATE INDEX IF NOT EXISTS idx_refs_file_id ON symbol_references(file_id);
            CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source_symbol_id);
            CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target_symbol_id);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_edges_unique
            ON edges(source_symbol_id, target_symbol_id, context);
            ",
        )?;
        self.ensure_symbol_qualified_names()?;
        Ok(())
    }

    fn ensure_symbol_qualified_names(&self) -> Result<()> {
        let has_column = {
            let mut stmt = self.conn.prepare("PRAGMA table_info(symbols)")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;

            let mut has_column = false;
            for row in rows {
                if row? == "qualified_name" {
                    has_column = true;
                    break;
                }
            }
            has_column
        };

        if !has_column {
            self.conn
                .execute("ALTER TABLE symbols ADD COLUMN qualified_name TEXT", [])?;
        }

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_symbols_qualified_name ON symbols(qualified_name)",
            [],
        )?;
        self.backfill_symbol_qualified_names()?;
        Ok(())
    }

    fn backfill_symbol_qualified_names(&self) -> Result<()> {
        let rows = {
            let mut stmt = self.conn.prepare(
                "SELECT s.id, f.path, s.name, s.parent_symbol_id, s.qualified_name
                 FROM symbols s
                 JOIN files f ON s.file_id = f.id
                 ORDER BY f.path, s.byte_start",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(ScopedSymbolRecord {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    name: row.get(2)?,
                    parent_id: row.get(3)?,
                    qualified_name: row.get(4)?,
                })
            })?;

            let mut values = Vec::new();
            for row in rows {
                values.push(row?);
            }
            values
        };

        if rows.iter().all(|row| {
            row.qualified_name
                .as_deref()
                .is_some_and(|value| !value.is_empty())
        }) {
            return Ok(());
        }

        let by_id: std::collections::HashMap<i64, ScopedSymbolRecord> =
            rows.iter().cloned().map(|row| (row.id, row)).collect();
        let mut cache = std::collections::HashMap::new();
        let tx = self.conn.unchecked_transaction()?;

        for row in &rows {
            let qualified_name = compute_qualified_name(row.id, &by_id, &mut cache);
            tx.execute(
                "UPDATE symbols SET qualified_name = ?1 WHERE id = ?2",
                params![qualified_name, row.id],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn get_file_hash(&self, path: &str) -> Result<Option<String>> {
        let hash = self
            .conn
            .query_row(
                "SELECT hash FROM files WHERE path = ?1",
                params![path],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(hash)
    }

    pub fn upsert_parsed_file(&self, parsed: &ParsedFile) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        // Delete existing file data
        tx.execute("DELETE FROM files WHERE path = ?1", params![&parsed.path])?;

        // Insert file record
        tx.execute(
            "INSERT INTO files (path, language, hash) VALUES (?1, ?2, ?3)",
            params![&parsed.path, &parsed.language, parsed.hash.to_string()],
        )?;
        let file_id = tx.last_insert_rowid();

        // Insert symbols recursively
        let mut inserted_symbols = Vec::new();
        insert_symbols_recursive(
            &tx,
            file_id,
            &parsed.path,
            &parsed.symbols,
            None,
            None,
            &mut inserted_symbols,
        )?;

        // Insert imports
        for imp in &parsed.imports {
            tx.execute(
                "INSERT INTO imports (file_id, raw_text, source_module) VALUES (?1, ?2, ?3)",
                params![file_id, &imp.raw_text, &imp.source_module],
            )?;
        }

        // Insert raw references
        insert_references(&tx, file_id, &parsed.references)?;

        // Resolve references to declaration edges inside the same transaction.
        insert_reference_edges(
            &tx,
            &parsed.path,
            &inserted_symbols,
            &parsed.imports,
            &parsed.references,
        )?;

        tx.commit()?;
        Ok(())
    }

    pub fn find_symbol(&self, name: &str, kind: Option<&str>) -> Result<Vec<SymbolRecord>> {
        let query = if let Some(k) = kind {
            format!(
                "SELECT s.id, f.path, s.name, COALESCE(s.qualified_name, ''), s.kind, \
                 s.visibility, s.signature, s.docstring, s.byte_start, s.byte_end, \
                 s.parent_symbol_id \
                 FROM symbols s JOIN files f ON s.file_id = f.id \
                 WHERE (s.qualified_name = ?1 OR s.name = ?1) AND s.kind = '{}'",
                k
            )
        } else {
            "SELECT s.id, f.path, s.name, COALESCE(s.qualified_name, ''), s.kind, \
             s.visibility, s.signature, s.docstring, s.byte_start, s.byte_end, s.parent_symbol_id \
             FROM symbols s JOIN files f ON s.file_id = f.id \
             WHERE s.qualified_name = ?1 OR s.name = ?1"
                .to_string()
        };

        let mut stmt = self.conn.prepare(&query)?;
        let rows = stmt.query_map(params![name], |row| {
            Ok(SymbolRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                name: row.get(2)?,
                qualified_name: row.get(3)?,
                kind: row.get(4)?,
                visibility: row.get(5)?,
                signature: row.get(6)?,
                docstring: row.get(7)?,
                byte_start: row.get(8)?,
                byte_end: row.get(9)?,
                parent_id: row.get(10)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn search_symbols(&self, query: &str, max_results: usize) -> Result<Vec<SymbolRecord>> {
        let words: Vec<&str> = query.split_whitespace().collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }

        // Filter out very short words (<=2 chars) for LIKE matching to avoid
        // "ai" matching "trait", "email", etc. Keep them for exact-name matching.
        let significant_words: Vec<&str> = words.iter().copied().filter(|w| w.len() > 2).collect();

        // If all words are short (e.g. query="ai"), still use them but require
        // word-boundary-like matching (name starts with or matches exactly)
        let mut where_parts = Vec::new();
        let mut param_values: Vec<String> = Vec::new();

        if significant_words.is_empty() {
            // All words are short — use stricter matching: name/file path word boundaries
            for word in &words {
                let p = param_values.len() + 1;
                let p2 = p + 1;
                let p3 = p2 + 1;
                where_parts.push(format!(
                    "(LOWER(s.name) = LOWER(?{p}) OR LOWER(s.qualified_name) = LOWER(?{p}) \
                     OR LOWER(s.name) LIKE LOWER(?{p2}) OR LOWER(s.qualified_name) LIKE LOWER(?{p2}) \
                     OR f.path LIKE ?{p3})"
                ));
                param_values.push(word.to_string());
                param_values.push(format!("{}%", word)); // prefix match
                param_values.push(format!("%/{}%", word)); // path component match
            }
        } else {
            // Normal multi-word search: match significant words in name/sig/doc/path
            for word in &significant_words {
                let p = param_values.len() + 1;
                where_parts.push(format!(
                    "(s.name LIKE ?{p} OR s.qualified_name LIKE ?{p} OR s.signature LIKE ?{p} \
                     OR COALESCE(s.docstring,'') LIKE ?{p} OR f.path LIKE ?{p})"
                ));
                param_values.push(format!("%{}%", word));
            }
        }

        let where_clause = where_parts.join(" OR ");

        // Ranking: exact name > multi-term match count > name match > sig match > path match
        let mut rank_parts = Vec::new();

        // Exact name match gets top priority
        let exact_idx = param_values.len() + 1;
        param_values.push(query.to_string());
        rank_parts.push(format!(
            "CASE WHEN s.name = ?{} OR s.qualified_name = ?{} THEN -100 ELSE 0 END",
            exact_idx, exact_idx
        ));

        // Score each word match: name match > path match > signature match
        let search_words = if significant_words.is_empty() {
            &words
        } else {
            &significant_words
        };
        for word in search_words {
            let p = param_values.len() + 1;
            param_values.push(format!("%{}%", word));
            rank_parts.push(format!(
                "CASE WHEN s.qualified_name LIKE ?{p} THEN -12 \
                 WHEN s.name LIKE ?{p} THEN -10 \
                 WHEN f.path LIKE ?{p} THEN -5 \
                 WHEN s.signature LIKE ?{p} THEN -3 \
                 WHEN COALESCE(s.docstring,'') LIKE ?{p} THEN -1 \
                 ELSE 0 END"
            ));
        }

        // Penalize very common symbol kinds that are usually noise
        rank_parts.push(
            "CASE WHEN s.kind = 'variable' THEN 5 \
             WHEN s.kind = 'const' THEN 3 \
             ELSE 0 END"
                .to_string(),
        );

        let rank_expr = rank_parts.join(" + ");
        let limit_idx = param_values.len() + 1;
        param_values.push(max_results.to_string());

        let sql = format!(
            "SELECT s.id, f.path, s.name, COALESCE(s.qualified_name, ''), s.kind, \
             s.visibility, s.signature, s.docstring, s.byte_start, s.byte_end, \
             s.parent_symbol_id \
             FROM symbols s JOIN files f ON s.file_id = f.id \
             WHERE {} \
             ORDER BY {} \
             LIMIT ?{}",
            where_clause, rank_expr, limit_idx
        );

        let params: Vec<&dyn rusqlite::types::ToSql> = param_values
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params.as_slice(), |row| {
            Ok(SymbolRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                name: row.get(2)?,
                qualified_name: row.get(3)?,
                kind: row.get(4)?,
                visibility: row.get(5)?,
                signature: row.get(6)?,
                docstring: row.get(7)?,
                byte_start: row.get(8)?,
                byte_end: row.get(9)?,
                parent_id: row.get(10)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_all_symbols(&self) -> Result<Vec<SymbolRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, f.path, s.name, COALESCE(s.qualified_name, ''), s.kind, \
             s.visibility, s.signature, s.docstring, s.byte_start, s.byte_end, \
             s.parent_symbol_id \
             FROM symbols s JOIN files f ON s.file_id = f.id \
             ORDER BY f.path, s.byte_start",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SymbolRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                name: row.get(2)?,
                qualified_name: row.get(3)?,
                kind: row.get(4)?,
                visibility: row.get(5)?,
                signature: row.get(6)?,
                docstring: row.get(7)?,
                byte_start: row.get(8)?,
                byte_end: row.get(9)?,
                parent_id: row.get(10)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_file_symbols(&self, file_path: &str) -> Result<Vec<SymbolRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, f.path, s.name, COALESCE(s.qualified_name, ''), s.kind, \
             s.visibility, s.signature, s.docstring, s.byte_start, s.byte_end, \
             s.parent_symbol_id \
             FROM symbols s JOIN files f ON s.file_id = f.id \
             WHERE f.path = ?1 \
             ORDER BY s.byte_start",
        )?;
        let rows = stmt.query_map(params![file_path], |row| {
            Ok(SymbolRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                name: row.get(2)?,
                qualified_name: row.get(3)?,
                kind: row.get(4)?,
                visibility: row.get(5)?,
                signature: row.get(6)?,
                docstring: row.get(7)?,
                byte_start: row.get(8)?,
                byte_end: row.get(9)?,
                parent_id: row.get(10)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_stats(&self) -> Result<IndexStats> {
        let file_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let symbol_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
        let import_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM imports", [], |r| r.get(0))?;
        let reference_count: usize =
            self.conn
                .query_row("SELECT COUNT(*) FROM symbol_references", [], |r| r.get(0))?;
        let edge_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;

        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT language FROM files ORDER BY language")?;
        let languages: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(IndexStats {
            file_count,
            symbol_count,
            import_count,
            reference_count,
            edge_count,
            languages,
        })
    }

    pub fn delete_file(&self, path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", params![path])?;
        Ok(())
    }

    pub fn get_all_files(&self) -> Result<Vec<FileRecord>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, language, hash FROM files ORDER BY path")?;
        let rows = stmt.query_map([], |row| {
            Ok(FileRecord {
                id: row.get(0)?,
                path: row.get(1)?,
                language: row.get(2)?,
                hash: row.get(3)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn search_imports(&self, query: &str, max_results: usize) -> Result<Vec<ImportRecord>> {
        let words: Vec<&str> = query.split_whitespace().collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }

        let mut where_parts = Vec::new();
        let mut param_values: Vec<String> = Vec::new();

        for word in &words {
            if word.len() <= 2 {
                // Short terms: match as exact module name or path component,
                // not substring. Prevents "ai" matching "tailwind-merge", "email", etc.
                let p1 = param_values.len() + 1;
                let p2 = p1 + 1;
                let p3 = p2 + 1;
                where_parts.push(format!(
                    "(COALESCE(i.source_module,'') = ?{p1} \
                     OR COALESCE(i.source_module,'') LIKE ?{p2} \
                     OR COALESCE(i.source_module,'') LIKE ?{p3})"
                ));
                param_values.push(word.to_string()); // exact: 'ai'
                param_values.push(format!("{}/%", word)); // prefix: 'ai/...'
                param_values.push(format!("%/{}", word)); // suffix: '.../ai'
            } else {
                let p = param_values.len() + 1;
                where_parts.push(format!(
                    "(i.raw_text LIKE ?{p} OR COALESCE(i.source_module,'') LIKE ?{p})"
                ));
                param_values.push(format!("%{}%", word));
            }
        }

        let limit_idx = param_values.len() + 1;
        param_values.push(max_results.to_string());

        let sql = format!(
            "SELECT f.path, i.raw_text, i.source_module \
             FROM imports i JOIN files f ON i.file_id = f.id \
             WHERE {} \
             ORDER BY f.path \
             LIMIT ?{}",
            where_parts.join(" OR "),
            limit_idx
        );

        let params: Vec<&dyn rusqlite::types::ToSql> = param_values
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params.as_slice(), |row| {
            Ok(ImportRecord {
                file_path: row.get(0)?,
                raw_text: row.get(1)?,
                source_module: row.get(2)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_all_imports(&self) -> Result<Vec<ImportRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.path, i.raw_text, i.source_module \
             FROM imports i JOIN files f ON i.file_id = f.id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ImportRecord {
                file_path: row.get(0)?,
                raw_text: row.get(1)?,
                source_module: row.get(2)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_all_edges(&self) -> Result<Vec<EdgeRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_symbol_id, target_symbol_id, context
             FROM edges
             ORDER BY source_symbol_id, target_symbol_id, context",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(EdgeRecord {
                source_symbol_id: row.get(0)?,
                target_symbol_id: row.get(1)?,
                context: row.get(2)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }
}

#[derive(Debug, Clone)]
struct InsertedSymbolRecord {
    id: i64,
    name: String,
    byte_start: usize,
    byte_end: usize,
}

#[derive(Debug, Clone)]
struct CandidateSymbolRecord {
    id: i64,
    file_path: String,
    parent_id: Option<i64>,
}

#[derive(Debug, Clone)]
struct ScopedSymbolRecord {
    id: i64,
    file_path: String,
    name: String,
    parent_id: Option<i64>,
    qualified_name: Option<String>,
}

fn insert_symbols_recursive(
    conn: &Connection,
    file_id: i64,
    file_path: &str,
    symbols: &[Symbol],
    parent_id: Option<i64>,
    parent_qualified_name: Option<&str>,
    inserted: &mut Vec<InsertedSymbolRecord>,
) -> Result<()> {
    for sym in symbols {
        let qualified_name = build_qualified_name(file_path, parent_qualified_name, &sym.name);
        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, visibility, signature, \
             docstring, byte_start, byte_end, parent_symbol_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                file_id,
                sym.name,
                &qualified_name,
                sym.kind.to_string(),
                sym.visibility.to_string(),
                sym.signature,
                sym.docstring,
                sym.byte_start,
                sym.byte_end,
                parent_id,
            ],
        )?;
        let sym_id = conn.last_insert_rowid();
        inserted.push(InsertedSymbolRecord {
            id: sym_id,
            name: sym.name.clone(),
            byte_start: sym.byte_start,
            byte_end: sym.byte_end,
        });

        if !sym.children.is_empty() {
            insert_symbols_recursive(
                conn,
                file_id,
                file_path,
                &sym.children,
                Some(sym_id),
                Some(&qualified_name),
                inserted,
            )?;
        }
    }
    Ok(())
}

fn insert_references(conn: &Connection, file_id: i64, references: &[Reference]) -> Result<()> {
    for reference in references {
        conn.execute(
            "INSERT INTO symbol_references (file_id, symbol_name, byte_start, byte_end, context)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                file_id,
                &reference.name,
                reference.byte_start,
                reference.byte_end,
                &reference.context
            ],
        )?;
    }
    Ok(())
}

fn insert_reference_edges(
    conn: &Connection,
    file_path: &str,
    local_symbols: &[InsertedSymbolRecord],
    imports: &[Import],
    references: &[Reference],
) -> Result<()> {
    for reference in references {
        let Some(source_symbol_id) = containing_symbol_id(reference.byte_start, local_symbols)
        else {
            continue;
        };
        let Some(target_symbol_id) = resolve_reference_target(
            conn,
            file_path,
            source_symbol_id,
            local_symbols,
            imports,
            reference,
        )?
        else {
            continue;
        };

        conn.execute(
            "INSERT OR IGNORE INTO edges (source_symbol_id, target_symbol_id, context)
             VALUES (?1, ?2, ?3)",
            params![source_symbol_id, target_symbol_id, &reference.context],
        )?;
    }

    Ok(())
}

fn containing_symbol_id(byte_offset: usize, symbols: &[InsertedSymbolRecord]) -> Option<i64> {
    symbols
        .iter()
        .filter(|symbol| symbol.byte_start <= byte_offset && byte_offset < symbol.byte_end)
        .min_by_key(|symbol| symbol.byte_end - symbol.byte_start)
        .map(|symbol| symbol.id)
}

fn resolve_reference_target(
    conn: &Connection,
    file_path: &str,
    source_symbol_id: i64,
    local_symbols: &[InsertedSymbolRecord],
    imports: &[Import],
    reference: &Reference,
) -> Result<Option<i64>> {
    let mut local_matches: Vec<&InsertedSymbolRecord> = local_symbols
        .iter()
        .filter(|symbol| symbol.name == reference.name)
        .collect();
    local_matches.sort_by_key(|symbol| symbol.byte_end - symbol.byte_start);
    if let Some(local) = local_matches
        .iter()
        .find(|symbol| symbol.id == source_symbol_id)
        .or_else(|| local_matches.first())
    {
        return Ok(Some(local.id));
    }

    let candidates = find_symbol_candidates(conn, &reference.name)?;
    if candidates.is_empty() {
        return Ok(None);
    }

    for import in imports {
        if !import_relevant_for_reference(import, &reference.name) {
            continue;
        }
        if let Some(module) = import.source_module.as_deref() {
            if let Some(candidate) = candidates.iter().find(|candidate| {
                module_matches_file_path(module, &candidate.file_path, &reference.name)
                    && candidate.file_path != file_path
            }) {
                return Ok(Some(candidate.id));
            }
        }
    }

    Ok(pick_best_candidate(&candidates).map(|candidate| candidate.id))
}

fn find_symbol_candidates(conn: &Connection, name: &str) -> Result<Vec<CandidateSymbolRecord>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, f.path, s.parent_symbol_id
         FROM symbols s
         JOIN files f ON s.file_id = f.id
         WHERE s.name = ?1
         ORDER BY f.path, s.byte_start",
    )?;
    let rows = stmt.query_map(params![name], |row| {
        Ok(CandidateSymbolRecord {
            id: row.get(0)?,
            file_path: row.get(1)?,
            parent_id: row.get(2)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

fn import_relevant_for_reference(import: &Import, reference_name: &str) -> bool {
    import
        .imported_names
        .iter()
        .any(|name| name == reference_name || name.ends_with(&format!(".{reference_name}")))
        || import
            .source_module
            .as_deref()
            .map(|module| {
                module == reference_name
                    || module.ends_with(&format!(".{reference_name}"))
                    || module.ends_with(&format!("::{reference_name}"))
            })
            .unwrap_or(false)
}

fn module_matches_file_path(module: &str, file_path: &str, reference_name: &str) -> bool {
    let module = normalize_module_path(module);
    if module.is_empty() {
        return false;
    }

    let file_path = normalize_file_path(file_path);
    file_path.ends_with(&module)
        || file_path.contains(&module)
        || file_path.ends_with(&format!("{module}/{reference_name}"))
}

fn normalize_module_path(module: &str) -> String {
    module
        .trim()
        .trim_matches('.')
        .replace("::", "/")
        .replace('.', "/")
        .trim_matches('/')
        .to_string()
}

fn normalize_file_path(path: &str) -> String {
    let mut normalized = path.replace('\\', "/");
    for suffix in [
        ".py", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".rs", ".go", ".java", ".c", ".cpp",
        ".cc", ".cxx", ".h", ".hpp", ".hh", ".hxx", ".rb",
    ] {
        if normalized.ends_with(suffix) {
            normalized.truncate(normalized.len() - suffix.len());
            break;
        }
    }
    normalized.trim_end_matches("/__init__").to_string()
}

fn pick_best_candidate(candidates: &[CandidateSymbolRecord]) -> Option<&CandidateSymbolRecord> {
    candidates
        .iter()
        .find(|candidate| candidate.parent_id.is_none())
        .or_else(|| candidates.first())
}

fn build_qualified_name(
    file_path: &str,
    parent_qualified_name: Option<&str>,
    name: &str,
) -> String {
    match parent_qualified_name {
        Some(parent) if !parent.is_empty() => format!("{parent}::{name}"),
        _ => format!("{file_path}::{name}"),
    }
}

fn compute_qualified_name(
    symbol_id: i64,
    symbols: &std::collections::HashMap<i64, ScopedSymbolRecord>,
    cache: &mut std::collections::HashMap<i64, String>,
) -> String {
    if let Some(cached) = cache.get(&symbol_id) {
        return cached.clone();
    }

    let symbol = symbols
        .get(&symbol_id)
        .expect("symbol id should exist while backfilling");
    let qualified_name = if let Some(parent_id) = symbol.parent_id {
        if symbols.contains_key(&parent_id) {
            format!(
                "{}::{}",
                compute_qualified_name(parent_id, symbols, cache),
                symbol.name
            )
        } else {
            format!("{}::{}", symbol.file_path, symbol.name)
        }
    } else {
        format!("{}::{}", symbol.file_path, symbol.name)
    };

    cache.insert(symbol_id, qualified_name.clone());
    qualified_name
}

#[cfg(test)]
mod tests {
    use super::Store;
    use crate::parser::parse_file;
    use rusqlite::Connection;
    use tempfile::tempdir;

    #[test]
    fn resolves_python_import_calls_into_edges() {
        let dir = tempdir().expect("tempdir");
        let store = Store::open(&dir.path().join("forgeindex.db")).expect("open store");

        let helper = parse_file(
            std::path::Path::new("pkg/util.py"),
            r#"
def helper():
    return 1
"#,
        )
        .expect("helper parses");
        store.upsert_parsed_file(&helper).expect("insert helper");

        let caller = parse_file(
            std::path::Path::new("main.py"),
            r#"
from pkg.util import helper

def caller():
    helper()
"#,
        )
        .expect("caller parses");
        store.upsert_parsed_file(&caller).expect("insert caller");

        let symbols = store.get_all_symbols().expect("symbols");
        let edges = store.get_all_edges().expect("edges");

        let caller_id = symbols
            .iter()
            .find(|symbol| symbol.name == "caller")
            .map(|symbol| symbol.id)
            .expect("caller symbol");
        let caller_qname = symbols
            .iter()
            .find(|symbol| symbol.name == "caller")
            .map(|symbol| symbol.qualified_name.as_str())
            .expect("caller qualified name");
        let helper_id = symbols
            .iter()
            .find(|symbol| symbol.name == "helper")
            .map(|symbol| symbol.id)
            .expect("helper symbol");
        let helper_qname = symbols
            .iter()
            .find(|symbol| symbol.name == "helper")
            .map(|symbol| symbol.qualified_name.as_str())
            .expect("helper qualified name");

        assert_eq!(caller_qname, "main.py::caller");
        assert_eq!(helper_qname, "pkg/util.py::helper");

        assert!(edges.iter().any(|edge| {
            edge.source_symbol_id == caller_id
                && edge.target_symbol_id == helper_id
                && edge.context == "call"
        }));
    }

    #[test]
    fn migrates_legacy_symbols_table_and_backfills_qualified_names() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("legacy.db");
        let conn = Connection::open(&db_path).expect("open legacy db");
        conn.execute_batch(
            "
            CREATE TABLE files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT UNIQUE NOT NULL,
                language TEXT NOT NULL,
                hash TEXT NOT NULL,
                last_indexed TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE symbols (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                visibility TEXT NOT NULL,
                signature TEXT NOT NULL,
                docstring TEXT,
                byte_start INTEGER NOT NULL,
                byte_end INTEGER NOT NULL,
                parent_symbol_id INTEGER
            );
            ",
        )
        .expect("create legacy schema");
        conn.execute(
            "INSERT INTO files (path, language, hash) VALUES ('main.py', 'python', 'abc')",
            [],
        )
        .expect("insert file");
        conn.execute(
            "INSERT INTO symbols (file_id, name, kind, visibility, signature, docstring, byte_start, byte_end, parent_symbol_id)
             VALUES (1, 'Application', 'class', 'public', 'class Application:', NULL, 0, 100, NULL)",
            [],
        )
        .expect("insert class");
        conn.execute(
            "INSERT INTO symbols (file_id, name, kind, visibility, signature, docstring, byte_start, byte_end, parent_symbol_id)
             VALUES (1, 'run', 'method', 'public', 'def run(self):', NULL, 10, 40, 1)",
            [],
        )
        .expect("insert method");
        drop(conn);

        let store = Store::open(&db_path).expect("migrate legacy db");
        let symbols = store.get_all_symbols().expect("read migrated symbols");

        assert!(symbols
            .iter()
            .any(|symbol| symbol.qualified_name == "main.py::Application"));
        assert!(symbols
            .iter()
            .any(|symbol| symbol.qualified_name == "main.py::Application::run"));
    }
}
