use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tree_sitter::{Language, Node, Parser};
use xxhash_rust::xxh3::xxh3_64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Type,
    Const,
    Interface,
    Module,
    /// Mutable or computed property (Swift `var`, incl. SwiftUI `body`).
    Property,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SymbolKind::Function => write!(f, "function"),
            SymbolKind::Class => write!(f, "class"),
            SymbolKind::Method => write!(f, "method"),
            SymbolKind::Type => write!(f, "type"),
            SymbolKind::Const => write!(f, "const"),
            SymbolKind::Interface => write!(f, "interface"),
            SymbolKind::Module => write!(f, "module"),
            SymbolKind::Property => write!(f, "property"),
        }
    }
}

impl SymbolKind {
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "function" => Some(Self::Function),
            "class" => Some(Self::Class),
            "method" => Some(Self::Method),
            "type" => Some(Self::Type),
            "const" => Some(Self::Const),
            "interface" => Some(Self::Interface),
            "module" => Some(Self::Module),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Private,
    Internal,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Visibility::Public => write!(f, "public"),
            Visibility::Private => write!(f, "private"),
            Visibility::Internal => write!(f, "internal"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub signature: String,
    pub docstring: Option<String>,
    pub byte_start: usize,
    pub byte_end: usize,
    pub children: Vec<Symbol>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Import {
    pub raw_text: String,
    pub source_module: Option<String>,
    pub imported_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub name: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFile {
    pub path: String,
    pub language: String,
    pub hash: u64,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
    pub references: Vec<Reference>,
}

pub fn detect_language(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "py" => Some("python".into()),
        "ts" => Some("typescript".into()),
        "tsx" => Some("tsx".into()),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript".into()),
        "rs" => Some("rust".into()),
        "go" => Some("go".into()),
        "java" => Some("java".into()),
        "c" | "h" => Some("c".into()),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp".into()),
        "rb" => Some("ruby".into()),
        "swift" => Some("swift".into()),
        _ => None,
    }
}

pub fn get_language(name: &str) -> Option<Language> {
    match name {
        "python" => Some(tree_sitter_python::LANGUAGE.into()),
        "javascript" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "typescript" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "rust" => Some(tree_sitter_rust::LANGUAGE.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
        "c" => Some(tree_sitter_c::LANGUAGE.into()),
        "cpp" => Some(tree_sitter_cpp::LANGUAGE.into()),
        "ruby" => Some(tree_sitter_ruby::LANGUAGE.into()),
        "swift" => Some(tree_sitter_swift::LANGUAGE.into()),
        _ => None,
    }
}

pub fn parse_file(path: &Path, source: &str) -> Result<ParsedFile> {
    let lang_name = detect_language(path)
        .ok_or_else(|| anyhow!("Unsupported file type: {}", path.display()))?;

    let language = get_language(&lang_name)
        .ok_or_else(|| anyhow!("No grammar for language: {}", lang_name))?;

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| anyhow!("Failed to set language: {}", e))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("Failed to parse: {}", path.display()))?;

    let root = tree.root_node();
    let source_bytes = source.as_bytes();
    let hash = xxh3_64(source_bytes);

    let symbols = extract_symbols(root, source_bytes, &lang_name);
    let imports = extract_imports(root, source_bytes, &lang_name);
    let references = extract_references(root, source_bytes, &lang_name);

    Ok(ParsedFile {
        path: path.to_string_lossy().replace('\\', "/"),
        language: lang_name,
        hash,
        symbols,
        imports,
        references,
    })
}

fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

fn signature_up_to_body(node: Node, source: &[u8]) -> String {
    if let Some(body) = node.child_by_field_name("body") {
        let start = node.start_byte();
        let end = body.start_byte();
        let sig = String::from_utf8_lossy(&source[start..end]);
        sig.trim_end().trim_end_matches('{').trim_end().to_string()
    } else {
        node_text(node, source)
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    }
}

// ─── Symbol extraction ───────────────────────────────────────────────

fn extract_symbols(root: Node, source: &[u8], lang: &str) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        extract_node_symbols(child, source, lang, &mut symbols);
    }
    symbols
}

fn extract_node_symbols(node: Node, source: &[u8], lang: &str, out: &mut Vec<Symbol>) {
    // Error recovery: a construct the grammar doesn't know can shatter the
    // parse, leaving well-formed declaration nodes strewn inside ERROR
    // subtrees. Descend into ERROR nodes so one bad expression doesn't erase
    // every symbol after it in the file.
    if node.kind() == "ERROR" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            extract_node_symbols(child, source, lang, out);
        }
        return;
    }
    match lang {
        "python" => extract_python_node(node, source, out),
        "typescript" | "tsx" => extract_ts_node(node, source, out, true),
        "javascript" => extract_ts_node(node, source, out, false),
        "rust" => extract_rust_node(node, source, out),
        "go" => extract_go_node(node, source, out),
        "java" => extract_java_node(node, source, out),
        "c" => extract_c_node(node, source, out),
        "cpp" => extract_cpp_node(node, source, out),
        "ruby" => extract_ruby_node(node, source, out),
        "swift" => extract_swift_node(node, source, out),
        _ => {}
    }
}

// ─── Python ──────────────────────────────────────────────────────────

fn python_visibility(name: &str) -> Visibility {
    if name.starts_with('_') {
        Visibility::Private
    } else {
        Visibility::Public
    }
}

fn python_docstring(node: Node, source: &[u8]) -> Option<String> {
    let body = node.child_by_field_name("body")?;
    let mut cursor = body.walk();
    let first = body.children(&mut cursor).next()?;
    if first.kind() == "expression_statement" {
        let expr = first.child(0)?;
        if expr.kind() == "string" || expr.kind() == "concatenated_string" {
            let text = node_text(expr, source);
            let trimmed = text
                .trim_start_matches("\"\"\"")
                .trim_start_matches("'''")
                .trim_end_matches("\"\"\"")
                .trim_end_matches("'''")
                .trim_start_matches('"')
                .trim_start_matches('\'')
                .trim_end_matches('"')
                .trim_end_matches('\'')
                .trim();
            return Some(trimmed.lines().next().unwrap_or(trimmed).to_string());
        }
    }
    None
}

