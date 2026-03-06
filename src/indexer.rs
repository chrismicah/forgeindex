use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;
use tracing::{debug, info};
use walkdir::WalkDir;
use xxhash_rust::xxh3::xxh3_64;

use crate::config::Config;
use crate::parser;
use crate::store::Store;

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

/// Check if a file should be indexed.
fn should_index(path: &Path, config: &Config, excludes: &GlobSet) -> bool {
    // Check file extension / language support
    let lang = match parser::detect_language(path) {
        Some(l) => l,
        None => return false,
    };

    // Swift not yet supported at runtime
    if lang == "swift" {
        return false;
    }

    // Check if language is enabled
    if !config.index.languages.contains(&lang) {
        return false;
    }

    // Check file size
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > config.index.max_file_size_kb * 1024 {
            return false;
        }
    }

    // Check exclusion patterns
    let path_str = path.to_string_lossy().replace('\\', "/");
    if excludes.is_match(&path_str) {
        return false;
    }

    // Skip test files if configured
    if !config.index.include_tests {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.starts_with("test_")
            || name.ends_with("_test.py")
            || name.ends_with(".test.ts")
            || name.ends_with(".test.js")
            || name.ends_with(".spec.ts")
            || name.ends_with(".spec.js")
            || name.ends_with("_test.go")
            || name.ends_with("_test.rs")
        {
            return false;
        }
    }

    true
}

/// Index all supported files in a directory. Returns the count of files indexed.
pub fn index_directory(root: &Path, store: &Store, config: &Config) -> Result<usize> {
    let excludes = build_exclude_set(&config.index.exclude_patterns);
    let mut count = 0;

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        // Skip hidden directories and files
        if path
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        {
            continue;
        }

        if !should_index(path, config, &excludes) {
            continue;
        }

        match index_file(root, path, store, config) {
            Ok(true) => count += 1,
            Ok(false) => {} // skipped, unchanged
            Err(e) => {
                debug!("Skipping {}: {}", path.display(), e);
            }
        }
    }

    info!("Indexed {} files", count);
    Ok(count)
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
            debug!("Unchanged: {}", rel_path);
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
    debug!("Indexed: {} ({} symbols)", rel_path, parsed.symbols.len());

    Ok(true)
}
