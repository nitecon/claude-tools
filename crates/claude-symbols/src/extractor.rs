use crate::languages::Language;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Impl,
    Module,
    Namespace,
    Macro,
    Type,
    Constant,
    Variable,
    Property,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SymbolKind::Function => write!(f, "fn"),
            SymbolKind::Method => write!(f, "method"),
            SymbolKind::Class => write!(f, "class"),
            SymbolKind::Struct => write!(f, "struct"),
            SymbolKind::Enum => write!(f, "enum"),
            SymbolKind::Trait => write!(f, "trait"),
            SymbolKind::Interface => write!(f, "interface"),
            SymbolKind::Impl => write!(f, "impl"),
            SymbolKind::Module => write!(f, "mod"),
            SymbolKind::Namespace => write!(f, "namespace"),
            SymbolKind::Macro => write!(f, "macro"),
            SymbolKind::Type => write!(f, "type"),
            SymbolKind::Constant => write!(f, "const"),
            SymbolKind::Variable => write!(f, "var"),
            SymbolKind::Property => write!(f, "prop"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub language: Language,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// Extract symbols from a parsed tree-sitter tree.
pub fn extract_symbols_from_tree(
    tree: &tree_sitter::Tree,
    source: &str,
    language: Language,
    file_path: &Path,
) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    let root = tree.root_node();
    let symbol_kinds = language.symbol_node_kinds();

    collect_symbols(
        root,
        source,
        language,
        file_path,
        symbol_kinds,
        None,
        &mut symbols,
    );

    symbols
}

fn collect_symbols(
    node: tree_sitter::Node,
    source: &str,
    language: Language,
    file_path: &Path,
    symbol_kinds: &[&str],
    parent_name: Option<&str>,
    symbols: &mut Vec<Symbol>,
) {
    let node_kind = node.kind();

    if symbol_kinds.contains(&node_kind) {
        if let Some(symbol) = make_symbol(node, source, language, file_path, parent_name) {
            let name_for_children = symbol.name.clone();
            symbols.push(symbol);

            // Recurse into children with this symbol as parent
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_symbols(
                    child,
                    source,
                    language,
                    file_path,
                    symbol_kinds,
                    Some(&name_for_children),
                    symbols,
                );
            }
            return;
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols(
            child,
            source,
            language,
            file_path,
            symbol_kinds,
            parent_name,
            symbols,
        );
    }
}

fn make_symbol(
    node: tree_sitter::Node,
    source: &str,
    language: Language,
    file_path: &Path,
    parent_name: Option<&str>,
) -> Option<Symbol> {
    let name = find_name(node, source, &language)?;
    let kind = classify_node(node.kind(), &language, parent_name.is_some());

    // Lines are 0-indexed in tree-sitter, we want 1-indexed
    let start_line = node.start_position().row + 1;
    let end_line = node.end_position().row + 1;

    Some(Symbol {
        name,
        kind,
        file: file_path.to_path_buf(),
        start_line,
        end_line,
        language,
        parent: parent_name.map(|s| s.to_string()),
    })
}

fn find_name(node: tree_sitter::Node, source: &str, language: &Language) -> Option<String> {
    let name_kinds = language.name_node_kinds();

    // For some node types, look for specific child field names first
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = name_node.utf8_text(source.as_bytes()).ok()?;
        return Some(text.to_string());
    }

    // For decorated definitions (Python), dig into the definition child
    if node.kind() == "decorated_definition" {
        if let Some(def_node) = node.child_by_field_name("definition") {
            if let Some(name_node) = def_node.child_by_field_name("name") {
                let text = name_node.utf8_text(source.as_bytes()).ok()?;
                return Some(text.to_string());
            }
        }
    }

    // For template declarations (C++), dig into the inner declaration
    if node.kind() == "template_declaration" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(name) = find_name(child, source, language) {
                return Some(name);
            }
        }
    }

    // Walk immediate children looking for an identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if name_kinds.contains(&child.kind()) {
            let text = child.utf8_text(source.as_bytes()).ok()?;
            return Some(text.to_string());
        }
        // For declarators (C/C++), look inside them
        if child.kind().contains("declarator") {
            if let Some(name) = find_name_in_declarator(child, source, name_kinds) {
                return Some(name);
            }
        }
    }

    None
}