fn extract_python_node(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    match node.kind() {
        "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let vis = python_visibility(&name);
                let sig = signature_up_to_body(node, source);
                let doc = python_docstring(node, source);
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Function,
                    visibility: vis,
                    signature: sig,
                    docstring: doc,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "class_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let vis = python_visibility(&name);
                let sig = signature_up_to_body(node, source);
                let doc = python_docstring(node, source);
                let mut children = Vec::new();
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cursor = body.walk();
                    for child in body.children(&mut cursor) {
                        match child.kind() {
                            "function_definition" => {
                                if let Some(mn) = child.child_by_field_name("name") {
                                    let mname = node_text(mn, source).to_string();
                                    let mvis = python_visibility(&mname);
                                    let msig = signature_up_to_body(child, source);
                                    let mdoc = python_docstring(child, source);
                                    children.push(Symbol {
                                        name: mname,
                                        kind: SymbolKind::Method,
                                        visibility: mvis,
                                        signature: msig,
                                        docstring: mdoc,
                                        byte_start: child.start_byte(),
                                        byte_end: child.end_byte(),
                                        children: vec![],
                                    });
                                }
                            }
                            "decorated_definition" => {
                                if let Some(def) = child.child_by_field_name("definition") {
                                    if def.kind() == "function_definition" {
                                        if let Some(mn) = def.child_by_field_name("name") {
                                            let mname = node_text(mn, source).to_string();
                                            let mvis = python_visibility(&mname);
                                            let msig = signature_up_to_body(def, source);
                                            let mdoc = python_docstring(def, source);
                                            children.push(Symbol {
                                                name: mname,
                                                kind: SymbolKind::Method,
                                                visibility: mvis,
                                                signature: msig,
                                                docstring: mdoc,
                                                byte_start: def.start_byte(),
                                                byte_end: def.end_byte(),
                                                children: vec![],
                                            });
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    visibility: vis,
                    signature: sig,
                    docstring: doc,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children,
                });
            }
        }
        "decorated_definition" => {
            if let Some(def) = node.child_by_field_name("definition") {
                extract_python_node(def, source, out);
            }
        }
        "expression_statement" => {
            // Module-level constant: NAME = value (UPPER_CASE convention)
            if let Some(assign) = node.child(0) {
                if assign.kind() == "assignment" {
                    if let Some(left) = assign.child_by_field_name("left") {
                        if left.kind() == "identifier" {
                            let name = node_text(left, source);
                            if name.chars().all(|c| c.is_ascii_uppercase() || c == '_')
                                && !name.is_empty()
                            {
                                let sig = node_text(node, source).trim().to_string();
                                out.push(Symbol {
                                    name: name.to_string(),
                                    kind: SymbolKind::Const,
                                    visibility: Visibility::Public,
                                    signature: sig,
                                    docstring: None,
                                    byte_start: node.start_byte(),
                                    byte_end: node.end_byte(),
                                    children: vec![],
                                });
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

// ─── TypeScript / JavaScript ────────────────────────────────────────

fn extract_ts_node(node: Node, source: &[u8], out: &mut Vec<Symbol>, is_ts: bool) {
    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let sig = signature_up_to_body(node, source);
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Function,
                    visibility: Visibility::Internal,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let sig = signature_up_to_body(node, source);
                let mut children = Vec::new();
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cursor = body.walk();
                    for child in body.children(&mut cursor) {
                        if child.kind() == "method_definition" {
                            if let Some(mn) = child.child_by_field_name("name") {
                                let mname = node_text(mn, source).to_string();
                                let msig = signature_up_to_body(child, source);
                                children.push(Symbol {
                                    name: mname,
                                    kind: SymbolKind::Method,
                                    visibility: Visibility::Public,
                                    signature: msig,
                                    docstring: None,
                                    byte_start: child.start_byte(),
                                    byte_end: child.end_byte(),
                                    children: vec![],
                                });
                            }
                        }
                        if child.kind() == "public_field_definition"
                            || child.kind() == "field_definition"
                        {
                            if let Some(pn) = child.child_by_field_name("name") {
                                let pname = node_text(pn, source).to_string();
                                let psig = node_text(child, source).trim().to_string();
                                children.push(Symbol {
                                    name: pname,
                                    kind: SymbolKind::Const,
                                    visibility: Visibility::Public,
                                    signature: psig,
                                    docstring: None,
                                    byte_start: child.start_byte(),
                                    byte_end: child.end_byte(),
                                    children: vec![],
                                });
                            }
                        }
                    }
                }
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    visibility: Visibility::Internal,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children,
                });
            }
        }
        "interface_declaration" if is_ts => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let sig = signature_up_to_body(node, source);
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Interface,
                    visibility: Visibility::Internal,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "type_alias_declaration" if is_ts => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let sig = node_text(node, source)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Type,
                    visibility: Visibility::Internal,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "export_statement" => {
            // Unwrap export and mark visibility as public
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let mut inner = Vec::new();
                extract_ts_node(child, source, &mut inner, is_ts);
                for mut sym in inner {
                    sym.visibility = Visibility::Public;
                    out.push(sym);
                }
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            // const FOO = ... or let/var at top level
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "variable_declarator" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = node_text(name_node, source).to_string();
                        let sig = node_text(node, source)
                            .lines()
                            .next()
                            .unwrap_or("")
                            .to_string();
                        // Check if const (parent starts with "const")
                        let full = node_text(node, source);
                        let kind = if full.trim_start().starts_with("const") {
                            SymbolKind::Const
                        } else {
                            SymbolKind::Function // variable
                        };
                        out.push(Symbol {
                            name,
                            kind,
                            visibility: Visibility::Internal,
                            signature: sig,
                            docstring: None,
                            byte_start: node.start_byte(),
                            byte_end: node.end_byte(),
                            children: vec![],
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

// ─── Rust ────────────────────────────────────────────────────────────

fn rust_visibility(node: Node, source: &[u8]) -> Visibility {
    let text = node_text(node, source);
    if text.contains("pub ") || text.starts_with("pub(") || text.starts_with("pub ") {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

fn extract_rust_node(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    match node.kind() {
        "function_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let vis = rust_visibility(node, source);
                let sig = signature_up_to_body(node, source);
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Function,
                    visibility: vis,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "struct_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let vis = rust_visibility(node, source);
                let sig = {
                    let text = node_text(node, source);
                    text.lines().next().unwrap_or("").to_string()
                };
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Type,
                    visibility: vis,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "enum_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let vis = rust_visibility(node, source);
                let sig = {
                    let text = node_text(node, source);
                    text.lines().next().unwrap_or("").to_string()
                };
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Type,
                    visibility: vis,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "trait_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let vis = rust_visibility(node, source);
                let sig = signature_up_to_body(node, source);
                let mut children = Vec::new();
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cursor = body.walk();
                    for child in body.children(&mut cursor) {
                        if child.kind() == "function_item" {
                            if let Some(mn) = child.child_by_field_name("name") {
                                let mname = node_text(mn, source).to_string();
                                let msig = signature_up_to_body(child, source);
                                children.push(Symbol {
                                    name: mname,
                                    kind: SymbolKind::Method,
                                    visibility: Visibility::Public,
                                    signature: msig,
                                    docstring: None,
                                    byte_start: child.start_byte(),
                                    byte_end: child.end_byte(),
                                    children: vec![],
                                });
                            }
                        }
                    }
                }
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Interface,
                    visibility: vis,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children,
                });
            }
        }
        "impl_item" => {
            // Extract methods from impl blocks
            let type_name = node
                .child_by_field_name("type")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();
            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                for child in body.children(&mut cursor) {
                    if child.kind() == "function_item" {
                        if let Some(mn) = child.child_by_field_name("name") {
                            let mname = node_text(mn, source).to_string();
                            let vis = rust_visibility(child, source);
                            let msig = signature_up_to_body(child, source);
                            out.push(Symbol {
                                name: format!("{}::{}", type_name, mname),
                                kind: SymbolKind::Method,
                                visibility: vis,
                                signature: msig,
                                docstring: None,
                                byte_start: child.start_byte(),
                                byte_end: child.end_byte(),
                                children: vec![],
                            });
                        }
                    }
                }
            }
        }
        "const_item" | "static_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let vis = rust_visibility(node, source);
                let sig = node_text(node, source)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Const,
                    visibility: vis,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "type_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let vis = rust_visibility(node, source);
                let sig = node_text(node, source).trim().to_string();
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Type,
                    visibility: vis,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        _ => {}
    }
}

// ─── Go ──────────────────────────────────────────────────────────────

fn go_visibility(name: &str) -> Visibility {
    if name.starts_with(|c: char| c.is_ascii_uppercase()) {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

fn extract_go_node(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let vis = go_visibility(&name);
                let sig = signature_up_to_body(node, source);
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Function,
                    visibility: vis,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let vis = go_visibility(&name);
                let sig = signature_up_to_body(node, source);
                let receiver = node
                    .child_by_field_name("receiver")
                    .map(|r| node_text(r, source).to_string())
                    .unwrap_or_default();
                let full_name = if !receiver.is_empty() {
                    // Extract type from receiver like (r *Router)
                    let recv_type = receiver
                        .trim_matches(|c: char| c == '(' || c == ')')
                        .split_whitespace()
                        .last()
                        .unwrap_or("")
                        .trim_start_matches('*');
                    format!("{}.{}", recv_type, name)
                } else {
                    name
                };
                out.push(Symbol {
                    name: full_name,
                    kind: SymbolKind::Method,
                    visibility: vis,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "type_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_spec" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = node_text(name_node, source).to_string();
                        let vis = go_visibility(&name);
                        let sig = node_text(child, source)
                            .lines()
                            .next()
                            .unwrap_or("")
                            .to_string();
                        let kind = if node_text(child, source).contains("interface") {
                            SymbolKind::Interface
                        } else {
                            SymbolKind::Type
                        };
                        out.push(Symbol {
                            name,
                            kind,
                            visibility: vis,
                            signature: sig,
                            docstring: None,
                            byte_start: child.start_byte(),
                            byte_end: child.end_byte(),
                            children: vec![],
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

// ─── Java ────────────────────────────────────────────────────────────

fn java_visibility(node: Node, source: &[u8]) -> Visibility {
    let text = node_text(node, source);
    if text.contains("public ") {
        Visibility::Public
    } else if text.contains("private ") {
        Visibility::Private
    } else {
        Visibility::Internal // protected or package-private
    }
}

fn extract_java_node(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    match node.kind() {
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let vis = java_visibility(node, source);
                let sig = signature_up_to_body(node, source);
                let mut children = Vec::new();
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cursor = body.walk();
                    for child in body.children(&mut cursor) {
                        if child.kind() == "method_declaration"
                            || child.kind() == "constructor_declaration"
                        {
                            if let Some(mn) = child.child_by_field_name("name") {
                                let mname = node_text(mn, source).to_string();
                                let mvis = java_visibility(child, source);
                                let msig = signature_up_to_body(child, source);
                                children.push(Symbol {
                                    name: mname,
                                    kind: SymbolKind::Method,
                                    visibility: mvis,
                                    signature: msig,
                                    docstring: None,
                                    byte_start: child.start_byte(),
                                    byte_end: child.end_byte(),
                                    children: vec![],
                                });
                            }
                        }
                        if child.kind() == "field_declaration" {
                            let text = node_text(child, source);
                            if text.contains("static") && text.contains("final") {
                                // Extract constant name from field declaration
                                let mut fc = child.walk();
                                for fchild in child.children(&mut fc) {
                                    if fchild.kind() == "variable_declarator" {
                                        if let Some(fname) = fchild.child_by_field_name("name") {
                                            let cname = node_text(fname, source).to_string();
                                            children.push(Symbol {
                                                name: cname,
                                                kind: SymbolKind::Const,
                                                visibility: java_visibility(child, source),
                                                signature: text.trim().to_string(),
                                                docstring: None,
                                                byte_start: child.start_byte(),
                                                byte_end: child.end_byte(),
                                                children: vec![],
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    visibility: vis,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children,
                });
            }
        }
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let vis = java_visibility(node, source);
                let sig = signature_up_to_body(node, source);
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Interface,
                    visibility: vis,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        _ => {}
    }
}

// ─── C ───────────────────────────────────────────────────────────────

fn extract_c_node(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    match node.kind() {
        "function_definition" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                let name = extract_declarator_name(declarator, source);
                if !name.is_empty() {
                    let sig = signature_up_to_body(node, source);
                    out.push(Symbol {
                        name,
                        kind: SymbolKind::Function,
                        visibility: Visibility::Public,
                        signature: sig,
                        docstring: None,
                        byte_start: node.start_byte(),
                        byte_end: node.end_byte(),
                        children: vec![],
                    });
                }
            }
        }
        "declaration" => {
            let text = node_text(node, source);
            if text.contains("const ") || text.starts_with("#define") {
                let name = extract_declaration_name(node, source);
                if !name.is_empty() {
                    out.push(Symbol {
                        name,
                        kind: SymbolKind::Const,
                        visibility: Visibility::Public,
                        signature: text.trim().to_string(),
                        docstring: None,
                        byte_start: node.start_byte(),
                        byte_end: node.end_byte(),
                        children: vec![],
                    });
                }
            }
        }
        "struct_specifier" | "enum_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let sig = node_text(node, source)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Type,
                    visibility: Visibility::Public,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "type_definition" => {
            // typedef ... name;
            let text = node_text(node, source).trim().to_string();
            // Last word before ; is the name
            if let Some(name) = text.trim_end_matches(';').split_whitespace().last() {
                out.push(Symbol {
                    name: name.to_string(),
                    kind: SymbolKind::Type,
                    visibility: Visibility::Public,
                    signature: text,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        _ => {}
    }
}

fn extract_declarator_name(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, source).to_string(),
        "function_declarator" => {
            if let Some(decl) = node.child_by_field_name("declarator") {
                extract_declarator_name(decl, source)
            } else {
                String::new()
            }
        }
        "pointer_declarator" => {
            if let Some(decl) = node.child_by_field_name("declarator") {
                extract_declarator_name(decl, source)
            } else {
                String::new()
            }
        }
        _ => {
            // Try first named child
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                let name = extract_declarator_name(child, source);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
    }
}

fn extract_declaration_name(node: Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "init_declarator" || child.kind() == "declarator" {
            return extract_declarator_name(child, source);
        }
    }
    String::new()
}

// ─── C++ ─────────────────────────────────────────────────────────────

fn extract_cpp_node(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    match node.kind() {
        "function_definition" => {
            extract_c_node(node, source, out);
        }
        "class_specifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let sig = format!("class {}", name);
                let mut children = Vec::new();
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cursor = body.walk();
                    for child in body.children(&mut cursor) {
                        if child.kind() == "function_definition" || child.kind() == "declaration" {
                            let mut inner = Vec::new();
                            extract_c_node(child, source, &mut inner);
                            for mut s in inner {
                                s.kind = SymbolKind::Method;
                                children.push(s);
                            }
                        }
                    }
                }
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    visibility: Visibility::Public,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children,
                });
            }
        }
        "namespace_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Module,
                    visibility: Visibility::Public,
                    signature: format!("namespace {}", node_text(name_node, source)),
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
            // Also extract children of namespace
            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                for child in body.children(&mut cursor) {
                    extract_cpp_node(child, source, out);
                }
            }
        }
        "struct_specifier" | "enum_specifier" | "declaration" | "type_definition" => {
            extract_c_node(node, source, out);
        }
        _ => {}
    }
}

// ─── Ruby ────────────────────────────────────────────────────────────

fn extract_ruby_node(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    match node.kind() {
        "method" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let sig = {
                    let text = node_text(node, source);
                    text.lines().next().unwrap_or("").to_string()
                };
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Function,
                    visibility: Visibility::Public,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "singleton_method" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let sig = node_text(node, source)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                out.push(Symbol {
                    name: format!("self.{}", name),
                    kind: SymbolKind::Method,
                    visibility: Visibility::Public,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        "class" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let sig = node_text(node, source)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                let mut children = Vec::new();
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cursor = body.walk();
                    for child in body.children(&mut cursor) {
                        if child.kind() == "method" {
                            if let Some(mn) = child.child_by_field_name("name") {
                                let mname = node_text(mn, source).to_string();
                                let msig = node_text(child, source)
                                    .lines()
                                    .next()
                                    .unwrap_or("")
                                    .to_string();
                                children.push(Symbol {
                                    name: mname,
                                    kind: SymbolKind::Method,
                                    visibility: Visibility::Public,
                                    signature: msig,
                                    docstring: None,
                                    byte_start: child.start_byte(),
                                    byte_end: child.end_byte(),
                                    children: vec![],
                                });
                            }
                        }
                    }
                }
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Class,
                    visibility: Visibility::Public,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children,
                });
            }
        }
        "module" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                let sig = node_text(node, source)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Module,
                    visibility: Visibility::Public,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
        _ => {}
    }
}

// ─── Swift ───────────────────────────────────────────────────────────
//
// Grammar: alex-pinkus tree-sitter-swift (ABI 14, crate 0.6.0). Notable shapes:
//  • class/struct/enum/actor/extension all parse as `class_declaration`,
//    discriminated by the `declaration_kind` field. `protocol` is its own node.
//  • function `name` field is the first `simple_identifier`; init/deinit name
//    fields are the keyword nodes. Property names live under `pattern`.
//  • Visibility lives in an optional `modifiers` child (default: internal).

/// Visibility from a declaration's optional `modifiers` child only (never body text).
fn swift_visibility(node: Node, source: &[u8]) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = node_text(child, source);
            if text.contains("public") || text.contains("open") {
                return Visibility::Public;
            }
            if text.contains("private") || text.contains("fileprivate") {
                return Visibility::Private;
            }
            break;
        }
    }
    Visibility::Internal
}

/// Pull the bound identifier out of a `pattern` node (handles `let foo`, `var foo`).
fn swift_pattern_name(node: Node, source: &[u8]) -> Option<String> {
    if let Some(bi) = node.child_by_field_name("bound_identifier") {
        return Some(node_text(bi, source).to_string());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "simple_identifier" {
            return Some(node_text(child, source).to_string());
        }
        if let Some(n) = swift_pattern_name(child, source) {
            return Some(n);
        }
    }
    None
}

fn swift_first_line(node: Node, source: &[u8]) -> String {
    node_text(node, source)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn swift_function_symbol(node: Node, source: &[u8], kind: SymbolKind) -> Option<Symbol> {
    // Subscripts and deinit have no `name` field — synthesize one.
    let name = match node.kind() {
        "subscript_declaration" => "subscript".to_string(),
        "deinit_declaration" => "deinit".to_string(),
        _ => {
            let name_node = node.child_by_field_name("name")?;
            node_text(name_node, source).trim().to_string()
        }
    };
    if name.is_empty() {
        return None;
    }
    Some(Symbol {
        name,
        kind,
        visibility: swift_visibility(node, source),
        signature: signature_up_to_body(node, source),
        docstring: None,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        children: vec![],
    })
}

/// True when a property declaration is a `var` binding (mutable/computed —
/// includes SwiftUI `body` and wrapped state), as opposed to a `let` constant.
fn swift_is_var_binding(node: Node, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    if node.children(&mut cursor).any(|c| {
        c.kind() == "var"
            || (c.kind() == "value_binding_pattern" && node_text(c, source).starts_with("var"))
    }) {
        return true;
    }
    // Fallback: token scan of the declaration head (before any `=`/`{`).
    let head = swift_first_line(node, source);
    head.split(['=', '{'])
        .next()
        .unwrap_or("")
        .split_whitespace()
        .any(|t| t == "var")
}

/// Emit one symbol per bound name (`var` → Property, `let` → Const). A single
/// `property_declaration` can carry several bindings (`let a = 1, b = 2`),
/// each as its own `pattern` child.
fn swift_push_properties(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    let vis = swift_visibility(node, source);
    let sig = swift_first_line(node, source);
    let kind = if swift_is_var_binding(node, source) {
        SymbolKind::Property
    } else {
        SymbolKind::Const
    };
    let mut pushed = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "pattern" {
            continue;
        }
        if let Some(name) = swift_pattern_name(child, source) {
            if !name.is_empty() {
                pushed = true;
                out.push(Symbol {
                    name,
                    kind,
                    visibility: vis,
                    signature: sig.clone(),
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
    }
    // Fallback for shapes where the binding isn't a direct `pattern` child.
    if !pushed {
        if let Some(nf) = node.child_by_field_name("name") {
            let name = swift_pattern_name(nf, source)
                .unwrap_or_else(|| node_text(nf, source).trim().to_string());
            if !name.is_empty() {
                out.push(Symbol {
                    name,
                    kind,
                    visibility: vis,
                    signature: sig,
                    docstring: None,
                    byte_start: node.start_byte(),
                    byte_end: node.end_byte(),
                    children: vec![],
                });
            }
        }
    }
}

fn swift_simple_symbol(
    node: Node,
    source: &[u8],
    name: String,
    kind: SymbolKind,
) -> Option<Symbol> {
    if name.is_empty() {
        return None;
    }
    Some(Symbol {
        name,
        kind,
        visibility: swift_visibility(node, source),
        signature: swift_first_line(node, source),
        docstring: None,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        children: vec![],
    })
}

/// First descendant of the given kind (shallow search across direct children, then deeper).
fn swift_first_child_text(node: Node, source: &[u8], kind: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(node_text(child, source).trim().to_string());
        }
    }
    None
}

fn swift_declaration_kind(node: Node, source: &[u8]) -> String {
    node.child_by_field_name("declaration_kind")
        .map(|n| node_text(n, source).trim().to_string())
        .unwrap_or_default()
}

/// Map a type-defining `class_declaration`/`protocol_declaration` to a SymbolKind.
fn swift_type_kind(decl_kind: &str) -> SymbolKind {
    match decl_kind {
        "protocol" => SymbolKind::Interface,
        "enum" => SymbolKind::Type,
        // class, struct, actor — value/reference types that carry member bodies.
        _ => SymbolKind::Class,
    }
}

/// Build a fully-formed Symbol for a nominal type, recursively populating members.
fn swift_build_type(node: Node, source: &[u8]) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, source).trim().to_string();
    if name.is_empty() {
        return None;
    }
    let decl_kind = swift_declaration_kind(node, source);
    let mut children = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for member in body.children(&mut cursor) {
            swift_member(member, source, &mut children);
        }
    }
    Some(Symbol {
        name,
        kind: swift_type_kind(&decl_kind),
        visibility: swift_visibility(node, source),
        signature: signature_up_to_body(node, source),
        docstring: None,
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        children,
    })
}

/// Flatten an `extension` body into top-level `Type.method` symbols (mirrors Rust impl handling).
fn swift_flatten_extension(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    let type_name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, source).trim().to_string())
        .unwrap_or_default();
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for member in body.children(&mut cursor) {
            let mut tmp = Vec::new();
            swift_member(member, source, &mut tmp);
            for mut sym in tmp {
                // Prefix every member — methods, properties, and nested types
                // alike — so extension members stay attached to their type
                // (SwiftUI code keeps most computed vars and nested enums in
                // extensions).
                if !type_name.is_empty() {
                    sym.name = format!("{}.{}", type_name, sym.name);
                }
                out.push(sym);
            }
        }
    }
}

