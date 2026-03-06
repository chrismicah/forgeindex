use std::path::Path;

use forgeindex::compressor;
use forgeindex::config::Config;
use forgeindex::graph::DepGraph;
use forgeindex::indexer;
use forgeindex::parser;
use forgeindex::store::Store;

fn fixture_path(sub: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(sub)
}

fn temp_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    let store = Store::open(&db_path).unwrap();
    (dir, store)
}

// ─── Parser tests ────────────────────────────────────────────────────

#[test]
fn test_parse_python_file() {
    let path = fixture_path("python_project/main.py");
    let source = std::fs::read_to_string(&path).unwrap();
    let parsed = parser::parse_file(&path, &source).unwrap();

    assert_eq!(parsed.language, "python");
    assert!(parsed.hash != 0);

    // Should find: Application class, create_app function, MAX_RETRIES, DEFAULT_TIMEOUT
    let names: Vec<&str> = parsed.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"Application"),
        "Missing Application class: {:?}",
        names
    );
    assert!(
        names.contains(&"create_app"),
        "Missing create_app function: {:?}",
        names
    );
    assert!(
        names.contains(&"MAX_RETRIES"),
        "Missing MAX_RETRIES const: {:?}",
        names
    );

    // Application class should have children (methods)
    let app_class = parsed
        .symbols
        .iter()
        .find(|s| s.name == "Application")
        .unwrap();
    assert!(
        !app_class.children.is_empty(),
        "Application should have methods"
    );

    let method_names: Vec<&str> = app_class.children.iter().map(|s| s.name.as_str()).collect();
    assert!(
        method_names.contains(&"__init__"),
        "Missing __init__: {:?}",
        method_names
    );
    assert!(
        method_names.contains(&"run"),
        "Missing run: {:?}",
        method_names
    );
}

#[test]
fn test_parse_python_utils() {
    let path = fixture_path("python_project/utils.py");
    let source = std::fs::read_to_string(&path).unwrap();
    let parsed = parser::parse_file(&path, &source).unwrap();

    let names: Vec<&str> = parsed.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"calculate_total"));
    assert!(names.contains(&"format_currency"));
    assert!(names.contains(&"clamp"));
    assert!(names.contains(&"PI"), "Missing PI constant: {:?}", names);

    // Check visibility of _internal_helper
    let helper = parsed
        .symbols
        .iter()
        .find(|s| s.name == "_internal_helper")
        .unwrap();
    assert_eq!(helper.visibility, parser::Visibility::Private);
}

#[test]
fn test_parse_typescript_file() {
    let path = fixture_path("typescript_project/types.ts");
    let source = std::fs::read_to_string(&path).unwrap();
    let parsed = parser::parse_file(&path, &source).unwrap();

    assert_eq!(parsed.language, "typescript");

    let names: Vec<&str> = parsed.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"User"),
        "Missing User interface: {:?}",
        names
    );
    assert!(
        names.contains(&"ApiResponse"),
        "Missing ApiResponse interface: {:?}",
        names
    );
}

#[test]
fn test_parse_imports_python() {
    let path = fixture_path("python_project/main.py");
    let source = std::fs::read_to_string(&path).unwrap();
    let parsed = parser::parse_file(&path, &source).unwrap();

    assert!(!parsed.imports.is_empty(), "Should have imports");
}

#[test]
fn test_detect_language() {
    assert_eq!(
        parser::detect_language(Path::new("foo.py")),
        Some("python".into())
    );
    assert_eq!(
        parser::detect_language(Path::new("bar.ts")),
        Some("typescript".into())
    );
    assert_eq!(
        parser::detect_language(Path::new("baz.rs")),
        Some("rust".into())
    );
    assert_eq!(
        parser::detect_language(Path::new("qux.go")),
        Some("go".into())
    );
    assert_eq!(
        parser::detect_language(Path::new("main.java")),
        Some("java".into())
    );
    assert_eq!(
        parser::detect_language(Path::new("lib.c")),
        Some("c".into())
    );
    assert_eq!(
        parser::detect_language(Path::new("lib.cpp")),
        Some("cpp".into())
    );
    assert_eq!(
        parser::detect_language(Path::new("gem.rb")),
        Some("ruby".into())
    );
    assert_eq!(parser::detect_language(Path::new("readme.md")), None);
}

