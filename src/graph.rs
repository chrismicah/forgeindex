use std::collections::{HashMap, HashSet, VecDeque};

use crate::store::{ImportRecord, SymbolRecord};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
    Both,
}

impl Direction {
    pub fn from_str(s: &str) -> Self {
        match s {
            "in" => Direction::In,
            "out" => Direction::Out,
            _ => Direction::Both,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RankedSymbol {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub score: f64,
}

pub struct DepGraph {
    /// node -> list of nodes it depends on (outgoing edges: A uses B)
    outgoing: HashMap<String, Vec<String>>,
    /// node -> list of nodes that depend on it (incoming edges: B is used by A)
    incoming: HashMap<String, Vec<String>>,
    /// PageRank scores
    pagerank: HashMap<String, f64>,
    /// Symbol metadata for ranking
    symbol_meta: HashMap<String, (String, String)>, // name -> (kind, file_path)
}

impl DepGraph {
    pub fn new() -> Self {
        Self {
            outgoing: HashMap::new(),
            incoming: HashMap::new(),
            pagerank: HashMap::new(),
            symbol_meta: HashMap::new(),
        }
    }

    /// Build the dependency graph from symbols and imports.
    pub fn build(symbols: &[SymbolRecord], imports: &[ImportRecord]) -> Self {
        let mut graph = Self::new();

        // Register all symbols as nodes
        let mut symbol_names: HashSet<String> = HashSet::new();
        for sym in symbols {
            symbol_names.insert(sym.name.clone());
            graph
                .symbol_meta
                .insert(sym.name.clone(), (sym.kind.clone(), sym.file_path.clone()));
            graph
                .outgoing
                .entry(sym.name.clone())
                .or_insert_with(Vec::new);
            graph
                .incoming
                .entry(sym.name.clone())
                .or_insert_with(Vec::new);
        }

        // Build edges from imports: if file A imports name X, and X is a known
        // symbol, then all symbols in file A depend on X.
        let mut file_symbols: HashMap<String, Vec<String>> = HashMap::new();
        for sym in symbols {
            file_symbols
                .entry(sym.file_path.clone())
                .or_default()
                .push(sym.name.clone());
        }

        for imp in imports {
            // Try to find imported names in our symbol index
            if let Some(ref source_mod) = imp.source_module {
                // Check if source_module matches any symbol name or file path
                let matching_symbols: Vec<String> = symbols
                    .iter()
                    .filter(|s| {
                        s.file_path.contains(source_mod)
                            || s.name == *source_mod
                            || source_mod.ends_with(&s.name)
                    })
                    .map(|s| s.name.clone())
                    .collect();

                if !matching_symbols.is_empty() {
                    // All symbols in the importing file depend on the imported symbols
                    if let Some(file_syms) = file_symbols.get(&imp.file_path) {
                        for src_sym in file_syms {
                            for target in &matching_symbols {
                                if src_sym != target {
                                    graph
                                        .outgoing
                                        .entry(src_sym.clone())
                                        .or_default()
                                        .push(target.clone());
                                    graph
                                        .incoming
                                        .entry(target.clone())
                                        .or_default()
                                        .push(src_sym.clone());
                                }
                            }
                        }
                    }
                }
            }

            // Also try matching imported names directly to known symbols
            let raw_parts: Vec<&str> = imp.raw_text.split_whitespace().collect();
            for part in raw_parts {
                let clean = part.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
                if symbol_names.contains(clean) {
                    if let Some(file_syms) = file_symbols.get(&imp.file_path) {
                        for src_sym in file_syms {
                            if src_sym != clean {
                                graph
                                    .outgoing
                                    .entry(src_sym.clone())
                                    .or_default()
                                    .push(clean.to_string());
                                graph
                                    .incoming
                                    .entry(clean.to_string())
                                    .or_default()
                                    .push(src_sym.clone());
                            }
                        }
                    }
                }
            }
        }

        // Deduplicate edges
        for edges in graph.outgoing.values_mut() {
            edges.sort();
            edges.dedup();
        }
        for edges in graph.incoming.values_mut() {
            edges.sort();
            edges.dedup();
        }

        // Compute PageRank
        graph.compute_pagerank(20, 0.85);
        graph
    }

    /// Compute PageRank scores using iterative power method.
    fn compute_pagerank(&mut self, iterations: usize, damping: f64) {
        let all_nodes: HashSet<String> = self
            .outgoing
            .keys()
            .chain(self.incoming.keys())
            .cloned()
            .collect();

        let n = all_nodes.len();
        if n == 0 {
            return;
        }
        let nf = n as f64;

        let mut scores: HashMap<String, f64> = all_nodes
            .iter()
            .map(|name| (name.clone(), 1.0 / nf))
            .collect();

        for _ in 0..iterations {
            let mut new_scores: HashMap<String, f64> = HashMap::new();

            for node in &all_nodes {
                let mut rank = (1.0 - damping) / nf;

                if let Some(incomers) = self.incoming.get(node) {
                    for incomer in incomers {
                        let out_degree = self
                            .outgoing
                            .get(incomer)
                            .map(|v| v.len())
                            .unwrap_or(1)
                            .max(1) as f64;
                        let incomer_score = scores.get(incomer).copied().unwrap_or(0.0);
                        rank += damping * incomer_score / out_degree;
                    }
                }

                new_scores.insert(node.clone(), rank);
            }

            scores = new_scores;
        }

        self.pagerank = scores;
    }

    /// Get direct dependencies or dependents of a symbol.
    pub fn get_dependencies(&self, symbol: &str, direction: Direction) -> Vec<String> {
        match direction {
            Direction::Out => self.outgoing.get(symbol).cloned().unwrap_or_default(),
            Direction::In => self.incoming.get(symbol).cloned().unwrap_or_default(),
            Direction::Both => {
                let mut result: Vec<String> =
                    self.outgoing.get(symbol).cloned().unwrap_or_default();
                let incoming = self.incoming.get(symbol).cloned().unwrap_or_default();
                result.extend(incoming);
                result.sort();
                result.dedup();
                result
            }
        }
    }

    /// Compute blast radius: all transitive dependents (who would be affected
    /// if this symbol changes).
    pub fn get_impact(&self, symbol: &str) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(symbol.to_string());

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current.clone()) {
                continue;
            }
            if let Some(dependents) = self.incoming.get(&current) {
                for dep in dependents {
                    if !visited.contains(dep) {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }

        visited.remove(symbol);
        let mut result: Vec<String> = visited.into_iter().collect();
        result.sort();
        result
    }

    /// Get symbols related within N hops.
    pub fn related(&self, symbol: &str, depth: usize) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back((symbol.to_string(), 0usize));

        while let Some((current, d)) = queue.pop_front() {
            if d > depth || !visited.insert(current.clone()) {
                continue;
            }
            // Traverse both directions
            for neighbor in self.get_dependencies(&current, Direction::Both) {
                if !visited.contains(&neighbor) {
                    queue.push_back((neighbor, d + 1));
                }
            }
        }

        visited.remove(symbol);
        let mut result: Vec<String> = visited.into_iter().collect();
        result.sort();
        result
    }

    /// Get top N symbols ranked by PageRank.
    pub fn get_ranked(&self, top_n: usize, kind: Option<&str>) -> Vec<RankedSymbol> {
        let mut ranked: Vec<(&String, &f64)> = self.pagerank.iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));

        ranked
            .into_iter()
            .filter(|(name, _)| {
                if let Some(k) = kind {
                    self.symbol_meta
                        .get(*name)
                        .map(|(sk, _)| sk == k)
                        .unwrap_or(false)
                } else {
                    true
                }
            })
            .take(top_n)
            .map(|(name, score)| {
                let (kind, file_path) = self
                    .symbol_meta
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| ("unknown".into(), "unknown".into()));
                RankedSymbol {
                    name: name.clone(),
                    kind,
                    file_path,
                    score: *score,
                }
            })
            .collect()
    }

    /// Get the PageRank score of a symbol.
    pub fn score(&self, symbol: &str) -> f64 {
        self.pagerank.get(symbol).copied().unwrap_or(0.0)
    }
}