/// Extract a single member of a type body (method, property, case, nested type, …).
fn swift_member(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    match node.kind() {
        "function_declaration" | "protocol_function_declaration" => {
            if let Some(s) = swift_function_symbol(node, source, SymbolKind::Method) {
                out.push(s);
            }
        }
        "init_declaration" | "deinit_declaration" | "subscript_declaration" => {
            if let Some(s) = swift_function_symbol(node, source, SymbolKind::Method) {
                out.push(s);
            }
        }
        "property_declaration" | "protocol_property_declaration" => {
            swift_push_properties(node, source, out);
        }
        "enum_entry" => {
            // `case up, down` → one Const per case name.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "simple_identifier" {
                    let name = node_text(child, source).to_string();
                    if let Some(s) = swift_simple_symbol(node, source, name, SymbolKind::Const) {
                        out.push(s);
                    }
                }
            }
        }
        "associatedtype_declaration" => {
            if let Some(name) = swift_first_child_text(node, source, "type_identifier") {
                if let Some(s) = swift_simple_symbol(node, source, name, SymbolKind::Type) {
                    out.push(s);
                }
            }
        }
        "typealias_declaration" => {
            if let Some(name) = swift_first_child_text(node, source, "type_identifier") {
                if let Some(s) = swift_simple_symbol(node, source, name, SymbolKind::Type) {
                    out.push(s);
                }
            }
        }
        "class_declaration" => {
            if swift_declaration_kind(node, source) == "extension" {
                swift_flatten_extension(node, source, out);
            } else if let Some(s) = swift_build_type(node, source) {
                out.push(s);
            }
        }
        "protocol_declaration" => {
            if let Some(s) = swift_build_type(node, source) {
                out.push(s);
            }
        }
        _ => {}
    }
}

