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
pub struct ParsedFile {
    pub path: String,
    pub language: String,
    pub hash: u64,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
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
        _ => None,
    }
}

pub fn parse_file(path: &Path, source: &str) -> Result<ParsedFile> {
    let lang_name = detect_language(path)
        .ok_or_else(|| anyhow!("Unsupported file type: {}", path.display()))?;

    // Swift not supported yet at runtime (no grammar crate)
    if lang_name == "swift" {
        return Err(anyhow!("Swift parsing not yet supported"));
    }

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

    Ok(ParsedFile {
        path: path.to_string_lossy().replace('\\', "/"),
        language: lang_name,
        hash,
        symbols,
        imports,
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
        _ => {}
    }
}

// ─── Python ──────────────────────────────────────────────────────────

fn python_visibility(name: &str) -> Visibility {
    if name.starts_with("__") && !name.ends_with("__") {
        Visibility::Private
    } else if name.starts_with('_') {
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
    } else if text.contains("protected ") {
        Visibility::Internal
    } else {
        Visibility::Internal // package-private
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
                let name = path.split('.').last().unwrap_or("").to_string();
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