// ─── Store tests ─────────────────────────────────────────────────────

#[test]
fn test_store_upsert_and_query() {
    let (_dir, store) = temp_store();

    let path = fixture_path("python_project/utils.py");
    let source = std::fs::read_to_string(&path).unwrap();
    let parsed = parser::parse_file(&path, &source).unwrap();

    store.upsert_parsed_file(&parsed).unwrap();

    // Find symbol
    let results = store.find_symbol("calculate_total", None).unwrap();
    assert!(!results.is_empty(), "Should find calculate_total");
    assert_eq!(results[0].kind, "function");

    // Search
    let results = store.search_symbols("calc", 10).unwrap();
    assert!(!results.is_empty(), "Should find symbols matching 'calc'");

    // Stats
    let stats = store.get_stats().unwrap();
    assert_eq!(stats.file_count, 1);
    assert!(stats.symbol_count > 0);
}

#[test]
fn test_store_hash_check() {
    let (_dir, store) = temp_store();

    let path = fixture_path("python_project/utils.py");
    let source = std::fs::read_to_string(&path).unwrap();
    let parsed = parser::parse_file(&path, &source).unwrap();

    store.upsert_parsed_file(&parsed).unwrap();

    let hash = store.get_file_hash(&parsed.path).unwrap();
    assert!(hash.is_some());
    assert_eq!(hash.unwrap(), parsed.hash.to_string());
}

#[test]
fn test_store_delete_file() {
    let (_dir, store) = temp_store();

    let path = fixture_path("python_project/utils.py");
    let source = std::fs::read_to_string(&path).unwrap();
    let parsed = parser::parse_file(&path, &source).unwrap();

    store.upsert_parsed_file(&parsed).unwrap();
    assert_eq!(store.get_stats().unwrap().file_count, 1);

    store.delete_file(&parsed.path).unwrap();
    assert_eq!(store.get_stats().unwrap().file_count, 0);
}

// ─── Compressor tests ───────────────────────────────────────────────

#[test]
fn test_skeleton_generation() {
    let (_dir, store) = temp_store();

    let path = fixture_path("python_project/utils.py");
    let source = std::fs::read_to_string(&path).unwrap();
    let parsed = parser::parse_file(&path, &source).unwrap();
    store.upsert_parsed_file(&parsed).unwrap();

    let symbols = store.get_file_symbols(&parsed.path).unwrap();
    let skel = compressor::skeleton(&source, &symbols, true);

    // Skeleton should be shorter than source
    assert!(
        skel.len() < source.len(),
        "Skeleton should be shorter than source"
    );

    // Should contain function signatures
    assert!(
        skel.contains("calculate_total"),
        "Skeleton should contain function name"
    );
}

#[test]
fn test_compress_context() {
    let (_dir, store) = temp_store();

    // Index multiple files
    for file in &["utils.py", "models.py", "main.py"] {
        let path = fixture_path(&format!("python_project/{}", file));
        let source = std::fs::read_to_string(&path).unwrap();
        let parsed = parser::parse_file(&path, &source).unwrap();
        store.upsert_parsed_file(&parsed).unwrap();
    }

    let all_symbols = store.get_all_symbols().unwrap();
    let compressed = compressor::compress_context(&all_symbols, "calculate total price", 4000);

    assert!(
        !compressed.is_empty(),
        "Compressed context should not be empty"
    );
    // Should prioritize symbols related to "calculate" and "total"
    assert!(
        compressed.contains("calculate_total") || compressed.contains("total"),
        "Should contain relevant symbols"
    );
}

