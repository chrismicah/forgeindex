use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info};

use crate::compressor;
use crate::config::Config;
use crate::graph::{DepGraph, Direction};
use crate::indexer;
use crate::store::Store;

pub struct McpServer {
    root_path: PathBuf,
    config: Config,
}

impl McpServer {
    pub fn new(root_path: PathBuf, config: Config) -> Self {
        Self { root_path, config }
    }

    pub fn run(&self) -> Result<()> {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();

        info!("ForgeIndex MCP server starting on stdio");

        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    error!("Failed to read stdin: {}", e);
                    break;
                }
            };

            if line.trim().is_empty() {
                continue;
            }

            debug!("← {}", line);

            let request: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    let err_response = json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {
                            "code": -32700,
                            "message": format!("Parse error: {}", e)
                        }
                    });
                    let resp = serde_json::to_string(&err_response)?;
                    debug!("→ {}", resp);
                    writeln!(stdout, "{}", resp)?;
                    stdout.flush()?;
                    continue;
                }
            };

            match self.handle_request(&request) {
                Ok(Some(response)) => {
                    let resp = serde_json::to_string(&response)?;
                    debug!("→ {}", resp);
                    writeln!(stdout, "{}", resp)?;
                    stdout.flush()?;
                }
                Ok(None) => {
                    // Notification, no response needed
                }
                Err(e) => {
                    let id = request.get("id").cloned().unwrap_or(Value::Null);
                    let err_response = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32603,
                            "message": format!("Internal error: {}", e)
                        }
                    });
                    let resp = serde_json::to_string(&err_response)?;
                    debug!("→ {}", resp);
                    writeln!(stdout, "{}", resp)?;
                    stdout.flush()?;
                }
            }
        }

        Ok(())
    }

    fn handle_request(&self, req: &Value) -> Result<Option<Value>> {
        let method = req["method"].as_str().unwrap_or("");
        let id = req.get("id").cloned();
        let params = req.get("params").cloned().unwrap_or(json!({}));

        match method {
            "initialize" => {
                let result = json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "forgeindex",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                });
                Ok(Some(jsonrpc_result(id, result)))
            }

            "notifications/initialized" => Ok(None),

            "tools/list" => {
                let tools = self.tool_definitions();
                Ok(Some(jsonrpc_result(id, json!({ "tools": tools }))))
            }

            "tools/call" => {
                let tool_name = params["name"].as_str().unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

                match self.call_tool(tool_name, &arguments) {
                    Ok(result) => Ok(Some(jsonrpc_result(
                        id,
                        json!({
                            "content": [{
                                "type": "text",
                                "text": result
                            }]
                        }),
                    ))),
                    Err(e) => Ok(Some(jsonrpc_result(
                        id,
                        json!({
                            "content": [{
                                "type": "text",
                                "text": format!("Error: {}", e)
                            }],
                            "isError": true
                        }),
                    ))),
                }
            }

            _ => {
                if id.is_some() {
                    Ok(Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32601,
                            "message": format!("Method not found: {}", method)
                        }
                    })))
                } else {
                    Ok(None) // Unknown notification, ignore
                }
            }
        }
    }

    fn call_tool(&self, name: &str, args: &Value) -> Result<String> {
        let db_path = Config::db_path(&self.root_path);
        let store = Store::open(&db_path)?;

        match name {
            "map_overview" => {
                let max_chars = args["max_chars"].as_u64().unwrap_or(120000) as usize;
                let detail = args["detail"].as_str().unwrap_or("summary");
                self.tool_map_overview(&store, max_chars, detail)
            }
            "find_symbol" => {
                let sym_name = args["name"]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing 'name' parameter"))?;
                let kind = args["kind"].as_str();
                self.tool_find_symbol(&store, sym_name, kind)
            }
            "read_source" => {
                let symbol = args["symbol"]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing 'symbol' parameter"))?;
                let max_chars = args["max_chars"].as_u64().unwrap_or(20000) as usize;
                let mode = args["mode"].as_str().unwrap_or("full");
                self.tool_read_source(&store, symbol, max_chars, mode)
            }
            "search_symbols" => {
                let query = args["query"]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing 'query' parameter"))?;
                let max_results = args["max_results"].as_u64().unwrap_or(10) as usize;
                self.tool_search_symbols(&store, query, max_results)
            }
            "get_skeleton" => {
                let file_path = args["file_path"]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing 'file_path' parameter"))?;
                self.tool_get_skeleton(&store, file_path)
            }
            "get_dependencies" => {
                let symbol = args["symbol"]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing 'symbol' parameter"))?;
                let direction = args["direction"].as_str().unwrap_or("both");
                let max_results = args["max_results"].as_u64().unwrap_or(20) as usize;
                self.tool_get_dependencies(&store, symbol, direction, max_results)
            }
            "get_impact" => {
                let symbol = args["symbol"]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing 'symbol' parameter"))?;
                let max_depth = args["max_depth"].as_u64().unwrap_or(3) as usize;
                let max_results = args["max_results"].as_u64().unwrap_or(30) as usize;
                self.tool_get_impact(&store, symbol, max_depth, max_results)
            }
            "trace_data_flow" => {
                let symbol = args["symbol"]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing 'symbol' parameter"))?;
                let max_depth = args["max_depth"].as_u64().unwrap_or(3) as usize;
                let max_results = args["max_results"].as_u64().unwrap_or(20) as usize;
                self.tool_trace_data_flow(&store, symbol, max_depth, max_results)
            }
            "get_ranked_symbols" => {
                let top_n = args["top_n"].as_u64().unwrap_or(10) as usize;
                let kind = args["kind"].as_str();
                let path_filter = args["path"].as_str();
                self.tool_get_ranked(&store, top_n, kind, path_filter)
            }
            "compress_context" => {
                let query = args["query"]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing 'query' parameter"))?;
                let budget = args["token_budget"]
                    .as_u64()
                    .unwrap_or(self.config.compression.default_token_budget as u64)
                    as usize;
                self.tool_compress_context(&store, query, budget)
            }
            "pack_repo" => {
                let budget = args["token_budget"]
                    .as_u64()
                    .unwrap_or(self.config.compression.default_token_budget as u64)
                    as usize;
                let format = args["format"].as_str().unwrap_or("xml");
                self.tool_pack_repo(&store, budget, format)
            }
            "search_imports" => {
                let query = args["query"]
                    .as_str()
                    .ok_or_else(|| anyhow!("missing 'query' parameter"))?;
                let max_results = args["max_results"].as_u64().unwrap_or(30) as usize;
                self.tool_search_imports(&store, query, max_results)
            }
            "index_status" => self.tool_index_status(&store),
            "reindex" => {
                let path = args["path"].as_str();
                self.tool_reindex(&store, path)
            }
            _ => Err(anyhow!("Unknown tool: {}", name)),
        }
    }

    fn tool_map_overview(&self, store: &Store, max_chars: usize, detail: &str) -> Result<String> {
        let symbols = store.get_all_symbols()?;
        if symbols.is_empty() {
            return Ok("No symbols indexed. Run `forgeindex reindex` first.".to_string());
        }

        // Group symbols by file
        let mut by_file: std::collections::BTreeMap<&str, Vec<&crate::store::SymbolRecord>> =
            std::collections::BTreeMap::new();
        for sym in &symbols {
            by_file.entry(&sym.file_path).or_default().push(sym);
        }

        let stats = store.get_stats()?;

        match detail {
            "tree" => {
                // Compact: directory tree with file symbol counts
                let mut output = format!(
                    "Codebase: {} files, {} symbols, {} imports\n\n",
                    stats.file_count, stats.symbol_count, stats.import_count
                );

                // Build directory tree
                let mut dirs: std::collections::BTreeMap<String, Vec<(&str, usize)>> =
                    std::collections::BTreeMap::new();
                for (file, syms) in &by_file {
                    let dir = std::path::Path::new(file)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| ".".to_string());
                    let top_level = syms.iter().filter(|s| s.parent_id.is_none()).count();
                    dirs.entry(dir).or_default().push((file, top_level));
                }

                for (dir, files) in &dirs {
                    let total_syms: usize = files.iter().map(|(_, c)| c).sum();
                    output.push_str(&format!(
                        "{}/  ({} files, {} symbols)\n",
                        dir,
                        files.len(),
                        total_syms
                    ));
                    if output.len() > max_chars {
                        output.push_str("... (truncated)\n");
                        break;
                    }
                }

                Ok(output)
            }

            "summary" => {
                // Medium: file paths + top-level symbol names (no signatures)
                let mut output = format!(
                    "Codebase: {} files, {} symbols, {} imports\n",
                    stats.file_count, stats.symbol_count, stats.import_count
                );

                for (file, syms) in &by_file {
                    output.push_str(&format!("\n{}:\n", file));

                    for sym in syms {
                        if sym.parent_id.is_some() {
                            continue;
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

                        // Show children names (no signatures) for classes
                        if sym.kind == "class" {
                            let children: Vec<&&_> = syms
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
                        }
                    }

                    if output.len() > max_chars {
                        output.push_str("\n... (truncated)\n");
                        break;
                    }
                }

                Ok(output)
            }

            _ => {
                // "full": original behavior — symbol names + signatures + children
                let mut output = format!(
                    "Codebase: {} files, {} symbols, {} imports\n",
                    stats.file_count, stats.symbol_count, stats.import_count
                );

                for (file, syms) in &by_file {
                    output.push_str(&format!("\n{}:\n", file));

                    for sym in syms {
                        if sym.parent_id.is_some() {
                            continue;
                        }

                        output.push_str(&format!("  {}\n", sym.signature));

                        let children: Vec<&&_> = syms
                            .iter()
                            .filter(|s| s.parent_id == Some(sym.id))
                            .collect();
                        for child in children {
                            output.push_str(&format!("    {}\n", child.signature));
                        }
                    }

                    if output.len() > max_chars {
                        output.push_str("\n... (truncated)\n");
                        break;
                    }
                }

                Ok(output)
            }
        }
    }

    fn tool_find_symbol(&self, store: &Store, name: &str, kind: Option<&str>) -> Result<String> {
        let results = store.find_symbol(name, kind)?;
        if results.is_empty() {
            return Err(anyhow!("SYMBOL_NOT_FOUND: {}", name));
        }

        let mut output = String::new();
        for sym in &results {
            output.push_str(&format!("Name: {}\n", sym.qualified_name));
            output.push_str(&format!("Simple Name: {}\n", sym.name));
            output.push_str(&format!("Kind: {}\n", sym.kind));
            output.push_str(&format!("File: {}\n", sym.file_path));
            output.push_str(&format!("Visibility: {}\n", sym.visibility));
            output.push_str(&format!("Signature: {}\n", sym.signature));
            if let Some(ref doc) = sym.docstring {
                output.push_str(&format!("Docstring: {}\n", doc));
            }
            output.push_str(&format!(
                "Location: bytes {}..{}\n",
                sym.byte_start, sym.byte_end
            ));
            output.push('\n');
        }

        Ok(output)
    }

    fn tool_read_source(
        &self,
        store: &Store,
        symbol_name: &str,
        max_chars: usize,
        mode: &str,
    ) -> Result<String> {
        let results = store.find_symbol(symbol_name, None)?;
        if results.is_empty() {
            return Err(anyhow!("SYMBOL_NOT_FOUND: {}", symbol_name));
        }

        let sym = &results[0];

        // Skeleton mode: return signature + child signatures without source body
        if mode == "skeleton" {
            let file_symbols = store.get_file_symbols(&sym.file_path)?;
            let mut output = format!("// {} (skeleton)\n", sym.file_path);
            output.push_str(&sym.signature);
            output.push('\n');
            let children: Vec<&_> = file_symbols
                .iter()
                .filter(|s| s.parent_id == Some(sym.id))
                .collect();
            for child in &children {
                output.push_str(&format!("  {}\n", child.signature));
            }
            if children.is_empty() {
                output.push_str("  ...\n");
            }
            return Ok(output);
        }

        let file_path = self.root_path.join(&sym.file_path);
        let source = std::fs::read_to_string(&file_path)
            .map_err(|_| anyhow!("Cannot read source file: {}", sym.file_path))?;

        let start = sym.byte_start.min(source.len());
        let end = sym.byte_end.min(source.len());
        let fragment = &source[start..end];
        let total_chars = fragment.len();

        if total_chars <= max_chars {
            Ok(format!(
                "// {}:{}-{}\n{}",
                sym.file_path, sym.byte_start, sym.byte_end, fragment
            ))
        } else {
            let head_budget = max_chars * 3 / 4;
            let tail_budget = max_chars - head_budget;
            let head = &fragment[..head_budget.min(fragment.len())];
            let tail_start = fragment.len().saturating_sub(tail_budget);
            let tail = &fragment[tail_start..];
            let omitted = total_chars - head_budget - tail_budget;

            Ok(format!(
                "// {}:{}-{} ({} chars total, showing first {} + last {})\n{}\n\n// ... ({} chars omitted) ...\n\n{}",
                sym.file_path,
                sym.byte_start,
                sym.byte_end,
                total_chars,
                head_budget,
                tail_budget,
                head,
                omitted,
                tail
            ))
        }
    }

    fn tool_search_symbols(
        &self,
        store: &Store,
        query: &str,
        max_results: usize,
    ) -> Result<String> {
        let results = store.search_symbols(query, max_results)?;
        if results.is_empty() {
            return Ok("No matching symbols found.".to_string());
        }

        let mut output = String::new();
        for sym in &results {
            output.push_str(&format!(
                "[{}] {} ({}) — {}\n  {}\n",
                sym.kind, sym.qualified_name, sym.visibility, sym.file_path, sym.signature
            ));
        }

        Ok(output)
    }

    fn tool_get_skeleton(&self, store: &Store, file_path: &str) -> Result<String> {
        let symbols = store.get_file_symbols(file_path)?;
        if symbols.is_empty() {
            return Err(anyhow!("No symbols found for file: {}", file_path));
        }

        // Read source for import extraction
        let full_path = self.root_path.join(file_path);
        let source = std::fs::read_to_string(&full_path).unwrap_or_default();

        Ok(compressor::skeleton(
            &source,
            &symbols,
            self.config.compression.aggregate_imports,
        ))
    }

    fn tool_get_dependencies(
        &self,
        store: &Store,
        symbol: &str,
        direction: &str,
        max_results: usize,
    ) -> Result<String> {
        let all_symbols = store.get_all_symbols()?;
        let all_edges = store.get_all_edges()?;
        let graph = DepGraph::build(&all_symbols, &all_edges);
        let dir = Direction::parse(direction);
        let mut results: Vec<(String, String, String)> = graph
            .get_dependencies(symbol, dir)
            .into_iter()
            .map(|name| {
                let (kind, file) = all_symbols
                    .iter()
                    .find(|sym| sym.qualified_name == name || sym.name == name)
                    .map(|sym| (sym.kind.clone(), sym.file_path.clone()))
                    .unwrap_or_else(|| ("unknown".to_string(), "unknown".to_string()));
                (name, kind, file)
            })
            .collect();

        if results.is_empty() {
            return Ok(format!(
                "No {} dependencies found for: {}",
                direction, symbol
            ));
        }

        // Deduplicate
        results.sort();
        results.dedup();
        let total = results.len();
        let shown = results.iter().take(max_results);

        let mut output = format!(
            "Dependencies ({}) for {} ({} total):\n",
            direction, symbol, total
        );
        for (name, kind, file) in shown {
            output.push_str(&format!("  → {} [{}] — {}\n", name, kind, file));
        }
        if total > max_results {
            output.push_str(&format!("  ... and {} more\n", total - max_results));
        }

        Ok(output)
    }

    fn tool_get_impact(
        &self,
        store: &Store,
        symbol: &str,
        max_depth: usize,
        max_results: usize,
    ) -> Result<String> {
        let all_symbols = store.get_all_symbols()?;
        let all_edges = store.get_all_edges()?;
        let graph = DepGraph::build(&all_symbols, &all_edges);

        let impact = graph.get_impact_bounded(symbol, max_depth);

        if impact.is_empty() {
            return Ok(format!("No transitive dependents found for: {}", symbol));
        }

        // Group by file path for a compact summary
        let mut by_file: std::collections::BTreeMap<String, Vec<&str>> =
            std::collections::BTreeMap::new();
        for dep in &impact {
            // Look up file path from symbol metadata
            let file = graph.file_of(dep).unwrap_or("unknown");
            by_file.entry(file.to_string()).or_default().push(dep);
        }

        let total_symbols = impact.len();
        let total_files = by_file.len();

        let mut output = format!(
            "Blast radius for {} (depth ≤ {}):\n  {} affected symbols across {} files\n\n",
            symbol, max_depth, total_symbols, total_files
        );

        let mut shown = 0;
        for (file, syms) in &by_file {
            if shown >= max_results {
                break;
            }
            output.push_str(&format!("{}:\n", file));
            for s in syms {
                if shown >= max_results {
                    break;
                }
                output.push_str(&format!("  ⚡ {}\n", s));
                shown += 1;
            }
        }
        if total_symbols > max_results {
            output.push_str(&format!(
                "\n... and {} more symbols in {} files\n",
                total_symbols - shown,
                total_files - by_file.keys().take(shown).count()
            ));
        }

        Ok(output)
    }

    fn tool_trace_data_flow(
        &self,
        store: &Store,
        symbol: &str,
        max_depth: usize,
        max_results: usize,
    ) -> Result<String> {
        let all_symbols = store.get_all_symbols()?;
        let all_edges = store.get_all_edges()?;
        let graph = DepGraph::build(&all_symbols, &all_edges);

        let sym_meta: HashMap<&str, (&str, &str)> = all_symbols
            .iter()
            .map(|s| {
                (
                    s.qualified_name.as_str(),
                    (s.kind.as_str(), s.file_path.as_str()),
                )
            })
            .collect();

        let current_symbol = all_symbols
            .iter()
            .find(|sym| sym.qualified_name == symbol)
            .or_else(|| all_symbols.iter().find(|sym| sym.name == symbol));
        let (sym_kind, sym_file, sym_name) = current_symbol
            .map(|sym| {
                (
                    sym.kind.as_str(),
                    sym.file_path.as_str(),
                    sym.qualified_name.as_str(),
                )
            })
            .unwrap_or(("symbol", "unknown", symbol));
        let (upstream, downstream) = graph.trace_flow(symbol, max_depth);

        let mut output = format!(
            "Data flow trace for {} [{}] in {}:\n\n",
            sym_name, sym_kind, sym_file
        );

        // Upstream
        output.push_str(&format!(
            "⬆ UPSTREAM (callers, {} total):\n",
            upstream.len()
        ));
        if upstream.is_empty() {
            output.push_str("  (none — this is a root/entry point)\n");
        } else {
            for (shown, (name, depth)) in upstream.iter().enumerate() {
                if shown >= max_results {
                    break;
                }
                let (kind, file) = sym_meta.get(name.as_str()).copied().unwrap_or(("?", "?"));
                let indent = "  ".repeat(*depth);
                output.push_str(&format!("{}← {} [{}] — {}\n", indent, name, kind, file));
            }
            if upstream.len() > max_results {
                output.push_str(&format!(
                    "  ... and {} more\n",
                    upstream.len() - max_results
                ));
            }
        }

        output.push('\n');

        // Downstream
        output.push_str(&format!(
            "⬇ DOWNSTREAM (callees, {} total):\n",
            downstream.len()
        ));
        if downstream.is_empty() {
            output.push_str("  (none — this is a leaf/terminal)\n");
        } else {
            for (shown, (name, depth)) in downstream.iter().enumerate() {
                if shown >= max_results {
                    break;
                }
                let (kind, file) = sym_meta.get(name.as_str()).copied().unwrap_or(("?", "?"));
                let indent = "  ".repeat(*depth);
                output.push_str(&format!("{}→ {} [{}] — {}\n", indent, name, kind, file));
            }
            if downstream.len() > max_results {
                output.push_str(&format!(
                    "  ... and {} more\n",
                    downstream.len() - max_results
                ));
            }
        }

        Ok(output)
    }

    fn tool_get_ranked(
        &self,
        store: &Store,
        top_n: usize,
        kind: Option<&str>,
        path_filter: Option<&str>,
    ) -> Result<String> {
        let all_symbols = store.get_all_symbols()?;
        let all_edges = store.get_all_edges()?;
        let graph = DepGraph::build(&all_symbols, &all_edges);

        // Get more results than needed if we're filtering, then trim
        let fetch_n = if path_filter.is_some() {
            top_n * 10
        } else {
            top_n
        };
        let ranked = graph.get_ranked(fetch_n, kind);

        let filtered: Vec<_> = if let Some(prefix) = path_filter {
            ranked
                .into_iter()
                .filter(|s| s.file_path.starts_with(prefix))
                .take(top_n)
                .collect()
        } else {
            ranked.into_iter().take(top_n).collect()
        };

        if filtered.is_empty() {
            return Ok("No ranked symbols found.".to_string());
        }

        let mut output = format!("Top {} symbols by PageRank:\n", filtered.len());
        for (i, sym) in filtered.iter().enumerate() {
            output.push_str(&format!(
                "  {}. {} [{}] — {} (score: {:.6})\n",
                i + 1,
                sym.name,
                sym.kind,
                sym.file_path,
                sym.score
            ));
        }

        Ok(output)
    }

    fn tool_compress_context(&self, store: &Store, query: &str, budget: usize) -> Result<String> {
        let all_symbols = store.get_all_symbols()?;
        Ok(compressor::compress_context(&all_symbols, query, budget))
    }

    fn tool_pack_repo(&self, store: &Store, budget: usize, format: &str) -> Result<String> {
        let all_symbols = store.get_all_symbols()?;
        Ok(compressor::pack_repo(&all_symbols, budget, format))
    }

    fn tool_search_imports(
        &self,
        store: &Store,
        query: &str,
        max_results: usize,
    ) -> Result<String> {
        let results = store.search_imports(query, max_results)?;
        if results.is_empty() {
            return Ok(format!("No imports matching: {}", query));
        }

        // Group by file for compact output
        let mut by_file: std::collections::BTreeMap<&str, Vec<&crate::store::ImportRecord>> =
            std::collections::BTreeMap::new();
        for imp in &results {
            by_file.entry(&imp.file_path).or_default().push(imp);
        }

        let mut output = format!(
            "{} imports across {} files:\n\n",
            results.len(),
            by_file.len()
        );
        for (file, imps) in &by_file {
            output.push_str(&format!("{}:\n", file));
            for imp in imps {
                output.push_str(&format!("  {}\n", imp.raw_text));
            }
            output.push('\n');
        }

        Ok(output)
    }

    fn tool_index_status(&self, store: &Store) -> Result<String> {
        let stats = store.get_stats()?;
        let mut output = String::new();
        output.push_str(&format!("Files indexed: {}\n", stats.file_count));
        output.push_str(&format!("Symbols: {}\n", stats.symbol_count));
        output.push_str(&format!("Imports tracked: {}\n", stats.import_count));
        output.push_str(&format!("References tracked: {}\n", stats.reference_count));
        output.push_str(&format!("Resolved edges: {}\n", stats.edge_count));
        output.push_str(&format!("Languages: {}\n", stats.languages.join(", ")));
        output.push_str(&format!(
            "Database: {}\n",
            Config::db_path(&self.root_path).display()
        ));
        Ok(output)
    }

    fn tool_reindex(&self, store: &Store, path: Option<&str>) -> Result<String> {
        let message = if let Some(p) = path {
            let indexed = indexer::index_file(&self.root_path, Path::new(p), store, &self.config)?;
            if indexed {
                format!("Re-indexed: {}", p)
            } else {
                format!("Re-index skipped: {} unchanged.", p)
            }
        } else {
            let summary = indexer::index_directory(&self.root_path, store, &self.config)?;
            format!(
                "Re-indexed {} files ({} unchanged, {} scanned).",
                summary.indexed, summary.unchanged, summary.total_files
            )
        };

        Ok(message)
    }

    fn tool_definitions(&self) -> Vec<Value> {
        vec![
            json!({
                "name": "map_overview",
                "description": "Get a map of the codebase. Use detail='tree' for a compact directory overview (~2K tokens for any codebase), 'summary' for file paths + symbol names (~15K tokens for large codebases), or 'full' for complete signatures.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "max_chars": { "type": "integer", "default": 120000, "description": "Maximum characters in the output" },
                        "detail": { "type": "string", "enum": ["tree","summary","full"], "default": "summary", "description": "Level of detail: tree (dirs only), summary (symbol names), full (signatures)" }
                    }
                }
            }),
            json!({
                "name": "find_symbol",
                "description": "Find a symbol by exact name. Returns signature, location, and docstring.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Exact symbol name to find" },
                        "kind": { "type": "string", "enum": ["function","class","method","type","const","interface"], "description": "Optional: filter by symbol kind" }
                    },
                    "required": ["name"]
                }
            }),
            json!({
                "name": "read_source",
                "description": "Read the source code of a specific symbol. Use mode='skeleton' for large symbols to get just signatures (~100 tokens vs 20K+ chars).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Symbol name to read" },
                        "max_chars": { "type": "integer", "default": 20000, "description": "Maximum characters for 'full' mode. Large symbols show head + tail." },
                        "mode": { "type": "string", "enum": ["full", "skeleton"], "default": "full", "description": "full = source code (truncated if large), skeleton = signatures only" }
                    },
                    "required": ["symbol"]
                }
            }),
            json!({
                "name": "search_symbols",
                "description": "Fuzzy search for symbols by name, signature, or docstring. Supports multi-word queries (e.g. 'auth login') by matching all terms.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "max_results": { "type": "integer", "default": 10, "description": "Maximum results" },
                        "max_tokens": { "type": "integer", "default": 2000, "description": "Maximum tokens in response" }
                    },
                    "required": ["query"]
                }
            }),
            json!({
                "name": "get_skeleton",
                "description": "Return a skeletonized view of a file: signatures only, bodies collapsed, imports aggregated.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string", "description": "Relative path to the source file" }
                    },
                    "required": ["file_path"]
                }
            }),
            json!({
                "name": "get_dependencies",
                "description": "Get direct dependencies or dependents of a symbol, sorted by PageRank importance.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Symbol name" },
                        "direction": { "type": "string", "enum": ["in","out","both"], "default": "both", "description": "Direction of dependencies" },
                        "max_results": { "type": "integer", "default": 20, "description": "Maximum results to return" }
                    },
                    "required": ["symbol"]
                }
            }),
            json!({
                "name": "get_impact",
                "description": "Compute the blast radius of changing a symbol. Groups affected symbols by file with a summary count.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Symbol name to analyze" },
                        "max_depth": { "type": "integer", "default": 3, "description": "Maximum traversal depth (hops). Use 1 for direct dependents only" },
                        "max_results": { "type": "integer", "default": 30, "description": "Maximum symbols to list (summary count always shown)" }
                    },
                    "required": ["symbol"]
                }
            }),
            json!({
                "name": "trace_data_flow",
                "description": "Trace a symbol's data flow: upstream callers and downstream callees across files. Shows the call chain for understanding request paths and data pipelines.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Symbol name to trace" },
                        "max_depth": { "type": "integer", "default": 3, "description": "Maximum traversal depth (hops)" },
                        "max_results": { "type": "integer", "default": 20, "description": "Maximum symbols per direction" }
                    },
                    "required": ["symbol"]
                }
            }),
            json!({
                "name": "get_ranked_symbols",
                "description": "Get top N symbols ranked by PageRank importance. Use 'path' to scope to a directory (e.g. 'backend-service/src/services/').",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "top_n": { "type": "integer", "default": 10, "description": "Number of top symbols" },
                        "kind": { "type": "string", "enum": ["function","class","method","type","const","interface"], "description": "Optional: filter by kind" },
                        "path": { "type": "string", "description": "Optional: filter to symbols in files under this path prefix" }
                    }
                }
            }),
            json!({
                "name": "compress_context",
                "description": "Return maximally relevant compressed code for a query within a strict token budget.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Natural language or identifier query" },
                        "token_budget": { "type": "integer", "default": 32000, "description": "Max tokens in response" }
                    },
                    "required": ["query"]
                }
            }),
            json!({
                "name": "pack_repo",
                "description": "Pack entire repo into a single artifact within a token budget.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "token_budget": { "type": "integer", "default": 32000, "description": "Max tokens" },
                        "format": { "type": "string", "enum": ["xml","json"], "default": "xml", "description": "Output format" }
                    }
                }
            }),
            json!({
                "name": "search_imports",
                "description": "Search import/require/use statements across all indexed files. Find which files import a specific module or package (e.g. 'ai/react', 'stripe', 'prisma').",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query for import text or module name" },
                        "max_results": { "type": "integer", "default": 30, "description": "Maximum results" }
                    },
                    "required": ["query"]
                }
            }),
            json!({
                "name": "index_status",
                "description": "Show index health: file count, symbol count, last update.",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            }),
            json!({
                "name": "reindex",
                "description": "Force re-index of a specific file or the entire repository.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Optional: specific file to re-index" }
                    }
                }
            }),
        ]
    }
}

fn jsonrpc_result(id: Option<Value>, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "result": result
    })
}