fn extract_swift_node(node: Node, source: &[u8], out: &mut Vec<Symbol>) {
    match node.kind() {
        "function_declaration" => {
            if let Some(s) = swift_function_symbol(node, source, SymbolKind::Function) {
                out.push(s);
            }
        }
        "property_declaration" => {
            swift_push_properties(node, source, out);
        }
        "typealias_declaration" => {
            if let Some(name) = swift_first_child_text(node, source, "type_identifier") {
                if let Some(s) = swift_simple_symbol(node, source, name, SymbolKind::Type) {
                    out.push(s);
                }
            }
        }
        "operator_declaration" => {
            if let Some(name) = swift_first_child_text(node, source, "custom_operator") {
                if let Some(s) = swift_simple_symbol(node, source, name, SymbolKind::Function) {
                    out.push(s);
                }
            }
        }
        "protocol_declaration" => {
            if let Some(s) = swift_build_type(node, source) {
                out.push(s);
            }
        }
        "class_declaration" => {
            if swift_declaration_kind(node, source) == "extension" {
                swift_flatten_extension(node, source, out);
            } else if let Some(s) = swift_build_type(node, source) {
                out.push(s);
            }
        }
        _ => {
            // Recovery path: only descend into unhandled nodes when the
            // subtree contains parse errors — well-formed files are untouched,
            // but declarations buried in a broken region are still found.
            if node.has_error() {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    extract_swift_node(child, source, out);
                }
            }
        }
    }
}

