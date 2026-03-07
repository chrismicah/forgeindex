use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::parser::{ParsedFile, Symbol};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolRecord {
    pub id: i64,
    pub file_path: String,
    pub name: String,
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
pub struct IndexStats {
    pub file_count: usize,
    pub symbol_count: usize,
    pub import_count: usize,
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

            CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
            CREATE INDEX IF NOT EXISTS idx_symbols_file_id ON symbols(file_id);
            CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);
            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            CREATE INDEX IF NOT EXISTS idx_imports_file_id ON imports(file_id);
            ",
        )?;
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
        insert_symbols_recursive(&tx, file_id, &parsed.symbols, None)?;

        // Insert imports
        for imp in &parsed.imports {
            tx.execute(
                "INSERT INTO imports (file_id, raw_text, source_module) VALUES (?1, ?2, ?3)",
                params![file_id, &imp.raw_text, &imp.source_module],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn find_symbol(&self, name: &str, kind: Option<&str>) -> Result<Vec<SymbolRecord>> {
        let query = if let Some(k) = kind {
            format!(
                "SELECT s.id, f.path, s.name, s.kind, s.visibility, s.signature, s.docstring, \
                 s.byte_start, s.byte_end, s.parent_symbol_id \
                 FROM symbols s JOIN files f ON s.file_id = f.id \
                 WHERE s.name = ?1 AND s.kind = '{}'",
                k
            )
        } else {
            "SELECT s.id, f.path, s.name, s.kind, s.visibility, s.signature, s.docstring, \
             s.byte_start, s.byte_end, s.parent_symbol_id \
             FROM symbols s JOIN files f ON s.file_id = f.id \
             WHERE s.name = ?1"
                .to_string()
        };

        let mut stmt = self.conn.prepare(&query)?;
        let rows = stmt.query_map(params![name], |row| {
            Ok(SymbolRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                visibility: row.get(4)?,
                signature: row.get(5)?,
                docstring: row.get(6)?,
                byte_start: row.get(7)?,
                byte_end: row.get(8)?,
                parent_id: row.get(9)?,
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
                    "(LOWER(s.name) = LOWER(?{p}) OR LOWER(s.name) LIKE LOWER(?{p2}) \
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
                    "(s.name LIKE ?{p} OR s.signature LIKE ?{p} \
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
            "CASE WHEN s.name = ?{} THEN -100 ELSE 0 END",
            exact_idx
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
                "CASE WHEN s.name LIKE ?{p} THEN -10 \
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
            "SELECT s.id, f.path, s.name, s.kind, s.visibility, s.signature, s.docstring, \
             s.byte_start, s.byte_end, s.parent_symbol_id \
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
                kind: row.get(3)?,
                visibility: row.get(4)?,
                signature: row.get(5)?,
                docstring: row.get(6)?,
                byte_start: row.get(7)?,
                byte_end: row.get(8)?,
                parent_id: row.get(9)?,
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
            "SELECT s.id, f.path, s.name, s.kind, s.visibility, s.signature, s.docstring, \
             s.byte_start, s.byte_end, s.parent_symbol_id \
             FROM symbols s JOIN files f ON s.file_id = f.id \
             ORDER BY f.path, s.byte_start",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SymbolRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                visibility: row.get(4)?,
                signature: row.get(5)?,
                docstring: row.get(6)?,
                byte_start: row.get(7)?,
                byte_end: row.get(8)?,
                parent_id: row.get(9)?,
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
            "SELECT s.id, f.path, s.name, s.kind, s.visibility, s.signature, s.docstring, \
             s.byte_start, s.byte_end, s.parent_symbol_id \
             FROM symbols s JOIN files f ON s.file_id = f.id \
             WHERE f.path = ?1 \
             ORDER BY s.byte_start",
        )?;
        let rows = stmt.query_map(params![file_path], |row| {
            Ok(SymbolRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                visibility: row.get(4)?,
                signature: row.get(5)?,
                docstring: row.get(6)?,
                byte_start: row.get(7)?,
                byte_end: row.get(8)?,
                parent_id: row.get(9)?,
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
            let p = param_values.len() + 1;
            where_parts.push(format!(
                "(i.raw_text LIKE ?{p} OR COALESCE(i.source_module,'') LIKE ?{p})"
            ));
            param_values.push(format!("%{}%", word));
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
}

fn insert_symbols_recursive(
    conn: &Connection,
    file_id: i64,
    symbols: &[Symbol],
    parent_id: Option<i64>,
) -> Result<()> {
    for sym in symbols {
        conn.execute(
            "INSERT INTO symbols (file_id, name, kind, visibility, signature, docstring, \
             byte_start, byte_end, parent_symbol_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                file_id,
                sym.name,
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

        if !sym.children.is_empty() {
            insert_symbols_recursive(conn, file_id, &sym.children, Some(sym_id))?;
        }
    }
    Ok(())
}
