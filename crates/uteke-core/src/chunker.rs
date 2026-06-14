//! AST-aware code chunking — splits source files by semantic boundaries.
//!
//! Uses regex-based pattern matching (no tree-sitter dependency) to detect
//! function, class, and struct definitions. Supports Rust, Go, Python,
//! TypeScript/JavaScript, and Dart.

/// A code chunk representing a semantic unit (function, class, etc).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeChunk {
    /// The source code of this chunk.
    pub content: String,
    /// Language detected from file extension or content.
    pub language: String,
    /// Symbol type: function, struct, class, impl, interface, etc.
    pub symbol_type: String,
    /// Symbol name (function/class/struct name).
    pub symbol_name: String,
}

/// Detect language from file extension.
pub fn detect_language(filename: &str) -> &str {
    let ext = filename.rsplit('.').next().unwrap_or("");
    match ext {
        "rs" => "rust",
        "go" => "go",
        "py" => "python",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "dart" => "dart",
        "java" | "kt" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "rb" => "ruby",
        "swift" => "swift",
        "lua" => "lua",
        "svelte" => "svelte",
        _ => "text",
    }
}

/// Chunk source code by semantic boundaries.
///
/// Detects function, struct, class, impl, and interface definitions.
/// Falls back to line-based splitting if no patterns match.
pub fn chunk_code(content: &str, language: &str) -> Vec<CodeChunk> {
    match language {
        "rust" => chunk_rust(content),
        "go" => chunk_go(content),
        "python" => chunk_python(content),
        "typescript" | "javascript" => chunk_typescript(content),
        "dart" => chunk_dart(content),
        _ => vec![CodeChunk {
            content: content.to_string(),
            language: language.to_string(),
            symbol_type: "file".to_string(),
            symbol_name: "full".to_string(),
        }],
    }
}

/// Extract import/use statements from source code.
pub fn extract_imports(content: &str, language: &str) -> Vec<String> {
    let mut imports = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        let is_import = match language {
            "rust" => trimmed.starts_with("use "),
            "go" => trimmed.starts_with("import "),
            "python" => trimmed.starts_with("import ") || trimmed.starts_with("from "),
            "typescript" | "javascript" | "dart" => {
                trimmed.starts_with("import ")
                    || trimmed.starts_with("const ") && trimmed.contains("require(")
            }
            _ => false,
        };
        if is_import && !trimmed.is_empty() {
            imports.push(trimmed.to_string());
        }
    }
    imports
}

// ── Language-specific chunkers ──────────────────────────────────────────

fn chunk_rust(content: &str) -> Vec<CodeChunk> {
    let patterns = [
        ("fn ", "function"),
        ("struct ", "struct"),
        ("enum ", "enum"),
        ("trait ", "trait"),
        ("impl ", "impl"),
        ("macro_rules!", "macro"),
    ];

    chunk_by_patterns(content, "rust", &patterns, '{', '}')
}

fn chunk_go(content: &str) -> Vec<CodeChunk> {
    let patterns = [
        ("func ", "function"),
        ("type ", "struct"),
        ("interface{}", "interface"),
    ];

    chunk_by_patterns(content, "go", &patterns, '{', '}')
}

fn chunk_python(content: &str) -> Vec<CodeChunk> {
    let mut chunks = Vec::new();
    let mut current_name = String::new();
    let mut current_type = String::new();
    let mut current_lines: Vec<&str> = Vec::new();
    let mut in_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect new definitions
        let (new_name, new_type) = if trimmed.starts_with("def ") {
            let name = trimmed
                .trim_start_matches("def ")
                .split('(')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            (name, "function".to_string())
        } else if trimmed.starts_with("class ") {
            let name = trimmed
                .trim_start_matches("class ")
                .split('(')
                .next()
                .unwrap_or("")
                .split(':')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            (name, "class".to_string())
        } else {
            (String::new(), String::new())
        };

        if !new_name.is_empty() {
            // Save previous block
            if in_block && !current_lines.is_empty() {
                chunks.push(CodeChunk {
                    content: current_lines.join("\n"),
                    language: "python".to_string(),
                    symbol_type: current_type,
                    symbol_name: current_name,
                });
            }
            current_name = new_name;
            current_type = new_type;
            current_lines = vec![line];
            in_block = true;
        } else if in_block {
            current_lines.push(line);
        }
    }

    // Don't forget the last block
    if in_block && !current_lines.is_empty() {
        chunks.push(CodeChunk {
            content: current_lines.join("\n"),
            language: "python".to_string(),
            symbol_type: current_type,
            symbol_name: current_name,
        });
    }

    // If no chunks found, return whole file
    if chunks.is_empty() {
        chunks.push(CodeChunk {
            content: content.to_string(),
            language: "python".to_string(),
            symbol_type: "file".to_string(),
            symbol_name: "full".to_string(),
        });
    }

    chunks
}

fn chunk_typescript(content: &str) -> Vec<CodeChunk> {
    let patterns = [
        ("function ", "function"),
        ("class ", "class"),
        ("interface ", "interface"),
        ("const ", "const"), // arrow functions, consts
        ("export default ", "export"),
        ("export ", "export"),
    ];

    let mut chunks = chunk_by_patterns(content, "typescript", &patterns, '{', '}');

    // Filter: only keep const/export chunks that contain function-like syntax
    chunks.retain(|c| {
        if c.symbol_type == "const" || c.symbol_type == "export" {
            c.content.contains("=>") || c.content.contains("function")
        } else {
            true
        }
    });

    if chunks.is_empty() {
        chunks.push(CodeChunk {
            content: content.to_string(),
            language: "typescript".to_string(),
            symbol_type: "file".to_string(),
            symbol_name: "full".to_string(),
        });
    }

    chunks
}