// ─── Import extraction ──────────────────────────────────────────────

fn extract_imports(root: Node, source: &[u8], lang: &str) -> Vec<Import> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match (lang, child.kind()) {
            // Python
            ("python", "import_statement") => {
                let text = node_text(child, source).to_string();
                let names: Vec<String> = text
                    .strip_prefix("import ")
                    .unwrap_or("")
                    .split(',')
                    .map(|s| {
                        s.trim()
                            .split(" as ")
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_string()
                    })
                    .filter(|s| !s.is_empty())
                    .collect();
                let module = names.first().cloned();
                imports.push(Import {
                    raw_text: text,
                    source_module: module,
                    imported_names: names,
                });
            }
            ("python", "import_from_statement") => {
                let text = node_text(child, source).to_string();
                let module = child
                    .child_by_field_name("module_name")
                    .map(|n| node_text(n, source).to_string());
                let mut names = Vec::new();
                let mut ic = child.walk();
                for c in child.children(&mut ic) {
                    if c.kind() == "dotted_name" || c.kind() == "aliased_import" {
                        let n = node_text(c, source)
                            .split(" as ")
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if !n.is_empty() {
                            names.push(n);
                        }
                    }
                }
                imports.push(Import {
                    raw_text: text,
                    source_module: module,
                    imported_names: names,
                });
            }
            // TypeScript / JavaScript
            ("typescript" | "tsx" | "javascript", "import_statement") => {
                let text = node_text(child, source).to_string();
                let source_mod = child.child_by_field_name("source").map(|n| {
                    node_text(n, source)
                        .trim_matches(|c: char| c == '\'' || c == '"')
                        .to_string()
                });
                let mut names = Vec::new();
                let mut ic = child.walk();
                for c in child.children(&mut ic) {
                    if c.kind() == "import_specifier" || c.kind() == "identifier" {
                        let n = c
                            .child_by_field_name("name")
                            .map(|n| node_text(n, source).to_string())
                            .unwrap_or_else(|| node_text(c, source).to_string());
                        if !n.is_empty() && n != "import" && n != "from" {
                            names.push(n);
                        }
                    }
                }
                imports.push(Import {
                    raw_text: text,
                    source_module: source_mod,
                    imported_names: names,
                });
            }
            // Rust
            ("rust", "use_declaration") => {
                let text = node_text(child, source).to_string();
                let path = text
                    .strip_prefix("use ")
                    .unwrap_or("")
                    .trim_end_matches(';')
                    .trim()
                    .to_string();
                let names = vec![path.clone()];
                imports.push(Import {
                    raw_text: text,
                    source_module: Some(path),
                    imported_names: names,
                });
            }
            // Go
            ("go", "import_declaration") => {
                let text = node_text(child, source).to_string();
                let mut names = Vec::new();
                let mut ic = child.walk();
                for c in child.children(&mut ic) {
                    if c.kind() == "import_spec" || c.kind() == "interpreted_string_literal" {
                        let n = node_text(c, source).trim_matches('"').to_string();
                        if !n.is_empty() {
                            names.push(n.clone());
                        }
                    }
                }
                let module = names.first().cloned();
                imports.push(Import {
                    raw_text: text,
                    source_module: module,
                    imported_names: names,
                });
            }
            // Java
            ("java", "import_declaration") => {
                let text = node_text(child, source).to_string();
                let path = text
                    .strip_prefix("import ")
                    .unwrap_or("")
                    .trim_end_matches(';')
                    .trim()
                    .to_string();
                let name = path.split('.').next_back().unwrap_or("").to_string();
                imports.push(Import {
                    raw_text: text,
                    source_module: Some(path),
                    imported_names: vec![name],
                });
            }
            // C / C++
            ("c" | "cpp", "preproc_include") => {
                let text = node_text(child, source).to_string();
                let path = child.child_by_field_name("path").map(|n| {
                    node_text(n, source)
                        .trim_matches(|c: char| c == '"' || c == '<' || c == '>')
                        .to_string()
                });
                imports.push(Import {
                    raw_text: text,
                    source_module: path.clone(),
                    imported_names: path.into_iter().collect(),
                });
            }
            // Swift
            ("swift", "import_declaration") => {
                let text = node_text(child, source).to_string();
                // The module path is the `identifier` child (dotted, e.g. `A.B`).
                let mut module = None;
                let mut ic = child.walk();
                for c in child.children(&mut ic) {
                    if c.kind() == "identifier" {
                        module = Some(node_text(c, source).trim().to_string());
                    }
                }
                let names = module.clone().into_iter().collect();
                imports.push(Import {
                    raw_text: text,
                    source_module: module,
                    imported_names: names,
                });
            }
            // Ruby
            ("ruby", "call") => {
                let text = node_text(child, source);
                if text.starts_with("require") {
                    let arg = child
                        .child_by_field_name("arguments")
                        .and_then(|a| a.child(0))
                        .map(|n| {
                            node_text(n, source)
                                .trim_matches(|c: char| {
                                    c == '\'' || c == '"' || c == '(' || c == ')'
                                })
                                .to_string()
                        });
                    imports.push(Import {
                        raw_text: text.to_string(),
                        source_module: arg.clone(),
                        imported_names: arg.into_iter().collect(),
                    });
                }
            }
            _ => {}
        }
    }
    imports
}

