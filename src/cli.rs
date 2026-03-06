use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "forgeindex",
    version,
    about = "AST-driven codebase intelligence for agentic workflows"
)]
pub struct Cli {
    /// Project root directory (default: current directory)
    #[arg(short, long, global = true)]
    pub root: Option<String>,

    /// Enable verbose output (show skipped files and diagnostics)
    #[arg(long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Initialize ForgeIndex in the current directory
    Init,

    /// Start the MCP server (stdio transport)
    Serve,

    /// Show index status
    Status,

    /// Force re-index of specific file or entire repo
    Reindex {
        /// Optional path to re-index (default: entire repo)
        path: Option<String>,
    },

    /// Query the index for symbols
    Query {
        /// Search query
        query: String,

        /// Maximum results
        #[arg(short, long, default_value = "10")]
        max_results: usize,
    },

    /// Show codebase map overview
    Map {
        /// Maximum characters in output
        #[arg(short = 'c', long, default_value = "8000")]
        max_chars: usize,
    },

    /// Manage git hooks
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },

    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
pub enum HooksAction {
    /// Install ForgeIndex git hooks
    Install,
    /// Remove ForgeIndex git hooks
    Uninstall,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Show current configuration
    Show,
    /// Initialize default configuration file
    Init,
}
