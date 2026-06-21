//! AST-aware code chunking — splits source files by semantic boundaries.
//!
//! Uses regex-based pattern matching (no tree-sitter dependency) to detect
//! function, class, and struct definitions. Supports Rust, Go, Python,
//! TypeScript/JavaScript, and Dart.
//!
//! Also provides markdown/prose chunking (#405) — splits by headings
//! while respecting a token window.

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

/// A text chunk from markdown/prose splitting (#405).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TextChunk {
    /// Section heading (empty string if no heading).
    pub heading: String,
    /// The text content of this chunk.
    pub content: String,
    /// Heading level (1-6, 0 = no heading).
    pub level: u8,
    /// Character offset from start of original text.
    pub char_start: usize,
    /// Character offset end (exclusive).
    pub char_end: usize,
}

/// Chunk markdown using embedder's max_seq_len (#407).
///
/// Derives max_chars from the embedder's token limit using the
/// heuristic: ~4 chars per token. This ensures chunks fit within
/// the embedding model's sequence window.
///
/// For ONNX (256 tokens): max_chars = 256 * 4 = 1024
/// For OpenAI (8191 tokens): max_chars = 8191 * 4 = 32764
pub fn chunk_markdown_embed_aware<E: crate::embed::Embedder>(
    text: &str,
    embedder: &E,
) -> Vec<TextChunk> {
    const CHARS_PER_TOKEN: usize = 4;
    let max_chars = embedder.max_seq_len().saturating_mul(CHARS_PER_TOKEN);
    // Guard against zero or implausibly small seq_len.
    let max_chars = if max_chars < 100 { 1024 } else { max_chars };
    chunk_markdown(text, max_chars)
}

/// Chunk markdown or prose text by headings (#405).
///
/// Splits by `#`, `##`, ... headings. When a section exceeds `max_chars`,
/// falls back to paragraph-level splitting. Code blocks (``` fences) are
/// never split mid-block.
///
/// `max_chars` should be derived from `embedder.max_seq_len()` — roughly
/// 4 chars per token. For ONNX (256 tokens): ~1024 chars. For OpenAI
/// (8191 tokens): ~32K chars.
pub fn chunk_markdown(text: &str, max_chars: usize) -> Vec<TextChunk> {
    if text.trim().is_empty() {
        return vec![];
    }
    let max_chars = if max_chars == 0 { 1024 } else { max_chars };

    // Split into sections by heading lines.
    let sections = split_by_headings(text);

    let mut chunks = Vec::new();
    let mut char_offset = 0usize;

    for section in &sections {
        let section_len = section.content.len();

        if section_len <= max_chars {
            // Section fits in one chunk.
            chunks.push(TextChunk {
                heading: section.heading.clone(),
                content: section.content.clone(),
                level: section.level,
                char_start: char_offset,
                char_end: char_offset + section_len,
            });
        } else {
            // Section too large — split by paragraphs, keeping heading.
            let sub_chunks = split_by_paragraphs(&section.content, max_chars);
            let heading_prefix = if section.heading.is_empty() {
                String::new()
            } else {
                format!(
                    "{} {}\n\n",
                    "#".repeat(section.level as usize),
                    section.heading
                )
            };

            for (i, sub) in sub_chunks.iter().enumerate() {
                let full = if i == 0 && !heading_prefix.is_empty() {
                    format!("{heading_prefix}{sub}")
                } else {
                    sub.clone()
                };
                chunks.push(TextChunk {
                    heading: if i == 0 {
                        section.heading.clone()
                    } else {
                        format!("{} (part {})", section.heading, i + 1)
                    },
                    content: full,
                    level: section.level,
                    char_start: char_offset,
                    char_end: char_offset + sub.len(),
                });
            }
        }

        char_offset += section_len + 1; // +1 for the separator consumed during split
    }

    chunks
}

/// Internal: a section bounded by headings.
struct MdSection {
    heading: String,
    content: String,
    level: u8,
}

/// Split markdown into sections by heading lines.
/// Each section includes its heading line in the content.
fn split_by_headings(text: &str) -> Vec<MdSection> {
    let lines: Vec<&str> = text.lines().collect();
    let mut sections = Vec::new();
    let mut current_heading = String::new();
    let mut current_level: u8 = 0;
    let mut current_lines: Vec<&str> = Vec::new();
    let mut in_code_block = false;

    for line in &lines {
        // Track code block state — don't treat # inside code blocks as headings.
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            current_lines.push(line);
            continue;
        }

        if !in_code_block {
            // Check if this line is a heading (1-6 # marks).
            if let Some((level, title)) = parse_heading(line) {
                // Flush previous section.
                if !current_lines.is_empty() {
                    sections.push(MdSection {
                        heading: current_heading.clone(),
                        content: current_lines.join("\n"),
                        level: current_level,
                    });
                }
                current_heading = title;
                current_level = level;
                current_lines = vec![line];
                continue;
            }
        }

        current_lines.push(line);
    }

    // Flush final section.
    if !current_lines.is_empty() {
        sections.push(MdSection {
            heading: current_heading,
            content: current_lines.join("\n"),
            level: current_level,
        });
    }

    sections
}

/// Parse a markdown heading line (e.g., "## Title" → (2, "Title")).
fn parse_heading(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    // Must have at least one space after #s (not a tag like #hashtag).
    if !rest.starts_with(' ') && !rest.is_empty() {
        return None;
    }
    let title = rest.trim_start().trim_end();
    Some((hashes as u8, title.to_string()))
}