fn find_name_in_declarator(
    node: tree_sitter::Node,
    source: &str,
    name_kinds: &[&str],
) -> Option<String> {
    // Check direct field
    if let Some(name_node) = node.child_by_field_name("declarator") {
        return find_name_in_declarator(name_node, source, name_kinds);
    }

    if name_kinds.contains(&node.kind()) {
        return node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if name_kinds.contains(&child.kind()) {
            return child
                .utf8_text(source.as_bytes())
                .ok()
                .map(|s| s.to_string());
        }
        if child.kind().contains("declarator") {
            if let Some(name) = find_name_in_declarator(child, source, name_kinds) {
                return Some(name);
            }
        }
    }

    None
}

fn classify_node(node_kind: &str, language: &Language, has_parent: bool) -> SymbolKind {
    match language {
        Language::Cpp => match node_kind {
            "function_definition" if has_parent => SymbolKind::Method,
            "function_definition" => SymbolKind::Function,
            "class_specifier" => SymbolKind::Class,
            "struct_specifier" => SymbolKind::Struct,
            "enum_specifier" => SymbolKind::Enum,
            "namespace_definition" => SymbolKind::Namespace,
            "template_declaration" => SymbolKind::Function, // refined by inner node
            "preproc_def" | "preproc_function_def" => SymbolKind::Macro,
            "type_definition" => SymbolKind::Type,
            "declaration" => SymbolKind::Variable,
            _ => SymbolKind::Variable,
        },
        Language::Rust => match node_kind {
            "function_item" if has_parent => SymbolKind::Method,
            "function_item" => SymbolKind::Function,
            "struct_item" => SymbolKind::Struct,
            "enum_item" => SymbolKind::Enum,
            "trait_item" => SymbolKind::Trait,
            "impl_item" => SymbolKind::Impl,
            "mod_item" => SymbolKind::Module,
            "type_item" => SymbolKind::Type,
            "macro_definition" => SymbolKind::Macro,
            "const_item" => SymbolKind::Constant,
            "static_item" => SymbolKind::Variable,
            _ => SymbolKind::Variable,
        },
        Language::Python => match node_kind {
            "function_definition" if has_parent => SymbolKind::Method,
            "function_definition" => SymbolKind::Function,
            "class_definition" => SymbolKind::Class,
            "decorated_definition" => SymbolKind::Function, // could be method/class too
            _ => SymbolKind::Variable,
        },
        Language::TypeScript | Language::JavaScript => match node_kind {
            "function_declaration" => SymbolKind::Function,
            "class_declaration" => SymbolKind::Class,
            "method_definition" => SymbolKind::Method,
            "interface_declaration" => SymbolKind::Interface,
            "type_alias_declaration" => SymbolKind::Type,
            "enum_declaration" => SymbolKind::Enum,
            _ => SymbolKind::Variable,
        },
        Language::CSharp => match node_kind {
            "method_declaration" => SymbolKind::Method,
            "class_declaration" => SymbolKind::Class,
            "struct_declaration" => SymbolKind::Struct,
            "enum_declaration" => SymbolKind::Enum,
            "interface_declaration" => SymbolKind::Interface,
            "namespace_declaration" => SymbolKind::Namespace,
            "property_declaration" => SymbolKind::Property,
            _ => SymbolKind::Variable,
        },
        Language::Go => match node_kind {
            "function_declaration" => SymbolKind::Function,
            "method_declaration" => SymbolKind::Method,
            "type_declaration" => SymbolKind::Type,
            "const_declaration" => SymbolKind::Constant,
            "var_declaration" => SymbolKind::Variable,
            _ => SymbolKind::Variable,
        },
    }
}
