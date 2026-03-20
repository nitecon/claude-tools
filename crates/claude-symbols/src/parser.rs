use crate::extractor::{extract_symbols_from_tree, Symbol};
use crate::languages::Language;
use anyhow::{Context, Result};
use std::path::Path;

pub struct SymbolParser {
    parser: tree_sitter::Parser,
}

impl Default for SymbolParser {
    fn default() -> Self {
        Self {
            parser: tree_sitter::Parser::new(),
        }
    }
}

impl SymbolParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a file and extract all symbols.
    pub fn parse_file(&mut self, path: &Path) -> Result<Vec<Symbol>> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let language = Language::from_path(path)?;
        self.parse_source(&source, language, path)
    }

    /// Parse source code and extract symbols.
    pub fn parse_source(
        &mut self,
        source: &str,
        language: Language,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        self.parser
            .set_language(&language.ts_language())
            .with_context(|| format!("Failed to set language: {language}"))?;

        let tree = self
            .parser
            .parse(source, None)
            .with_context(|| "Failed to parse source code")?;

        Ok(extract_symbols_from_tree(
            &tree, source, language, file_path,
        ))
    }

    /// Extract a single symbol by name from a file.
    /// Returns the symbol's source code with line numbers.
    pub fn extract_symbol(
        &mut self,
        path: &Path,
        symbol_name: &str,
    ) -> Result<Option<SymbolSource>> {
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let language = Language::from_path(path)?;
        self.parser
            .set_language(&language.ts_language())
            .with_context(|| format!("Failed to set language: {language}"))?;

        let tree = self
            .parser
            .parse(&source, None)
            .with_context(|| "Failed to parse source code")?;

        let symbols = extract_symbols_from_tree(&tree, &source, language, path);

        for symbol in symbols {
            if symbol.name == symbol_name {
                let lines: Vec<&str> = source.lines().collect();
                let start = symbol.start_line;
                let end = symbol.end_line.min(lines.len());

                let mut numbered_source = String::new();
                for (i, line) in lines[start - 1..end].iter().enumerate() {
                    numbered_source.push_str(&format!("{:>5} | {}\n", start + i, line));
                }

                return Ok(Some(SymbolSource {
                    symbol,
                    source: numbered_source,
                }));
            }
        }

        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct SymbolSource {
    pub symbol: Symbol,
    pub source: String,
}

impl std::fmt::Display for SymbolSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "// {} {} ({}:{}-{})",
            self.symbol.kind,
            self.symbol.name,
            self.symbol.file.display(),
            self.symbol.start_line,
            self.symbol.end_line
        )?;
        write!(f, "{}", self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rust_source() {
        let source = r#"
fn hello() {
    println!("hello");
}

struct Point {
    x: f64,
    y: f64,
}

impl Point {
    fn distance(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
}
"#;
        let mut parser = SymbolParser::new();
        let symbols = parser
            .parse_source(source, Language::Rust, Path::new("test.rs"))
            .unwrap();

        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"hello"));
        assert!(names.contains(&"Point"));
    }

    #[test]
    fn test_parse_cpp_source() {
        let source = r#"
class MyClass {
public:
    void doSomething() {}
    int getValue() { return 42; }
};

void freeFunction(int x) {
    return;
}
"#;
        let mut parser = SymbolParser::new();
        let symbols = parser
            .parse_source(source, Language::Cpp, Path::new("test.cpp"))
            .unwrap();

        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"MyClass"));
        assert!(names.contains(&"freeFunction"));
    }

    #[test]
    fn test_parse_python_source() {
        let source = r#"
def greet(name):
    print(f"Hello, {name}!")

class Animal:
    def __init__(self, name):
        self.name = name

    def speak(self):
        pass
"#;
        let mut parser = SymbolParser::new();
        let symbols = parser
            .parse_source(source, Language::Python, Path::new("test.py"))
            .unwrap();

        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"Animal"));
    }
}