/// Split text by paragraphs, respecting code block boundaries.
/// Accumulates paragraphs until `max_chars` is reached.
fn split_by_paragraphs(text: &str, max_chars: usize) -> Vec<String> {
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let mut chunks = Vec::new();
    let mut current = String::new();

    for para in &paragraphs {
        if current.len() + para.len() + 2 > max_chars && !current.is_empty() {
            // Current chunk is full — flush it.
            chunks.push(std::mem::take(&mut current).trim_end().to_string());
        }

        if para.len() > max_chars {
            // Single paragraph exceeds limit — hard split by lines.
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current).trim_end().to_string());
            }
            for line_chunk in split_long_text(para, max_chars) {
                chunks.push(line_chunk);
            }
        } else {
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(para);
        }
    }

    if !current.is_empty() {
        chunks.push(current.trim_end().to_string());
    }

    if chunks.is_empty() {
        chunks.push(text.to_string());
    }

    chunks
}

/// Hard-split very long text by character boundary.
/// Tries to break at sentence boundaries (.), then words (space).
fn split_long_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let end = (start + max_chars).min(text.len());
        if end == text.len() {
            chunks.push(text[start..].to_string());
            break;
        }

        // Try to find a sentence boundary (.) or word boundary (space).
        let slice = &text[start..end];
        let break_at = slice
            .rfind(". ")
            .or_else(|| slice.rfind('\n'))
            .or_else(|| slice.rfind(' '))
            .map(|pos| start + pos + 1);

        let chunk_end = break_at.unwrap_or(end);
        chunks.push(text[start..chunk_end].to_string());
        start = chunk_end;
    }

    chunks
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

    // ── Markdown chunker tests (#405) ──────────────────────────────

    #[test]
    fn test_md_simple_headings() {
        let md = "# Title 1\n\nSome content here.\n\n## Subsection\n\nMore content.";
        let chunks = chunk_markdown(md, 1024);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].heading, "Title 1");
        assert_eq!(chunks[1].heading, "Subsection");
        assert_eq!(chunks[1].level, 2);
    }

    #[test]
    fn test_md_respects_code_blocks() {
        let md = "# Code\n\n```rust\n## Not a heading\nfn main() {}\n```\n\nAfter code.";
        let chunks = chunk_markdown(md, 1024);
        // The ## inside code block should NOT create a new section.
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("## Not a heading"));
    }

    #[test]
    fn test_md_large_section_splits() {
        let para = "This is a paragraph. ".repeat(100);
        let md = format!("# Big Section\n\n{para}");
        let chunks = chunk_markdown(&md, 200);
        assert!(chunks.len() > 1);
        // First chunk should have the heading.
        assert_eq!(chunks[0].heading, "Big Section");
        // Subsequent chunks should have "part N" suffix.
        assert!(chunks[1].heading.contains("part 2"));
    }

    #[test]
    fn test_md_no_headings() {
        let text = "Just some prose.\n\nNo headings here.\n\nMore text.";
        let chunks = chunk_markdown(text, 1024);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].heading, "");
    }

    #[test]
    fn test_md_empty_input() {
        assert!(chunk_markdown("", 1024).is_empty());
        assert!(chunk_markdown("   \n\n  ", 1024).is_empty());
    }

    #[test]
    fn test_md_nested_levels() {
        let md = "# H1\n\nText A\n\n### H3\n\nText B\n\n## H2\n\nText C";
        let chunks = chunk_markdown(md, 1024);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].heading, "H1");
        assert_eq!(chunks[1].heading, "H3");
        assert_eq!(chunks[1].level, 3);
        assert_eq!(chunks[2].heading, "H2");
        assert_eq!(chunks[2].level, 2);
    }

    #[test]
    fn test_md_char_offsets() {
        let md = "# A\n\nText A\n\n# B\n\nText B";
        let chunks = chunk_markdown(md, 1024);
        assert_eq!(chunks.len(), 2);
        // Offsets should be within text bounds.
        assert_eq!(chunks[0].char_start, 0);
        assert!(chunks[0].char_end > 0);
        assert!(chunks[1].char_start >= chunks[0].char_end);
    }

    #[test]
    fn test_md_hashtag_not_heading() {
        // #hashtag (no space) should not be treated as heading.
        let md = "This has #hashtag and #another\n\nText.";
        let chunks = chunk_markdown(md, 1024);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].heading, "");
    }

    #[test]
    fn test_embed_aware_chunking() {
        // Mock embedder with small seq len for testing.
        struct MockEmbedder {
            seq_len: usize,
        }
        impl crate::embed::Embedder for MockEmbedder {
            fn embed(&self, _text: &str) -> Result<Vec<f32>, crate::Error> {
                Ok(vec![0.0; 8])
            }
            fn dims(&self) -> usize {
                8
            }
            fn max_seq_len(&self) -> usize {
                self.seq_len
            }
            fn name(&self) -> &str {
                "mock"
            }
        }

        // 256 tokens * 4 chars/token = 1024 chars.
        let embedder = MockEmbedder { seq_len: 256 };
        let long_text = "A".repeat(2000);
        let chunks = chunk_markdown_embed_aware(&long_text, &embedder);
        // Should split into chunks of ~1024 chars.
        assert!(
            chunks.len() > 1,
            "expected multiple chunks for 2000 chars with 1024 limit"
        );
        assert!(chunks[0].content.len() <= 1024);
    }
}
