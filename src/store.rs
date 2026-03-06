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
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT s.id, f.path, s.name, s.kind, s.visibility, s.signature, s.docstring, \
             s.byte_start, s.byte_end, s.parent_symbol_id \
             FROM symbols s JOIN files f ON s.file_id = f.id \
             WHERE s.name LIKE ?1 OR s.signature LIKE ?1 \
             ORDER BY CASE WHEN s.name = ?2 THEN 0 \
                          WHEN s.name LIKE ?2 || '%' THEN 1 \
                          ELSE 2 END \
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![pattern, query, max_results as i64], |row| {
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