// ─── Reference extraction ───────────────────────────────────────────

fn extract_references(root: Node, source: &[u8], lang: &str) -> Vec<Reference> {
    let mut references = Vec::new();
    walk_tree(root, &mut |node| {
        if !is_call_node(lang, node.kind()) {
            return;
        }

        let Some(callee) = call_target_node(node, lang) else {
            return;
        };
        let Some(name) = extract_reference_name(callee, source) else {
            return;
        };

        references.push(Reference {
            name,
            byte_start: callee.start_byte(),
            byte_end: callee.end_byte(),
            context: "call".to_string(),
        });
    });

    references.sort_by_key(|r| (r.byte_start, r.byte_end, r.name.clone()));
    references.dedup_by(|a, b| {
        a.name == b.name && a.byte_start == b.byte_start && a.byte_end == b.byte_end
    });
    references
}

fn walk_tree(node: Node, visit: &mut dyn FnMut(Node)) {
    visit(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_tree(child, visit);
    }
}

fn is_call_node(lang: &str, kind: &str) -> bool {
    match lang {
        "python" => kind == "call",
        "typescript" | "tsx" | "javascript" | "rust" | "go" | "c" | "cpp" => {
            kind == "call_expression"
        }
        "java" => kind == "method_invocation" || kind == "object_creation_expression",
        "ruby" => kind == "call",
        "swift" => kind == "call_expression",
        _ => false,
    }
}

