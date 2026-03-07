use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;
use tracing::{debug, info, warn};

/// Check if verbose logging is enabled via env vars.
fn is_verbose() -> bool {
    std::env::var("FORGEINDEX_DEBUG").is_ok()
        || std::env::var("RUST_LOG")
            .map(|v| v.contains("debug") || v.contains("trace"))
            .unwrap_or(false)
}

macro_rules! verbose {
    ($($arg:tt)*) => {
        if is_verbose() {
            eprintln!($($arg)*);
        }
    };
}
use walkdir::WalkDir;
use xxhash_rust::xxh3::xxh3_64;

use crate::config::Config;
use crate::parser;
use crate::store::Store;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndexSummary {
    pub indexed: usize,
    pub unchanged: usize,
    pub total_entries: usize,
    pub total_files: usize,
    pub skipped_hidden: usize,
    pub skipped_excluded_dir: usize,
    pub skipped_filter: usize,
    pub walk_errors: usize,
}

/// Build a GlobSet from exclusion patterns.
fn build_exclude_set(patterns: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        if let Ok(glob) = Glob::new(pattern) {
            builder.add(glob);
        }
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}

/// Check if a relative path component is hidden (starts with '.')
fn is_hidden_component(component: &std::ffi::OsStr) -> bool {
    component.to_string_lossy().starts_with('.')
}

/// Directories to always skip during traversal (never descend into these).
fn should_skip_dir(dir_name: &str) -> bool {
    matches!(
        dir_name,
        "node_modules"
            | ".git"
            | ".hg"
            | ".svn"
            | "dist"
            | "build"
            | "target"
            | "__pycache__"
            | ".venv"
            | "venv"
            | ".next"
            | ".turbo"
            | ".nuxt"
            | ".output"
            | "coverage"
            | ".tox"
            | ".mypy_cache"
            | ".pytest_cache"
            | ".cargo"
            | ".forgeindex"
    )
}

/// Check why a file should or shouldn't be indexed.
/// Returns Some(reason) if skipped, None if it should be indexed.
fn skip_reason(
    rel_path: &Path,
    full_path: &Path,
    config: &Config,
    excludes: &GlobSet,
) -> Option<String> {
    // Check file extension / language support
    let lang = match parser::detect_language(full_path) {
        Some(l) => l,
        None => {
            let ext = full_path
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_else(|| "none".to_string());
            return Some(format!("unsupported extension: .{}", ext));
        }
    };

    // Swift not yet supported at runtime
    if lang == "swift" {
        return Some("swift not yet supported".to_string());
    }

    // Check if language is enabled
    if !config.index.languages.contains(&lang) {
        return Some(format!(
            "language '{}' not in config languages list {:?}",
            lang, config.index.languages
        ));
    }

    // Check file size
    if let Ok(meta) = std::fs::metadata(full_path) {
        if meta.len() > config.index.max_file_size_kb * 1024 {
            return Some(format!(
                "file too large: {} KB > {} KB limit",
                meta.len() / 1024,
                config.index.max_file_size_kb
            ));
        }
    }

    // Check exclusion patterns against RELATIVE path
    let rel_str = rel_path.to_string_lossy().replace('\\', "/");
    if excludes.is_match(&rel_str) {
        return Some(format!("matched exclude pattern (rel: {})", rel_str));
    }

    // Skip test files if configured
    if !config.index.include_tests {
        let name = full_path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.starts_with("test_")
            || name.ends_with("_test.py")
            || name.ends_with(".test.ts")
            || name.ends_with(".test.tsx")
            || name.ends_with(".test.js")
            || name.ends_with(".test.jsx")
            || name.ends_with(".spec.ts")
            || name.ends_with(".spec.tsx")
            || name.ends_with(".spec.js")
            || name.ends_with(".spec.jsx")
            || name.ends_with("_test.go")
            || name.ends_with("_test.rs")
        {
            return Some(format!("test file: {}", name));
        }
    }

    None
}

