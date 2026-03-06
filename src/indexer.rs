use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;
use tracing::{debug, info, warn};
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

/// Check if a relative path component is hidden (starts with '.')
fn is_hidden_component(component: &std::ffi::OsStr) -> bool {
    component
        .to_string_lossy()
        .starts_with('.')
}

/// Check if a file should be indexed, using the RELATIVE path for all checks.
fn should_index(rel_path: &Path, full_path: &Path, config: &Config, excludes: &GlobSet) -> bool {
    // Check file extension / language support
    let lang = match parser::detect_language(full_path) {
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
    if let Ok(meta) = std::fs::metadata(full_path) {
        if meta.len() > config.index.max_file_size_kb * 1024 {
            return false;
        }
    }

    // Check exclusion patterns against RELATIVE path (not absolute)
    let rel_str = rel_path.to_string_lossy().replace('\\', "/");
    if excludes.is_match(&rel_str) {
        return false;
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
            return false;
        }
    }

    true
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

/// Index all supported files in a directory. Returns the count of files indexed.
pub fn index_directory(root: &Path, store: &Store, config: &Config) -> Result<usize> {
    let excludes = build_exclude_set(&config.index.exclude_patterns);
    let mut count = 0;
    let mut skipped_dirs = 0;
    let mut skipped_files = 0;
    let mut errors = 0;

    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter();

    for entry_result in walker {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                // Log walkdir errors instead of silently swallowing them
                if let Some(path) = e.path() {
                    debug!("Walk error at {}: {}", path.display(), e);
                } else {
                    debug!("Walk error: {}", e);
                }
                errors += 1;
                continue;
            }
        };

        let path = entry.path();

        // For directories: check if we should skip entirely (prune)
        if entry.file_type().is_dir() {
            if let Some(dir_name) = path.file_name() {
                let name = dir_name.to_string_lossy();
                // Skip hidden directories (relative to root)
                if path != root && (name.starts_with('.') || should_skip_dir(&name)) {
                    skipped_dirs += 1;
                    // Note: we can't skip with filter_entry since we're using into_iter()
                    // but walkdir will still descend. We handle this by checking rel_path below.
                    continue;
                }
            }
            continue;
        }

        if !path.is_file() {
            continue;
        }

        // Get relative path
        let rel_path = match path.strip_prefix(root) {
            Ok(r) => r,
            Err(_) => {
                debug!("Could not compute relative path for {}", path.display());
                continue;
            }
        };

        // Skip files under hidden/excluded directories (relative to root)
        let in_excluded_dir = rel_path.components().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            is_hidden_component(c.as_os_str()) || should_skip_dir(&s)
        });
        if in_excluded_dir {
            continue;
        }

        if !should_index(rel_path, path, config, &excludes) {
            skipped_files += 1;
            continue;
        }

        match index_file(root, path, store, config) {
            Ok(true) => count += 1,
            Ok(false) => {} // skipped, unchanged hash
            Err(e) => {
                debug!("Skipping {}: {}", rel_path.display(), e);
                errors += 1;
            }
        }
    }

    if count == 0 {
        // Emit diagnostic info when nothing was indexed
        warn!(
            "Indexed 0 files (skipped {} dirs, {} files, {} errors). \
             Check that source files exist and languages are enabled in config.",
            skipped_dirs, skipped_files, errors
        );
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