fn call_target_node<'a>(node: Node<'a>, lang: &str) -> Option<Node<'a>> {
    match lang {
        "python" | "typescript" | "tsx" | "javascript" | "rust" | "go" | "c" | "cpp" => {
            node.child_by_field_name("function")
        }
        "java" => node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("type")),
        "ruby" => node
            .child_by_field_name("method")
            .or_else(|| node.child_by_field_name("name")),
        // Swift `call_expression`'s callee is its first child (a `simple_identifier`
        // for free calls, or a `navigation_expression` for method/static calls).
        "swift" => node.child(0),
        _ => None,
    }
}

fn extract_reference_name(node: Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier"
        | "type_identifier"
        | "field_identifier"
        | "property_identifier"
        | "simple_identifier" => sanitize_reference_name(node_text(node, source)),
        _ => {
            let mut result = None;
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if let Some(name) = extract_reference_name(child, source) {
                    result = Some(name);
                }
            }
            result
        }
    }
}

fn sanitize_reference_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if matches!(trimmed, "self" | "super" | "this" | "crate") {
        return None;
    }
    Some(trimmed.to_string())
}

/// Flatten all symbols (including children) into a linear list.
pub fn flatten_symbols(symbols: &[Symbol]) -> Vec<&Symbol> {
    let mut result = Vec::new();
    for sym in symbols {
        result.push(sym);
        for child in &sym.children {
            result.push(child);
        }
    }
    result
}

/// Get the full source text of a symbol from the original source.
pub fn symbol_source<'a>(sym: &Symbol, source: &'a str) -> &'a str {
    let start = sym.byte_start.min(source.len());
    let end = sym.byte_end.min(source.len());
    &source[start..end]
}

#[cfg(test)]
mod tests {
    use super::{flatten_symbols, parse_file, SymbolKind, Visibility};
    use std::path::Path;