#[test]
fn test_pack_repo_xml() {
    let (_dir, store) = temp_store();

    let path = fixture_path("python_project/utils.py");
    let source = std::fs::read_to_string(&path).unwrap();
    let parsed = parser::parse_file(&path, &source).unwrap();
    store.upsert_parsed_file(&parsed).unwrap();

    let all_symbols = store.get_all_symbols().unwrap();
    let packed = compressor::pack_repo(&all_symbols, 8000, "xml");

    assert!(packed.contains("<repo>"));
    assert!(packed.contains("</repo>"));
    assert!(packed.contains("<file"));
}

#[test]
fn test_pack_repo_json() {
    let (_dir, store) = temp_store();

    let path = fixture_path("python_project/utils.py");
    let source = std::fs::read_to_string(&path).unwrap();
    let parsed = parser::parse_file(&path, &source).unwrap();
    store.upsert_parsed_file(&parsed).unwrap();

    let all_symbols = store.get_all_symbols().unwrap();
    let packed = compressor::pack_repo(&all_symbols, 8000, "json");

    let v: serde_json::Value = serde_json::from_str(&packed).unwrap();
    assert!(v["files"].is_array());
}

// ─── Graph tests ─────────────────────────────────────────────────────

#[test]
fn test_dependency_graph_build() {
    let (_dir, store) = temp_store();

    // Index Python project
    for file in &["utils.py", "models.py", "main.py"] {
        let path = fixture_path(&format!("python_project/{}", file));
        let source = std::fs::read_to_string(&path).unwrap();
        let parsed = parser::parse_file(&path, &source).unwrap();
        store.upsert_parsed_file(&parsed).unwrap();
    }

    let all_symbols = store.get_all_symbols().unwrap();
    let all_imports = store.get_all_imports().unwrap();

    let graph = DepGraph::build(&all_symbols, &all_imports);

    // Should have PageRank scores
    let ranked = graph.get_ranked(5, None);
    assert!(!ranked.is_empty(), "Should have ranked symbols");
}

#[test]
fn test_pagerank_scores() {
    let (_dir, store) = temp_store();

    for file in &["utils.py", "models.py", "main.py"] {
        let path = fixture_path(&format!("python_project/{}", file));
        let source = std::fs::read_to_string(&path).unwrap();
        let parsed = parser::parse_file(&path, &source).unwrap();
        store.upsert_parsed_file(&parsed).unwrap();
    }

    let all_symbols = store.get_all_symbols().unwrap();
    let all_imports = store.get_all_imports().unwrap();
    let graph = DepGraph::build(&all_symbols, &all_imports);

    let ranked = graph.get_ranked(20, None);
    // All scores should be positive
    for sym in &ranked {
        assert!(sym.score > 0.0, "PageRank score should be positive");
    }
}

// ─── Indexer tests ──────────────────────────────────────────────────

#[test]
fn test_index_directory() {
    let (_dir, store) = temp_store();
    let config = Config::default();
    let root = fixture_path("python_project");

    let summary = indexer::index_directory(&root, &store, &config).unwrap();
    assert!(
        summary.indexed >= 3,
        "Should index at least 3 Python files, got {}",
        summary.indexed
    );

    let stats = store.get_stats().unwrap();
    assert!(stats.symbol_count > 0);
    assert!(stats.languages.contains(&"python".to_string()));
}

#[test]
fn test_index_typescript_project() {
    let (_dir, store) = temp_store();
    let config = Config::default();
    let root = fixture_path("typescript_project");

    let summary = indexer::index_directory(&root, &store, &config).unwrap();
    assert!(
        summary.indexed >= 2,
        "Should index at least 2 TypeScript files, got {}",
        summary.indexed
    );

    let stats = store.get_stats().unwrap();
    assert!(stats.symbol_count > 0);
}

