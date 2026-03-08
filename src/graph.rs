use std::collections::{HashMap, HashSet, VecDeque};

use crate::store::{EdgeRecord, SymbolRecord};

/// A traced symbol with its depth from the origin.
pub type TraceEntry = (String, usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
    Both,
}

impl Direction {
    pub fn parse(s: &str) -> Self {
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

#[derive(Debug, Clone)]
struct SymbolMeta {
    qualified_name: String,
    kind: String,
    file_path: String,
}

pub struct DepGraph {
    /// node_id -> list of nodes it depends on (outgoing edges: A uses B)
    outgoing: HashMap<i64, Vec<i64>>,
    /// node_id -> list of nodes that depend on it (incoming edges: B is used by A)
    incoming: HashMap<i64, Vec<i64>>,
    /// PageRank scores
    pagerank: HashMap<i64, f64>,
    /// Symbol metadata for ranking and output
    symbol_meta: HashMap<i64, SymbolMeta>,
    /// Resolve exact scoped symbols without ambiguity.
    qualified_index: HashMap<String, i64>,
    /// Preserve bare-name lookup as a fallback.
    name_index: HashMap<String, Vec<i64>>,
}

impl Default for DepGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl DepGraph {
    pub fn new() -> Self {
        Self {
            outgoing: HashMap::new(),
            incoming: HashMap::new(),
            pagerank: HashMap::new(),
            symbol_meta: HashMap::new(),
            qualified_index: HashMap::new(),
            name_index: HashMap::new(),
        }
    }

    /// Build the dependency graph from stored symbols and resolved edges.
    pub fn build(symbols: &[SymbolRecord], edges: &[EdgeRecord]) -> Self {
        let mut graph = Self::new();

        for sym in symbols {
            graph.symbol_meta.insert(
                sym.id,
                SymbolMeta {
                    qualified_name: sym.qualified_name.clone(),
                    kind: sym.kind.clone(),
                    file_path: sym.file_path.clone(),
                },
            );
            graph
                .qualified_index
                .insert(sym.qualified_name.clone(), sym.id);
            graph
                .name_index
                .entry(sym.name.clone())
                .or_default()
                .push(sym.id);
            graph.outgoing.entry(sym.id).or_default();
            graph.incoming.entry(sym.id).or_default();
        }

        for edge in edges {
            if !(graph.symbol_meta.contains_key(&edge.source_symbol_id)
                && graph.symbol_meta.contains_key(&edge.target_symbol_id))
            {
                continue;
            }
            graph
                .outgoing
                .entry(edge.source_symbol_id)
                .or_default()
                .push(edge.target_symbol_id);
            graph
                .incoming
                .entry(edge.target_symbol_id)
                .or_default()
                .push(edge.source_symbol_id);
        }

        for neighbors in graph.outgoing.values_mut() {
            neighbors.sort_unstable();
            neighbors.dedup();
        }
        for neighbors in graph.incoming.values_mut() {
            neighbors.sort_unstable();
            neighbors.dedup();
        }

        graph.compute_pagerank(20, 0.85);
        graph
    }

    /// Compute PageRank scores using iterative power method.
    fn compute_pagerank(&mut self, iterations: usize, damping: f64) {
        let all_nodes: Vec<i64> = self.symbol_meta.keys().copied().collect();
        if all_nodes.is_empty() {
            return;
        }
        let nf = all_nodes.len() as f64;

        let mut scores: HashMap<i64, f64> =
            all_nodes.iter().copied().map(|id| (id, 1.0 / nf)).collect();

        for _ in 0..iterations {
            let mut new_scores = HashMap::with_capacity(all_nodes.len());

            for node in &all_nodes {
                let mut rank = (1.0 - damping) / nf;
                if let Some(incomers) = self.incoming.get(node) {
                    for incomer in incomers {
                        let out_degree = self
                            .outgoing
                            .get(incomer)
                            .map(|neighbors| neighbors.len())
                            .unwrap_or(1)
                            .max(1) as f64;
                        let incomer_score = scores.get(incomer).copied().unwrap_or(0.0);
                        rank += damping * incomer_score / out_degree;
                    }
                }
                new_scores.insert(*node, rank);
            }

            scores = new_scores;
        }

        self.pagerank = scores;
    }

    /// Get direct dependencies or dependents of a symbol.
    pub fn get_dependencies(&self, symbol: &str, direction: Direction) -> Vec<String> {
        let ids = self.symbol_ids(symbol);
        let mut result_ids = HashSet::new();

        for id in ids {
            match direction {
                Direction::Out => self.extend_neighbors(&mut result_ids, &self.outgoing, id),
                Direction::In => self.extend_neighbors(&mut result_ids, &self.incoming, id),
                Direction::Both => {
                    self.extend_neighbors(&mut result_ids, &self.outgoing, id);
                    self.extend_neighbors(&mut result_ids, &self.incoming, id);
                }
            }
        }

        self.names_for_ids(result_ids.into_iter())
    }

    /// Compute blast radius: all transitive dependents (who would be affected
    /// if this symbol changes).
    pub fn get_impact(&self, symbol: &str) -> Vec<String> {
        self.get_impact_bounded(symbol, usize::MAX)
    }

    /// Compute blast radius with a depth limit.
    pub fn get_impact_bounded(&self, symbol: &str, max_depth: usize) -> Vec<String> {
        let start_ids = self.symbol_ids(symbol);
        if start_ids.is_empty() {
            return Vec::new();
        }

        let visited = self.bfs_ids(&start_ids, max_depth, &self.incoming);
        let mut impacted = visited;
        for start_id in &start_ids {
            impacted.remove(start_id);
        }

        self.names_for_ids(impacted.into_iter())
    }

    /// Look up the file path for a symbol.
    pub fn file_of(&self, symbol: &str) -> Option<&str> {
        self.resolve_symbol_ids(symbol)
            .and_then(|ids| ids.first().copied())
            .and_then(|id| self.symbol_meta.get(&id))
            .map(|meta| meta.file_path.as_str())
    }

    /// Trace data flow: follow a symbol upstream (callers) and downstream (callees)
    /// returning the chain with depth info, sorted by direction then rank.
    pub fn trace_flow(&self, symbol: &str, max_depth: usize) -> (Vec<TraceEntry>, Vec<TraceEntry>) {
        let start_ids = self.symbol_ids(symbol);
        if start_ids.is_empty() {
            return (Vec::new(), Vec::new());
        }

        let upstream = self.trace_direction(&start_ids, max_depth, &self.incoming);
        let downstream = self.trace_direction(&start_ids, max_depth, &self.outgoing);
        (upstream, downstream)
    }

    /// Look up the kind for a symbol.
    pub fn kind_of(&self, symbol: &str) -> Option<&str> {
        self.resolve_symbol_ids(symbol)
            .and_then(|ids| ids.first().copied())
            .and_then(|id| self.symbol_meta.get(&id))
            .map(|meta| meta.kind.as_str())
    }

    /// Get symbols related within N hops.
    pub fn related(&self, symbol: &str, depth: usize) -> Vec<String> {
        let start_ids = self.symbol_ids(symbol);
        if start_ids.is_empty() {
            return Vec::new();
        }

        let mut related = self.bfs_ids(&start_ids, depth, &self.outgoing);
        related.extend(self.bfs_ids(&start_ids, depth, &self.incoming));
        for start_id in &start_ids {
            related.remove(start_id);
        }

        self.names_for_ids(related.into_iter())
    }

    /// Get top N symbols ranked by PageRank.
    pub fn get_ranked(&self, top_n: usize, kind: Option<&str>) -> Vec<RankedSymbol> {
        let mut ranked: Vec<(i64, f64)> = self
            .pagerank
            .iter()
            .map(|(id, score)| (*id, *score))
            .collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        ranked
            .into_iter()
            .filter(|(id, _)| {
                if let Some(kind_filter) = kind {
                    self.symbol_meta
                        .get(id)
                        .map(|meta| meta.kind == kind_filter)
                        .unwrap_or(false)
                } else {
                    true
                }
            })
            .take(top_n)
            .filter_map(|(id, score)| {
                self.symbol_meta.get(&id).map(|meta| RankedSymbol {
                    name: meta.qualified_name.clone(),
                    kind: meta.kind.clone(),
                    file_path: meta.file_path.clone(),
                    score,
                })
            })
            .collect()
    }

    /// Get the PageRank score of a symbol.
    pub fn score(&self, symbol: &str) -> f64 {
        self.resolve_symbol_ids(symbol)
            .into_iter()
            .flatten()
            .filter_map(|id| self.pagerank.get(&id).copied())
            .fold(0.0, f64::max)
    }

    fn symbol_ids(&self, symbol: &str) -> Vec<i64> {
        self.resolve_symbol_ids(symbol).unwrap_or_default()
    }

    fn resolve_symbol_ids(&self, symbol: &str) -> Option<Vec<i64>> {
        if let Some(id) = self.qualified_index.get(symbol) {
            return Some(vec![*id]);
        }
        self.name_index.get(symbol).cloned()
    }

    fn extend_neighbors(&self, out: &mut HashSet<i64>, graph: &HashMap<i64, Vec<i64>>, id: i64) {
        if let Some(neighbors) = graph.get(&id) {
            out.extend(neighbors.iter().copied());
        }
    }

    fn names_for_ids<I>(&self, ids: I) -> Vec<String>
    where
        I: IntoIterator<Item = i64>,
    {
        let mut names: Vec<String> = ids
            .into_iter()
            .filter_map(|id| {
                self.symbol_meta
                    .get(&id)
                    .map(|meta| meta.qualified_name.clone())
            })
            .collect();
        names.sort();
        names.dedup();
        names
    }

    fn bfs_ids(
        &self,
        start_ids: &[i64],
        max_depth: usize,
        graph: &HashMap<i64, Vec<i64>>,
    ) -> HashSet<i64> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        for start_id in start_ids {
            queue.push_back((*start_id, 0usize));
        }

        while let Some((current, depth)) = queue.pop_front() {
            if !visited.insert(current) || depth >= max_depth {
                continue;
            }
            if let Some(neighbors) = graph.get(&current) {
                for neighbor in neighbors {
                    if !visited.contains(neighbor) {
                        queue.push_back((*neighbor, depth + 1));
                    }
                }
            }
        }

        visited
    }

    fn trace_direction(
        &self,
        start_ids: &[i64],
        max_depth: usize,
        graph: &HashMap<i64, Vec<i64>>,
    ) -> Vec<TraceEntry> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut by_name: HashMap<String, usize> = HashMap::new();

        for start_id in start_ids {
            visited.insert(*start_id);
            queue.push_back((*start_id, 0usize));
        }

        while let Some((current, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            if let Some(neighbors) = graph.get(&current) {
                for neighbor in neighbors {
                    if let Some(meta) = self.symbol_meta.get(neighbor) {
                        let entry = by_name
                            .entry(meta.qualified_name.clone())
                            .or_insert(depth + 1);
                        *entry = (*entry).min(depth + 1);
                    }
                    if visited.insert(*neighbor) {
                        queue.push_back((*neighbor, depth + 1));
                    }
                }
            }
        }

        let mut traced: Vec<TraceEntry> = by_name.into_iter().collect();
        traced.sort_by(|a, b| {
            a.1.cmp(&b.1).then_with(|| {
                let sa = self.score(&a.0);
                let sb = self.score(&b.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        traced
    }
}

#[cfg(test)]
mod tests {
    use super::{DepGraph, Direction};
    use crate::store::{EdgeRecord, SymbolRecord};

    fn symbol(id: i64, file_path: &str, name: &str) -> SymbolRecord {
        SymbolRecord {
            id,
            file_path: file_path.to_string(),
            name: name.to_string(),
            qualified_name: format!("{file_path}::{name}"),
            kind: "function".to_string(),
            visibility: "public".to_string(),
            signature: format!("fn {name}()"),
            docstring: None,
            byte_start: 0,
            byte_end: 10,
            parent_id: None,
        }
    }

    #[test]
    fn graph_uses_resolved_edges() {
        let symbols = vec![
            symbol(1, "a.py", "alpha"),
            symbol(2, "b.py", "beta"),
            symbol(3, "c.py", "gamma"),
        ];
        let edges = vec![
            EdgeRecord {
                source_symbol_id: 1,
                target_symbol_id: 2,
                context: "call".to_string(),
            },
            EdgeRecord {
                source_symbol_id: 2,
                target_symbol_id: 3,
                context: "call".to_string(),
            },
        ];

        let graph = DepGraph::build(&symbols, &edges);

        assert_eq!(
            graph.get_dependencies("alpha", Direction::Out),
            vec!["b.py::beta"]
        );
        assert_eq!(graph.get_impact("gamma"), vec!["a.py::alpha", "b.py::beta"]);
        assert_eq!(
            graph.trace_flow("alpha", 2).1,
            vec![
                ("b.py::beta".to_string(), 1),
                ("c.py::gamma".to_string(), 2)
            ]
        );
    }
}