/// Index all supported files in a directory and return a summary.
pub fn index_directory(root: &Path, store: &Store, config: &Config) -> Result<IndexSummary> {
    let excludes = build_exclude_set(&config.index.exclude_patterns);
    let mut indexed = 0;
    let mut unchanged = 0;
    let mut skipped_hidden = 0;
    let mut skipped_excluded_dir = 0;
    let mut skipped_filter = 0;
    let mut walk_errors = 0;
    let mut total_entries = 0;
    let mut total_files = 0;

    let walker = WalkDir::new(root).follow_links(false).into_iter();

    verbose!("Walking directory: {}", root.display());
    verbose!("Config languages: {:?}", config.index.languages);

    for entry_result in walker {
        total_entries += 1;

        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                if let Some(path) = e.path() {
                    verbose!("WALK_ERROR at {}: {}", path.display(), e);
                } else {
                    verbose!("WALK_ERROR: {}", e);
                }
                walk_errors += 1;
                continue;
            }
        };

        let path = entry.path();

        // Skip non-files
        if !path.is_file() {
            continue;
        }

        total_files += 1;

        // Get relative path
        let rel_path = match path.strip_prefix(root) {
            Ok(r) => r,
            Err(_) => {
                verbose!(
                    "SKIP_NO_REL: {} (could not strip prefix {})",
                    path.display(),
                    root.display()
                );
                continue;
            }
        };

        // Skip files under hidden/excluded directories (relative to root)
        let mut excluded_by: Option<String> = None;
        let mut hidden_component = false;
        for c in rel_path.components() {
            let s = c.as_os_str().to_string_lossy();
            if is_hidden_component(c.as_os_str()) {
                excluded_by = Some(format!("hidden component: {}", s));
                hidden_component = true;
                break;
            }
            if should_skip_dir(&s) {
                excluded_by = Some(format!("excluded dir: {}", s));
                break;
            }
        }
        if let Some(reason) = excluded_by {
            verbose!("SKIP_DIR: {} ({})", rel_path.display(), reason);
            if hidden_component {
                skipped_hidden += 1;
            } else {
                skipped_excluded_dir += 1;
            }
            continue;
        }

        // Check if file should be indexed
        if let Some(reason) = skip_reason(rel_path, path, config, &excludes) {
            verbose!("SKIP_FILTER: {} ({})", rel_path.display(), reason);
            skipped_filter += 1;
            continue;
        }

        match index_file(root, path, store, config) {
            Ok(true) => {
                verbose!("INDEXED: {} ", rel_path.display());
                indexed += 1;
            }
            Ok(false) => {
                verbose!("UNCHANGED: {}", rel_path.display());
                unchanged += 1;
            }
            Err(e) => {
                verbose!("INDEX_ERROR: {} ({})", rel_path.display(), e);
                walk_errors += 1;
            }
        }
    }

    if is_verbose() || (indexed == 0 && unchanged == 0) {
        eprintln!(
            "[forgeindex] walked {} entries, {} files | indexed {} | unchanged {} | skipped: {} hidden, {} excluded-dir, {} filtered, {} errors",
            total_entries, total_files, indexed, unchanged, skipped_hidden, skipped_excluded_dir, skipped_filter, walk_errors
        );
    }

    if indexed == 0 && unchanged == 0 && total_files > 0 {
        eprintln!(
            "[forgeindex] WARNING: 0 files indexed out of {} found! Check language config and exclude patterns.",
            total_files
        );
    }

    info!("Indexed {} files", indexed);

    Ok(IndexSummary {
        indexed,
        unchanged,
        total_entries,
        total_files,
        skipped_hidden,
        skipped_excluded_dir,
        skipped_filter,
        walk_errors,
    })
}

/// Index a single file. Returns true if the file was (re)indexed, false if skipped
/// due to unchanged content hash.
pub fn index_file(root: &Path, path: &Path, store: &Store, _config: &Config) -> Result<bool> {
    let source = std::fs::read_to_string(path)?;
    let hash = xxh3_64(source.as_bytes());

    // Compute relative path
    let rel_path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    // JIT hash check: skip if unchanged
    if let Ok(Some(stored_hash)) = store.get_file_hash(&rel_path) {
        if stored_hash == hash.to_string() {
            verbose!("Unchanged: {}", rel_path);
            return Ok(false);
        }
    }

    let parsed = parser::parse_file(path, &source)?;

    // Override the path to be relative
    let parsed = crate::parser::ParsedFile {
        path: rel_path.clone(),
        ..parsed
    };

    store.upsert_parsed_file(&parsed)?;
    verbose!("Indexed: {} ({} symbols)", rel_path, parsed.symbols.len());

    Ok(true)
}
