use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

fn default_languages() -> Vec<String> {
    vec![
        "python".into(),
        "typescript".into(),
        "javascript".into(),
        "rust".into(),
        "go".into(),
        "java".into(),
        "c".into(),
        "cpp".into(),
        "ruby".into(),
    ]
}

fn default_exclude_patterns() -> Vec<String> {
    vec![
        "**/node_modules/**".into(),
        "**/dist/**".into(),
        "**/*.min.js".into(),
        "**/target/**".into(),
        "**/.git/**".into(),
        "**/vendor/**".into(),
        "**/build/**".into(),
        "**/__pycache__/**".into(),
        "**/.venv/**".into(),
    ]
}

fn default_max_file_size_kb() -> u64 {
    512
}

fn default_token_budget() -> usize {
    4000
}

fn default_skeleton_threshold() -> usize {
    3
}

fn default_true() -> bool {
    true
}

fn default_debounce_ms() -> u64 {
    200
}

fn default_transport() -> String {
    "stdio".into()
}

fn default_sse_port() -> u16 {
    3945
}

fn default_log_level() -> String {
    "warn".into()
}

fn default_log_file() -> String {
    ".forgeindex/forge.log".into()
}

fn default_hook_types() -> Vec<String> {
    vec!["post-commit".into(), "post-checkout".into()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub index: IndexConfig,
    #[serde(default)]
    pub compression: CompressionConfig,
    #[serde(default)]
    pub watcher: WatcherConfig,
    #[serde(default)]
    pub git_hooks: GitHooksConfig,
    #[serde(default)]
    pub server: ServerConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            index: IndexConfig::default(),
            compression: CompressionConfig::default(),
            watcher: WatcherConfig::default(),
            git_hooks: GitHooksConfig::default(),
            server: ServerConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexConfig {
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
    #[serde(default = "default_exclude_patterns")]
    pub exclude_patterns: Vec<String>,
    #[serde(default)]
    pub include_tests: bool,
    #[serde(default = "default_max_file_size_kb")]
    pub max_file_size_kb: u64,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            languages: default_languages(),
            exclude_patterns: default_exclude_patterns(),
            include_tests: false,
            max_file_size_kb: default_max_file_size_kb(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    #[serde(default = "default_token_budget")]
    pub default_token_budget: usize,
    #[serde(default = "default_skeleton_threshold")]
    pub skeleton_collapse_threshold_lines: usize,
    #[serde(default = "default_true")]
    pub aggregate_imports: bool,
    #[serde(default = "default_true")]
    pub strip_comments: bool,
    #[serde(default = "default_true")]
    pub strip_docstrings_beyond_first_line: bool,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            default_token_budget: default_token_budget(),
            skeleton_collapse_threshold_lines: default_skeleton_threshold(),
            aggregate_imports: true,
            strip_comments: true,
            strip_docstrings_beyond_first_line: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatcherConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    #[serde(default = "default_true")]
    pub respect_gitignore: bool,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            debounce_ms: default_debounce_ms(),
            respect_gitignore: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHooksConfig {
    #[serde(default = "default_true")]
    pub auto_install: bool,
    #[serde(default = "default_hook_types")]
    pub hook_types: Vec<String>,
}

impl Default for GitHooksConfig {
    fn default() -> Self {
        Self {
            auto_install: true,
            hook_types: default_hook_types(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default = "default_sse_port")]
    pub sse_port: u16,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_log_file")]
    pub log_file: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            transport: default_transport(),
            sse_port: default_sse_port(),
            log_level: default_log_level(),
            log_file: default_log_file(),
        }
    }
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let config_path = Self::config_path(root);
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config: {}", config_path.display()))?;
            let config: Config = toml::from_str(&content)
                .with_context(|| "Failed to parse config.toml")?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let config_dir = root.join(".forgeindex");
        std::fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join("config.toml");
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&config_path, content)?;
        Ok(())
    }

    pub fn config_path(root: &Path) -> PathBuf {
        root.join(".forgeindex").join("config.toml")
    }

    pub fn db_path(root: &Path) -> PathBuf {
        root.join(".forgeindex").join("index.db")
    }
}