    #[test]
    fn extracts_python_call_references() {
        let parsed = parse_file(
            Path::new("main.py"),
            r#"
from pkg.util import helper

def target():
    helper()

def caller():
    target()
"#,
        )
        .expect("python should parse");

        let names: Vec<_> = parsed.references.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"helper"));
        assert!(names.contains(&"target"));
        assert!(parsed.references.iter().all(|r| r.context == "call"));
    }

    #[test]
    fn extracts_swift_symbols_imports_and_references() {
        let parsed = parse_file(
            Path::new("Sample.swift"),
            r#"
import Foundation
@testable import STVADomain

public func freeFunction() -> Bool { helper() }
func helper() {}

public struct MyStruct {
    let id: Int
    public func method() -> Int { return id }
}

class MyClass {
    init() {}
    private func secret() {}
}

enum Direction { case up, down }

protocol Drawable {
    func draw()
    var area: Double { get }
}

extension MyStruct: Drawable {
    func draw() { method() }
}

typealias Handler = (Int) -> Void
"#,
        )
        .expect("swift should parse");

        // Top-level symbols
        let top: Vec<_> = parsed.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            top.contains(&"freeFunction"),
            "missing freeFunction: {top:?}"
        );
        assert!(top.contains(&"MyStruct"), "missing MyStruct: {top:?}");
        assert!(top.contains(&"MyClass"));
        assert!(top.contains(&"Direction"));
        assert!(top.contains(&"Drawable"));
        assert!(top.contains(&"Handler"));
        // Extension methods are flattened to `Type.method`
        assert!(
            top.contains(&"MyStruct.draw"),
            "missing extension method: {top:?}"
        );

        // Kinds
        let struct_sym = parsed
            .symbols
            .iter()
            .find(|s| s.name == "MyStruct")
            .unwrap();
        assert_eq!(struct_sym.kind, SymbolKind::Class);
        assert_eq!(struct_sym.visibility, Visibility::Public);
        let proto = parsed
            .symbols
            .iter()
            .find(|s| s.name == "Drawable")
            .unwrap();
        assert_eq!(proto.kind, SymbolKind::Interface);

        // Members nested under their type
        let method_names: Vec<_> = struct_sym
            .children
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(
            method_names.contains(&"method"),
            "members: {method_names:?}"
        );
        assert!(method_names.contains(&"id"));

        // Enum cases
        let dir = parsed
            .symbols
            .iter()
            .find(|s| s.name == "Direction")
            .unwrap();
        let cases: Vec<_> = dir.children.iter().map(|c| c.name.as_str()).collect();
        assert!(
            cases.contains(&"up") && cases.contains(&"down"),
            "cases: {cases:?}"
        );

        // Imports
        let modules: Vec<_> = parsed
            .imports
            .iter()
            .filter_map(|i| i.source_module.as_deref())
            .collect();
        assert!(modules.contains(&"Foundation"), "imports: {modules:?}");
        assert!(modules.contains(&"STVADomain"));

        // References (calls)
        let refs: Vec<_> = parsed.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"helper"), "refs: {refs:?}");
        assert!(refs.contains(&"method"));
    }

    #[test]
    fn swift_deinit_extension_members_and_property_kinds() {
        let parsed = parse_file(
            Path::new("View.swift"),
            r#"
import SwiftUI

struct ContentView: View {
    @State private var counter = 0
    let fixed = 42
    var body: some View { Text("hi") }
}

actor Cache {
    init() {}
    deinit {}
}

extension ContentView {
    enum Route { case home, detail }
    var title: String { "t" }
    static func preview() -> ContentView { ContentView() }
}
"#,
        )
        .expect("swift should parse");

        let flat = flatten_symbols(&parsed.symbols);
        let names: Vec<_> = flat.iter().map(|s| s.name.as_str()).collect();

        // deinit is captured
        assert!(names.contains(&"deinit"), "missing deinit: {names:?}");

        // ALL extension members keep their type prefix, not just methods
        let top: Vec<_> = parsed.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(top.contains(&"ContentView.Route"), "top: {top:?}");
        assert!(top.contains(&"ContentView.title"), "top: {top:?}");
        assert!(top.contains(&"ContentView.preview"), "top: {top:?}");

        // var → Property (incl. wrapped state and computed body), let → Const
        let view = parsed
            .symbols
            .iter()
            .find(|s| s.name == "ContentView")
            .unwrap();
        let counter = view.children.iter().find(|c| c.name == "counter").unwrap();
        assert_eq!(
            counter.kind,
            SymbolKind::Property,
            "wrapped var is a property"
        );
        let body = view.children.iter().find(|c| c.name == "body").unwrap();
        assert_eq!(
            body.kind,
            SymbolKind::Property,
            "computed var is a property"
        );
        let fixed = view.children.iter().find(|c| c.name == "fixed").unwrap();
        assert_eq!(fixed.kind, SymbolKind::Const, "let stays const");
        let title = parsed
            .symbols
            .iter()
            .find(|s| s.name == "ContentView.title")
            .unwrap();
        assert_eq!(title.kind, SymbolKind::Property);
    }

    #[test]
    fn swift_recovers_symbols_after_parse_errors() {
        // The `@#$%` garbage guarantees a parse error mid-file; declarations
        // after it must still be extracted via ERROR-node recovery.
        let parsed = parse_file(
            Path::new("Broken.swift"),
            r#"
class BeforeError {
    func early() {}
}

let x = @#$%^&*

class AfterError {
    deinit {}
    func late() {}
}
"#,
        )
        .expect("swift should parse even with errors");

        let flat = flatten_symbols(&parsed.symbols);
        let names: Vec<_> = flat.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"BeforeError"), "names: {names:?}");
        assert!(
            names.contains(&"AfterError"),
            "symbols after a parse error were lost: {names:?}"
        );
        assert!(names.contains(&"late"), "names: {names:?}");
    }

    #[test]
    fn swift_handles_unicode_generics_nesting_and_operators() {
        let parsed = parse_file(
            Path::new("Adv.swift"),
            r#"
import struct SwiftUI.Color

func café(naïve: Int) -> Bool { return true }
let π = 3.14159

public func transform<T, U>(
    _ input: [T],
    using mapper: (T) -> U
) -> [U] where T: Equatable {
    return input.map(mapper)
}

infix operator <=>: ComparisonPrecedence

enum Outer {
    struct Middle {
        class Inner { func deep() {} }
    }
}

@MainActor
final class VM {
    @Published private(set) var count = 0
}
"#,
        )
        .expect("adversarial swift parses");

        let flat = flatten_symbols(&parsed.symbols);
        let names: Vec<_> = flat.iter().map(|s| s.name.as_str()).collect();

        // Unicode identifiers survive with valid byte offsets.
        assert!(names.contains(&"café"));
        assert!(names.contains(&"π"));
        for s in &flat {
            assert!(parsed_offsets_valid(s), "bad offsets for {}", s.name);
        }

        // Generic function keeps its bare name despite <T, U> and multi-line where clause.
        let t = parsed
            .symbols
            .iter()
            .find(|s| s.name == "transform")
            .unwrap();
        assert_eq!(t.visibility, Visibility::Public);

        // Operator declaration captured.
        assert!(names.contains(&"<=>"));

        // 3-level nesting: Outer > Middle > Inner > deep.
        let outer = parsed.symbols.iter().find(|s| s.name == "Outer").unwrap();
        let middle = outer.children.iter().find(|s| s.name == "Middle").unwrap();
        let inner = middle.children.iter().find(|s| s.name == "Inner").unwrap();
        assert!(inner.children.iter().any(|s| s.name == "deep"));

        // `private(set)` modifier resolves to private visibility.
        let vm = parsed.symbols.iter().find(|s| s.name == "VM").unwrap();
        let count = vm.children.iter().find(|s| s.name == "count").unwrap();
        assert_eq!(count.visibility, Visibility::Private);

        // `import struct SwiftUI.Color` resolves the dotted module.
        assert!(parsed
            .imports
            .iter()
            .any(|i| i.source_module.as_deref() == Some("SwiftUI.Color")));
    }

    fn parsed_offsets_valid(s: &super::Symbol) -> bool {
        s.byte_start <= s.byte_end
    }

    #[test]
    fn swift_emits_one_symbol_per_binding() {
        let parsed = parse_file(
            Path::new("Bind.swift"),
            "let a = 1, b = 2\nvar p: Int, q: String\n",
        )
        .expect("swift parses");
        let names: Vec<_> = parsed.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"a") && names.contains(&"b"),
            "got {names:?}"
        );
        assert!(
            names.contains(&"p") && names.contains(&"q"),
            "got {names:?}"
        );
    }

    #[test]
    fn swift_empty_and_malformed_do_not_panic() {
        // Empty file
        let empty = parse_file(Path::new("Empty.swift"), "").expect("empty swift parses");
        assert!(empty.symbols.is_empty());
        // Truncated / malformed source must not panic
        let broken = parse_file(
            Path::new("Broken.swift"),
            "func foo( {\n  let x =\nstruct {",
        );
        assert!(broken.is_ok(), "malformed swift should degrade, not error");
    }
}
