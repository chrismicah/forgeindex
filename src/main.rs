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

    let config = match Config::load(&root) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[forgeindex] WARNING: failed to load .forgeindex/config.toml ({}); using defaults",
                e
            );
            Config::default()
        }
    };

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
        Command::Register => cmd_register()?,
        Command::Doctor => cmd_doctor(&root)?,
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
    // No index yet is fine: the server auto-indexes on the first tool call so
    // fresh checkouts and worktrees work without a manual `forgeindex init`.
    if !Config::db_path(root).exists() {
        eprintln!(
            "[forgeindex] No index in {} yet — will auto-index on first tool call.",
            root.display()
        );
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
    println!("References: {}", stats.reference_count);
    println!("Edges:      {}", stats.edge_count);
    println!("Languages:  {}", stats.languages.join(", "));
    println!("Database:   {}", db_path.display());

    if let Ok(Some(json)) = store.get_meta(indexer::LAST_SUMMARY_META_KEY) {
        if let Ok(summary) = serde_json::from_str::<indexer::IndexSummary>(&json) {
            if !summary.too_large_files.is_empty() {
                println!();
                println!(
                    "⚠ {} file(s) skipped for exceeding max_file_size_kb:",
                    summary.too_large_files.len()
                );
                for f in &summary.too_large_files {
                    println!("    {}", f);
                }
                println!("  Raise max_file_size_kb in .forgeindex/config.toml, then `forgeindex reindex`.");
            }
        }
    }

    Ok(())
}

/// Register this binary (by absolute path) as a user-scoped MCP server in
/// Claude Code. A bare command name breaks in GUI-launched apps whose PATH
/// lacks ~/.cargo/bin; current_exe() sidesteps that entirely.
fn cmd_register() -> Result<()> {
    let exe = std::env::current_exe()?
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_exe().unwrap());
    let exe_str = exe.display().to_string();

    let claude = which_claude();
    let Some(claude) = claude else {
        println!("Could not find the `claude` CLI on PATH.");
        println!("Register manually by adding this to the mcpServers section of ~/.claude.json:");
        println!(
            "  \"forgeindex\": {{ \"type\": \"stdio\", \"command\": \"{}\", \"args\": [\"serve\"] }}",
            exe_str
        );
        return Ok(());
    };

    // Remove any existing registration first (ignore failure if none exists),
    // then add with the absolute path at user scope.
    let _ = std::process::Command::new(&claude)
        .args(["mcp", "remove", "--scope", "user", "forgeindex"])
        .output();
    let out = std::process::Command::new(&claude)
        .args(["mcp", "add", "--scope", "user", "forgeindex", "--", &exe_str, "serve"])
        .output()?;

    if out.status.success() {
        println!("✓ Registered forgeindex with Claude Code (user scope)");
        println!("  Command: {} serve", exe_str);
        println!("  Works in every project and every GUI-launched app.");
        println!("  Restart running Claude Code sessions to pick it up.");
    } else {
        println!("`claude mcp add` failed:");
        println!("{}", String::from_utf8_lossy(&out.stderr));
        println!("Register manually by adding this to the mcpServers section of ~/.claude.json:");
        println!(
            "  \"forgeindex\": {{ \"type\": \"stdio\", \"command\": \"{}\", \"args\": [\"serve\"] }}",
            exe_str
        );
    }
    Ok(())
}

/// Locate the `claude` CLI, checking PATH plus common install locations that
/// GUI-launched shells may be missing.
fn which_claude() -> Option<PathBuf> {
    if let Ok(out) = std::process::Command::new("which").arg("claude").output() {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(PathBuf::from(p));
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for candidate in [
        format!("{home}/.local/bin/claude"),
        format!("{home}/.claude/local/claude"),
        "/opt/homebrew/bin/claude".to_string(),
        "/usr/local/bin/claude".to_string(),
    ] {
        let p = PathBuf::from(&candidate);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Diagnose the most common "forgeindex doesn't work here" causes.
fn cmd_doctor(root: &Path) -> Result<()> {
    println!("ForgeIndex Doctor");
    println!("─────────────────");

    // 1. Binary location
    let exe = std::env::current_exe()?;
    println!("Binary:      {}", exe.display());

    // 2. Claude Code registration
    let home = std::env::var("HOME").unwrap_or_default();
    let claude_json = PathBuf::from(format!("{home}/.claude.json"));
    if claude_json.exists() {
        let content = std::fs::read_to_string(&claude_json).unwrap_or_default();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            match v.get("mcpServers").and_then(|s| s.get("forgeindex")) {
                Some(entry) => {
                    let cmd = entry.get("command").and_then(|c| c.as_str()).unwrap_or("");
                    if Path::new(cmd).is_absolute() {
                        println!("Registered:  ✓ user scope, absolute path ({})", cmd);
                    } else {
                        println!("Registered:  ⚠ command is \"{}\" (not an absolute path)", cmd);
                        println!("             GUI-launched apps may fail to spawn it.");
                        println!("             Fix: forgeindex register");
                    }
                }
                None => {
                    println!("Registered:  ✗ not found in ~/.claude.json mcpServers");
                    println!("             Fix: forgeindex register");
                }
            }
        }
    } else {
        println!("Registered:  ? ~/.claude.json not found (Claude Code not set up?)");
    }

    // 3. Index health for the current project
    let db_path = Config::db_path(root);
    if db_path.exists() {
        let store = Store::open(&db_path)?;
        let stats = store.get_stats()?;
        println!(
            "Index:       ✓ {} files, {} symbols ({})",
            stats.file_count,
            stats.symbol_count,
            db_path.display()
        );
    } else {
        println!("Index:       none yet for {} (auto-indexes on first MCP tool call)", root.display());
    }

    Ok(())
}

fn cmd_reindex(root: &Path, config: &Config, path: Option<&str>) -> Result<()> {
    let db_path = Config::db_path(root);
    let store = Store::open(&db_path)?;

    if let Some(p) = path {
        match indexer::index_file(root, &root.join(p), &store, config)? {
            indexer::IndexOutcome::Indexed => println!("Re-indexed: {}", p),
            indexer::IndexOutcome::Unchanged => println!("Re-index skipped: {} unchanged.", p),
            indexer::IndexOutcome::Skipped(reason) => {
                println!("Re-index skipped: {} ({})", p, reason)
            }
        }
    } else {
        let summary = indexer::index_directory(root, &store, config)?;
        println!(
            "Re-indexed {} files ({} unchanged, {} scanned, {} pruned).",
            summary.indexed, summary.unchanged, summary.total_files, summary.pruned
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
            sym.kind, sym.qualified_name, sym.visibility, sym.file_path
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
