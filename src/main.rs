use anyhow::Result;
use clap::Parser;
use std::path::{Path, PathBuf};
use tracing_subscriber::EnvFilter;

use forgeindex::cli::{Cli, Command, ConfigAction, HooksAction};
use forgeindex::config::Config;
use forgeindex::indexer;
use forgeindex::mcp::McpServer;
use forgeindex::store::Store;
use forgeindex::watcher;

fn main() -> Result<()> {
    let cli = Cli::parse();

    let root = cli
        .root
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let config = Config::load(&root).unwrap_or_default();

    // Initialize logging
    let default_level = if cli.verbose {
        "debug"
    } else {
        &config.server.log_level
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    match cli.command {
        Command::Init => cmd_init(&root, &config)?,
        Command::Serve => cmd_serve(&root, &config)?,
        Command::Status => cmd_status(&root)?,
        Command::Reindex { path } => cmd_reindex(&root, &config, path.as_deref())?,
        Command::Query { query, max_results } => cmd_query(&root, &query, max_results)?,
        Command::Map { max_chars } => cmd_map(&root, max_chars)?,
        Command::Hooks { action } => cmd_hooks(&root, &config, action)?,
        Command::Config { action } => cmd_config(&root, &config, action)?,
    }

    Ok(())
}

fn cmd_init(root: &Path, config: &Config) -> Result<()> {
    // Create .forgeindex directory
    let forge_dir = root.join(".forgeindex");
    std::fs::create_dir_all(&forge_dir)?;

    // Save default config
    config.save(root)?;
    println!("Initialized .forgeindex/ in {}", root.display());

    // Create database and index
    let db_path = Config::db_path(root);
    let store = Store::open(&db_path)?;
    let summary = indexer::index_directory(root, &store, config)?;
    println!(
        "Indexed {} files ({} unchanged, {} scanned).",
        summary.indexed, summary.unchanged, summary.total_files
    );

    // Install git hooks if configured
    if config.git_hooks.auto_install && root.join(".git").exists() {
        watcher::install_hooks(root, &config.git_hooks.hook_types)?;
        println!("Git hooks installed.");
    }

    Ok(())
}

fn cmd_serve(root: &Path, config: &Config) -> Result<()> {
    // Ensure index exists
    let db_path = Config::db_path(root);
    if !db_path.exists() {
        eprintln!("No index found. Run `forgeindex init` first.");
        std::process::exit(1);
    }

    let server = McpServer::new(root.to_path_buf(), config.clone());
    server.run()
}

fn cmd_status(root: &Path) -> Result<()> {
    let db_path = Config::db_path(root);
    if !db_path.exists() {
        println!("No index found. Run `forgeindex init` first.");
        return Ok(());
    }

    let store = Store::open(&db_path)?;
    let stats = store.get_stats()?;

    println!("ForgeIndex Status");
    println!("─────────────────");
    println!("Root:       {}", root.display());
    println!("Files:      {}", stats.file_count);
    println!("Symbols:    {}", stats.symbol_count);
    println!("Imports:    {}", stats.import_count);
    println!("Languages:  {}", stats.languages.join(", "));
    println!("Database:   {}", db_path.display());

    Ok(())
}

fn cmd_reindex(root: &Path, config: &Config, path: Option<&str>) -> Result<()> {
    let db_path = Config::db_path(root);
    let store = Store::open(&db_path)?;

    if let Some(p) = path {
        if indexer::index_file(root, &root.join(p), &store, config)? {
            println!("Re-indexed: {}", p);
        } else {
            println!("Re-index skipped: {} unchanged.", p);
        }
    } else {
        let summary = indexer::index_directory(root, &store, config)?;
        println!(
            "Re-indexed {} files ({} unchanged, {} scanned).",
            summary.indexed, summary.unchanged, summary.total_files
        );
    }

    Ok(())
}

fn cmd_query(root: &Path, query: &str, max_results: usize) -> Result<()> {
    let db_path = Config::db_path(root);
    if !db_path.exists() {
        println!("No index found. Run `forgeindex init` first.");
        return Ok(());
    }

    let store = Store::open(&db_path)?;
    let results = store.search_symbols(query, max_results)?;

    if results.is_empty() {
        println!("No matching symbols found for: {}", query);
        return Ok(());
    }

    for sym in &results {
        println!(
            "[{}] {} ({}) — {}",
            sym.kind, sym.name, sym.visibility, sym.file_path
        );
        println!("  {}", sym.signature);
        if let Some(ref doc) = sym.docstring {
            println!("  /// {}", doc);
        }
        println!();
    }

    Ok(())
}

fn cmd_map(root: &Path, max_chars: usize) -> Result<()> {
    let db_path = Config::db_path(root);
    if !db_path.exists() {
        println!("No index found. Run `forgeindex init` first.");
        return Ok(());
    }

    let store = Store::open(&db_path)?;
    let symbols = store.get_all_symbols()?;

    let mut output = String::new();
    let mut current_file = String::new();

    for sym in &symbols {
        if sym.parent_id.is_some() {
            continue;
        }

        if sym.file_path != current_file {
            current_file = sym.file_path.clone();
            output.push_str(&format!("\n{}:\n", current_file));
        }

        let kind_prefix = match sym.kind.as_str() {
            "function" => "fn",
            "class" => "class",
            "method" => "  fn",
            "type" => "type",
            "const" => "const",
            "interface" => "iface",
            "module" => "mod",
            _ => "",
        };

        let vis = match sym.visibility.as_str() {
            "public" => "+",
            "private" => "-",
            _ => "~",
        };

        output.push_str(&format!("  {} {} {}\n", vis, kind_prefix, sym.name));

        // Show children inline
        let children: Vec<&_> = symbols
            .iter()
            .filter(|s| s.parent_id == Some(sym.id))
            .collect();
        for child in children {
            let cvis = match child.visibility.as_str() {
                "public" => "+",
                "private" => "-",
                _ => "~",
            };
            output.push_str(&format!("    {} fn {}\n", cvis, child.name));
        }

        if output.len() > max_chars {
            output.push_str("\n... (truncated)\n");
            break;
        }
    }

    if output.is_empty() {
        println!("No symbols indexed. Run `forgeindex init` first.");
    } else {
        print!("{}", output);
    }

    Ok(())
}

fn cmd_hooks(root: &Path, config: &Config, action: HooksAction) -> Result<()> {
    match action {
        HooksAction::Install => {
            watcher::install_hooks(root, &config.git_hooks.hook_types)?;
            println!("Git hooks installed.");
        }
        HooksAction::Uninstall => {
            watcher::uninstall_hooks(root, &config.git_hooks.hook_types)?;
            println!("Git hooks removed.");
        }
    }
    Ok(())
}

fn cmd_config(root: &Path, config: &Config, action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Show => {
            let toml_str = toml::to_string_pretty(config)?;
            println!("{}", toml_str);
        }
        ConfigAction::Init => {
            config.save(root)?;
            println!(
                "Configuration written to {}",
                Config::config_path(root).display()
            );
        }
    }
    Ok(())
}
