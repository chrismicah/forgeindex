use std::collections::HashMap;

use crate::store::SymbolRecord;

/// Estimate token count: ~4 characters per token on average for code.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Generate a skeleton view of a file: signatures only, bodies collapsed.
pub fn skeleton(source: &str, symbols: &[SymbolRecord], aggregate_imports: bool) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut output = String::new();

    // Collect import lines at top
    if aggregate_imports {
        let mut import_lines = Vec::new();
        for line in &lines {
            let trimmed = line.trim();
            if trimmed.starts_with("import ")
                || trimmed.starts_with("from ")
                || trimmed.starts_with("use ")
                || trimmed.starts_with("#include")
                || trimmed.starts_with("require")
                || trimmed.starts_with("package ")
            {
                import_lines.push(*line);
            }
        }
        if !import_lines.is_empty() {
            for line in &import_lines {
                output.push_str(line);
                output.push('\n');
            }
            output.push('\n');
        }
    }

    // Group symbols by top-level (no parent)
    let top_level: Vec<&SymbolRecord> = symbols.iter().filter(|s| s.parent_id.is_none()).collect();
    let children: Vec<&SymbolRecord> = symbols.iter().filter(|s| s.parent_id.is_some()).collect();

    for sym in &top_level {
        output.push_str(&sym.signature);
        output.push('\n');

        // Find children of this symbol
        let kids: Vec<&&SymbolRecord> = children
            .iter()
            .filter(|c| c.parent_id == Some(sym.id))
            .collect();

        if !kids.is_empty() {
            for kid in &kids {
                output.push_str("  ");
                output.push_str(&kid.signature);
                output.push('\n');
            }
        }

        // Add body placeholder if this is a function/method/class with a body
        match sym.kind.as_str() {
            "function" | "method" => {
                output.push_str("  ...\n");
            }
            "class" => {
                if kids.is_empty() {
                    output.push_str("  ...\n");
                }
            }
            _ => {}
        }
        output.push('\n');
    }

    output
}

/// TF-IDF scoring for symbols against a query, with file-path and multi-term boosting.
pub fn tfidf_rank(symbols: &[SymbolRecord], query: &str) -> Vec<(usize, f64)> {
    let query_terms = tokenize(query);
    if query_terms.is_empty() {
        return symbols.iter().enumerate().map(|(i, _)| (i, 0.0)).collect();
    }

    let total_docs = symbols.len() as f64;

    // Tokenize each symbol: name + signature + docstring + file path
    let mut df: HashMap<String, usize> = HashMap::new();
    let symbol_tokens: Vec<Vec<String>> = symbols
        .iter()
        .map(|s| {
            let mut text = s.name.clone();
            text.push(' ');
            text.push_str(&s.qualified_name);
            text.push(' ');
            text.push_str(&s.signature);
            text.push(' ');
            text.push_str(&s.file_path);
            if let Some(ref doc) = s.docstring {
                text.push(' ');
                text.push_str(doc);
            }
            tokenize(&text)
        })
        .collect();

    for tokens in &symbol_tokens {
        let unique: std::collections::HashSet<&str> = tokens.iter().map(|s| s.as_str()).collect();
        for term in unique {
            *df.entry(term.to_string()).or_insert(0) += 1;
        }
    }

    // Compute TF-IDF score for each symbol
    let mut scores: Vec<(usize, f64)> = Vec::new();
    for (i, tokens) in symbol_tokens.iter().enumerate() {
        let mut score = 0.0f64;
        let doc_len = tokens.len().max(1) as f64;

        let mut terms_matched = 0usize;
        for qt in &query_terms {
            // Skip very short terms (<=2 chars) in fuzzy TF-IDF to avoid
            // "ai" matching "trait", "email", etc. Still allow exact name matches below.
            if qt.len() <= 2 {
                // Only count if it's an exact token match (not substring)
                let exact_count = tokens.iter().filter(|t| *t == qt).count();
                if exact_count > 0 {
                    terms_matched += 1;
                    score += exact_count as f64 / doc_len;
                }
                continue;
            }
            let tf = tokens.iter().filter(|t| *t == qt).count() as f64 / doc_len;
            let idf = if let Some(&d) = df.get(qt) {
                (total_docs / (d as f64 + 1.0)).ln() + 1.0
            } else {
                0.0
            };
            let term_score = tf * idf;
            if term_score > 0.0 {
                terms_matched += 1;
            }
            score += term_score;
        }

        // Boost exact name matches
        let name_lower = symbols[i].name.to_lowercase();
        let qualified_name_lower = symbols[i].qualified_name.to_lowercase();
        for qt in &query_terms {
            if qualified_name_lower == *qt {
                score += 6.0;
            } else if name_lower == *qt {
                score += 5.0;
            } else if qt.len() > 2 && qualified_name_lower.contains(qt.as_str()) {
                score += 2.5;
            } else if qt.len() > 2 && name_lower.contains(qt.as_str()) {
                score += 2.0;
            }
        }

        // Boost file path matches (symbols in relevant directories)
        let path_lower = symbols[i].file_path.to_lowercase();
        for qt in &query_terms {
            if qt.len() <= 2 {
                continue; // Skip short terms for path matching
            }
            if path_lower.contains(qt.as_str()) {
                score += 1.5;
            }
        }

        // Multi-term coverage bonus: symbols matching MORE query terms rank higher.
        // This reduces tangential matches that only hit one common word.
        if query_terms.len() > 1 && terms_matched > 1 {
            let coverage = terms_matched as f64 / query_terms.len() as f64;
            score *= 1.0 + coverage;
        }

        scores.push((i, score));
    }

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores
}

