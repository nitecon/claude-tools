use claude_core::ToolError;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Cpp,
    Rust,
    Python,
    TypeScript,
    JavaScript,
    CSharp,
    Go,
}

impl Language {
    /// Detect language from file extension.
    pub fn from_path(path: &Path) -> Result<Self, ToolError> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        match ext {
            "cpp" | "cc" | "cxx" | "c" | "h" | "hpp" | "hxx" | "hh" => Ok(Language::Cpp),
            "rs" => Ok(Language::Rust),
            "py" | "pyi" => Ok(Language::Python),
            "ts" | "tsx" => Ok(Language::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Ok(Language::JavaScript),
            "cs" => Ok(Language::CSharp),
            "go" => Ok(Language::Go),
            _ => Err(ToolError::UnsupportedLanguage(format!(
                "No grammar for extension: .{ext}"
            ))),
        }
    }

    /// Get the tree-sitter language for this language.
    pub fn ts_language(&self) -> tree_sitter::Language {
        match self {
            Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::JavaScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            Language::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }

    /// Get the tree-sitter node types that represent named symbols for this language.
    pub fn symbol_node_kinds(&self) -> &'static [&'static str] {
        match self {
            Language::Cpp => &[
                "function_definition",
                "declaration",
                "class_specifier",
                "struct_specifier",
                "enum_specifier",
                "namespace_definition",
                "template_declaration",
                "preproc_def",
                "preproc_function_def",
                "type_definition",
            ],
            Language::Rust => &[
                "function_item",
                "struct_item",
                "enum_item",
                "trait_item",
                "impl_item",
                "mod_item",
                "type_item",
                "macro_definition",
                "const_item",
                "static_item",
            ],
            Language::Python => &[
                "function_definition",
                "class_definition",
                "decorated_definition",
            ],
            Language::TypeScript | Language::JavaScript => &[
                "function_declaration",
                "class_declaration",
                "method_definition",
                "lexical_declaration",
                "export_statement",
                "interface_declaration",
                "type_alias_declaration",
                "enum_declaration",
            ],
            Language::CSharp => &[
                "method_declaration",
                "class_declaration",
                "struct_declaration",
                "enum_declaration",
                "interface_declaration",
                "namespace_declaration",
                "property_declaration",
            ],
            Language::Go => &[
                "function_declaration",
                "method_declaration",
                "type_declaration",
                "const_declaration",
                "var_declaration",
            ],
        }
    }

    /// Get the node type names that represent identifiers/names for this language.
    pub fn name_node_kinds(&self) -> &'static [&'static str] {
        match self {
            Language::Cpp => &[
                "identifier",
                "field_identifier",
                "type_identifier",
                "namespace_identifier",
            ],
            Language::Rust => &["identifier", "type_identifier"],
            Language::Python => &["identifier"],
            Language::TypeScript | Language::JavaScript => {
                &["identifier", "property_identifier", "type_identifier"]
            }
            Language::CSharp => &["identifier"],
            Language::Go => &["identifier", "type_identifier", "field_identifier"],
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Language::Cpp => write!(f, "C/C++"),
            Language::Rust => write!(f, "Rust"),
            Language::Python => write!(f, "Python"),
            Language::TypeScript => write!(f, "TypeScript"),
            Language::JavaScript => write!(f, "JavaScript"),
            Language::CSharp => write!(f, "C#"),
            Language::Go => write!(f, "Go"),
        }
    }
}