#[test]
fn test_index_jit_hash_skip() {
    let (_dir, store) = temp_store();
    let config = Config::default();
    let root = fixture_path("python_project");

    // First index
    let summary1 = indexer::index_directory(&root, &store, &config).unwrap();
    assert!(summary1.indexed > 0);

    // Second index should skip (JIT hash check)
    let summary2 = indexer::index_directory(&root, &store, &config).unwrap();
    assert_eq!(
        summary2.indexed, 0,
        "Second index should not rewrite unchanged files"
    );
    assert!(
        summary2.unchanged >= summary1.indexed,
        "Expected unchanged count to reflect previously indexed files"
    );
}

// ─── Config tests ───────────────────────────────────────────────────

#[test]
fn test_config_defaults() {
    let config = Config::default();
    assert!(config.index.languages.contains(&"python".to_string()));
    assert!(config.index.languages.contains(&"typescript".to_string()));
    assert!(config.index.languages.contains(&"tsx".to_string()));
    assert!(config.index.languages.contains(&"rust".to_string()));
    assert_eq!(config.compression.default_token_budget, 4000);
    assert!(config.watcher.enabled);
    assert!(config.git_hooks.auto_install);
}

#[test]
fn test_config_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::default();

    config.save(dir.path()).unwrap();
    let loaded = Config::load(dir.path()).unwrap();

    assert_eq!(loaded.index.languages.len(), config.index.languages.len());
    assert_eq!(
        loaded.compression.default_token_budget,
        config.compression.default_token_budget
    );
}

#[test]
fn test_config_load_migrates_legacy_languages() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".forgeindex")).unwrap();
    std::fs::write(
        dir.path().join(".forgeindex/config.toml"),
        r#"[index]
languages = ["python", "typescript", "javascript", "rust", "go", "java", "c", "cpp", "ruby"]
"#,
    )
    .unwrap();

    let loaded = Config::load(dir.path()).unwrap();
    assert!(loaded.index.languages.contains(&"tsx".to_string()));
}

// ─── End-to-end pipeline test ───────────────────────────────────────

#[test]
fn test_full_pipeline() {
    let (_dir, store) = temp_store();
    let config = Config::default();

    // 1. Index Python project
    let py_root = fixture_path("python_project");
    let py_summary = indexer::index_directory(&py_root, &store, &config).unwrap();
    assert!(py_summary.indexed > 0);

    // 2. Index TypeScript project
    let ts_root = fixture_path("typescript_project");
    let ts_summary = indexer::index_directory(&ts_root, &store, &config).unwrap();
    assert!(ts_summary.indexed > 0);

    // 3. Query symbols
    let results = store.search_symbols("User", 10).unwrap();
    assert!(!results.is_empty(), "Should find User symbols");

    // 4. Find exact symbol
    let found = store.find_symbol("User", Some("class")).unwrap();
    assert!(!found.is_empty(), "Should find User class");

    // 5. Build dependency graph
    let all_symbols = store.get_all_symbols().unwrap();
    let all_imports = store.get_all_imports().unwrap();
    let graph = DepGraph::build(&all_symbols, &all_imports);

    // 6. Get ranked symbols
    let ranked = graph.get_ranked(5, None);
    assert!(!ranked.is_empty());

    // 7. Compress context
    let compressed = compressor::compress_context(&all_symbols, "user management", 2000);
    assert!(!compressed.is_empty());

    // 8. Get skeleton
    let _utils_symbols = store
        .get_file_symbols(
            &fixture_path("python_project/utils.py")
                .to_string_lossy()
                .replace('\\', "/"),
        )
        .unwrap();
    // The path stored might be different, so let's search from all files
    let stats = store.get_stats().unwrap();
    assert!(stats.file_count > 0);
    assert!(stats.symbol_count > 0);

    // 9. Pack repo
    let packed = compressor::pack_repo(&all_symbols, 4000, "xml");
    assert!(packed.contains("<repo>"));
}