/// Greedy knapsack: select symbols within token budget, ranked by value density.
pub fn greedy_knapsack<'a>(
    symbols: &'a [SymbolRecord],
    ranked: &[(usize, f64)],
    token_budget: usize,
) -> Vec<&'a SymbolRecord> {
    let mut selected = Vec::new();
    let mut remaining = token_budget;

    for &(idx, score) in ranked {
        if score <= 0.0 {
            continue;
        }
        let sym = &symbols[idx];
        let cost = estimate_tokens(&sym.signature);
        if cost <= remaining {
            remaining -= cost;
            selected.push(sym);
        }
        if remaining == 0 {
            break;
        }
    }

    selected
}

/// Compress context: TF-IDF rank + greedy knapsack, formatted output.
pub fn compress_context(symbols: &[SymbolRecord], query: &str, token_budget: usize) -> String {
    let ranked = tfidf_rank(symbols, query);
    let selected = greedy_knapsack(symbols, &ranked, token_budget);

    // Group by file
    let mut by_file: std::collections::BTreeMap<&str, Vec<&SymbolRecord>> =
        std::collections::BTreeMap::new();
    for sym in &selected {
        by_file.entry(&sym.file_path).or_default().push(sym);
    }

    let mut output = String::new();
    for (file, syms) in &by_file {
        output.push_str("// ");
        output.push_str(file);
        output.push('\n');
        for sym in syms {
            output.push_str(&sym.signature);
            output.push('\n');
        }
        output.push('\n');
    }

    output
}

/// Pack entire repo into a single artifact within token budget.
pub fn pack_repo(symbols: &[SymbolRecord], token_budget: usize, format: &str) -> String {
    // Group by file
    let mut by_file: std::collections::BTreeMap<&str, Vec<&SymbolRecord>> =
        std::collections::BTreeMap::new();
    for sym in symbols {
        if sym.parent_id.is_none() {
            by_file.entry(&sym.file_path).or_default().push(sym);
        }
    }

    match format {
        "json" => pack_json(&by_file, token_budget),
        _ => pack_xml(&by_file, token_budget),
    }
}

fn pack_xml(
    by_file: &std::collections::BTreeMap<&str, Vec<&SymbolRecord>>,
    token_budget: usize,
) -> String {
    let mut output = String::from("<repo>\n");
    let mut used_tokens = estimate_tokens(&output);

    for (file, syms) in by_file {
        let mut file_block = format!("<file path=\"{}\">\n", file);
        for sym in syms {
            file_block.push_str(&sym.signature);
            file_block.push('\n');
        }
        file_block.push_str("</file>\n");

        let cost = estimate_tokens(&file_block);
        if used_tokens + cost > token_budget {
            break;
        }
        output.push_str(&file_block);
        used_tokens += cost;
    }

    output.push_str("</repo>\n");
    output
}

fn pack_json(
    by_file: &std::collections::BTreeMap<&str, Vec<&SymbolRecord>>,
    token_budget: usize,
) -> String {
    let mut files = Vec::new();
    let mut used_tokens = 20; // overhead for JSON structure

    for (file, syms) in by_file {
        let content: String = syms
            .iter()
            .map(|s| s.signature.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let cost = estimate_tokens(&content) + 20; // per-file JSON overhead
        if used_tokens + cost > token_budget {
            break;
        }
        files.push(serde_json::json!({
            "path": file,
            "symbols": content
        }));
        used_tokens += cost;
    }

    serde_json::to_string_pretty(&serde_json::json!({ "files": files })).unwrap_or_default()
}

/// Tokenize text into lowercase terms for TF-IDF.
fn tokenize(text: &str) -> Vec<String> {
    // Split on non-alphanumeric, also split camelCase
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            if ch.is_uppercase() && !current.is_empty() {
                tokens.push(current.to_lowercase());
                current.clear();
            }
            current.push(ch);
        } else {
            if !current.is_empty() {
                tokens.push(current.to_lowercase());
                current.clear();
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current.to_lowercase());
    }

    // Filter short tokens
    tokens.retain(|t| t.len() >= 2);
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("getUserById");
        assert!(tokens.contains(&"get".to_string()));
        assert!(tokens.contains(&"user".to_string()));
        assert!(tokens.contains(&"by".to_string()));
        assert!(tokens.contains(&"id".to_string()));
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens("hello world"), 3); // 11 chars / 4 ≈ 3
    }

    #[test]
    fn test_tokenize_snake_case() {
        let tokens = tokenize("get_user_by_id");
        assert!(tokens.contains(&"get".to_string()));
        assert!(tokens.contains(&"user".to_string()));
        assert!(tokens.contains(&"by".to_string()));
        assert!(tokens.contains(&"id".to_string()));
    }
}