fn chunk_dart(content: &str) -> Vec<CodeChunk> {
    let patterns = [
        ("void ", "function"),
        ("Future<", "function"),
        ("Stream<", "function"),
        ("class ", "class"),
        ("enum ", "enum"),
        ("Widget ", "widget"),
    ];

    chunk_by_patterns(content, "dart", &patterns, '{', '}')
}

/// Generic pattern-based chunker for brace-delimited languages.
fn chunk_by_patterns(
    content: &str,
    language: &str,
    patterns: &[(&str, &str)],
    open: char,
    close: char,
) -> Vec<CodeChunk> {
    let mut chunks = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        for (keyword, sym_type) in patterns {
            if trimmed.starts_with(keyword)
                || (keyword.starts_with("export") && trimmed.contains(keyword))
            {
                // Extract symbol name
                let after_kw = trimmed.trim_start_matches(*keyword);
                let name = after_kw
                    .split(|c: char| {
                        c.is_whitespace() || c == '(' || c == '<' || c == '{' || c == ':'
                    })
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();

                if name.is_empty() {
                    continue;
                }

                // Find the body by matching braces
                let body = match extract_block(&lines, i, open, close) {
                    Some(b) => b,
                    None => continue,
                };

                chunks.push(CodeChunk {
                    content: body,
                    language: language.to_string(),
                    symbol_type: sym_type.to_string(),
                    symbol_name: name,
                });
                break; // Don't match same line twice
            }
        }
    }

    if chunks.is_empty() {
        chunks.push(CodeChunk {
            content: content.to_string(),
            language: language.to_string(),
            symbol_type: "file".to_string(),
            symbol_name: "full".to_string(),
        });
    }

    chunks
}

/// Extract a brace-delimited block starting from `start_line`.
/// Returns the full text from the definition line to the closing brace.
fn extract_block(lines: &[&str], start: usize, open: char, close: char) -> Option<String> {
    let mut depth = 0i32;
    let mut found_open = false;
    let mut block_lines: Vec<&str> = Vec::new();

    for &line in &lines[start..] {
        block_lines.push(line);

        for ch in line.chars() {
            if ch == open {
                depth += 1;
                found_open = true;
            } else if ch == close {
                depth -= 1;
            }
        }

        if found_open && depth <= 0 {
            return Some(block_lines.join("\n"));
        }
    }

    // If no braces found but we have content, return a few lines
    if !block_lines.is_empty() && !found_open {
        let end = (start + 5).min(lines.len());
        return Some(lines[start..end].join("\n"));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language("main.rs"), "rust");
        assert_eq!(detect_language("main.go"), "go");
        assert_eq!(detect_language("app.py"), "python");
        assert_eq!(detect_language("App.tsx"), "typescript");
        assert_eq!(detect_language("main.dart"), "dart");
        assert_eq!(detect_language("README.md"), "text");
    }

    #[test]
    fn test_chunk_rust_functions() {
        let code = r#"
fn hello() {
    println!("hello");
}

fn world(x: i32) -> i32 {
    x + 1
}
"#;
        let chunks = chunk_code(code, "rust");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].symbol_type, "function");
        assert_eq!(chunks[0].symbol_name, "hello");
        assert_eq!(chunks[1].symbol_name, "world");
    }

    #[test]
    fn test_chunk_rust_struct() {
        let code = r#"
struct Config {
    name: String,
    port: u16,
}
"#;
        let chunks = chunk_code(code, "rust");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].symbol_type, "struct");
        assert_eq!(chunks[0].symbol_name, "Config");
    }

    #[test]
    fn test_chunk_python_functions() {
        let code = r#"
def greet(name):
    return f"Hello {name}"

class Calculator:
    def add(self, a, b):
        return a + b
"#;
        let chunks = chunk_code(code, "python");
        // def greet, class Calculator (with add method inside)
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].symbol_name, "greet");
    }

    #[test]
    fn test_chunk_go_functions() {
        let code = r#"
package main

func main() {
    fmt.Println("hello")
}

func add(a, b int) int {
    return a + b
}
"#;
        let chunks = chunk_code(code, "go");
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn test_chunk_typescript() {
        let code = r#"
function greet(name: string): string {
    return `Hello ${name}`;
}

interface User {
    id: number;
    name: string;
}
"#;
        let chunks = chunk_code(code, "typescript");
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn test_extract_imports_rust() {
        let code = r#"
use serde::{Serialize, Deserialize};

fn main() {}
"#;
        let imports = extract_imports(code, "rust");
        assert!(
            !imports.is_empty(),
            "expected at least 1 import, got {}: {:?}",
            imports.len(),
            imports
        );
    }

    #[test]
    fn test_chunk_fallback_text() {
        let code = "some random text\nno patterns here";
        let chunks = chunk_code(code, "text");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].symbol_type, "file");
    }

    #[test]
    fn test_chunk_dart() {
        let code = r#"
class MyApp extends StatelessWidget {
  Widget build(BuildContext context) {
    return Container();
  }
}
"#;
        let chunks = chunk_code(code, "dart");
        assert!(!chunks.is_empty());
    }
}
