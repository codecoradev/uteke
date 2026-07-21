//! Tree-sitter AST-based code chunking (gated behind the `treesitter` feature).
//!
//! Replaces the regex chunker with precise AST boundaries. Each supported
//! language declares which top-level node kinds are "chunkable" (functions,
//! structs, classes, impls, etc.) and how to read the symbol name from the
//! node. Byte spans are converted to 1-based inclusive line numbers.
//!
//! The `unsafe` code lives inside the vendored grammar crates; the code here
//! is entirely safe and honors the workspace `unsafe_code = "forbid"` lint.
//!
//! When the AST walk yields no chunks (parse error, unsupported construct),
//! the caller falls back to the regex chunker in [`crate::chunker`].

use crate::chunker::CodeChunk;
use tree_sitter::{Node, Parser};

/// A mapping from a tree-sitter node kind to a chunk symbol type, plus the
/// grammar field name that holds the symbol's identifier.
struct NodeSpec {
    /// Tree-sitter node `kind()` to treat as a chunk boundary.
    kind: &'static str,
    /// Symbol type recorded on the produced [`CodeChunk`].
    symbol_type: &'static str,
    /// Field name for the identifier child (e.g. "name"). Empty = search the
    /// first `identifier`/`type_identifier` descendant.
    name_field: &'static str,
}

/// Language-specific chunking rules.
struct LangSpec {
    language: &'static str,
    /// Node kinds that form chunk boundaries. Order matters only for docs.
    nodes: &'static [NodeSpec],
}

/// Resolve the tree-sitter [`tree_sitter::Language`] and chunking spec for a
/// uteke language name. Returns `None` when tree-sitter has no grammar wired
/// for the language (caller falls back to the regex chunker).
fn lang_for(language: &str) -> Option<(tree_sitter::Language, LangSpec)> {
    let spec = match language {
        "rust" => (
            tree_sitter_rust::LANGUAGE.into(),
            LangSpec {
                language: "rust",
                nodes: &[
                    NodeSpec { kind: "function_item", symbol_type: "function", name_field: "name" },
                    NodeSpec { kind: "struct_item", symbol_type: "struct", name_field: "name" },
                    NodeSpec { kind: "enum_item", symbol_type: "enum", name_field: "name" },
                    NodeSpec { kind: "trait_item", symbol_type: "trait", name_field: "name" },
                    NodeSpec { kind: "impl_item", symbol_type: "impl", name_field: "type" },
                    NodeSpec { kind: "mod_item", symbol_type: "module", name_field: "name" },
                    NodeSpec { kind: "macro_definition", symbol_type: "macro", name_field: "name" },
                    NodeSpec { kind: "const_item", symbol_type: "const", name_field: "name" },
                    NodeSpec { kind: "static_item", symbol_type: "static", name_field: "name" },
                    NodeSpec { kind: "type_item", symbol_type: "type", name_field: "name" },
                ],
            },
        ),
        "go" => (
            tree_sitter_go::LANGUAGE.into(),
            LangSpec {
                language: "go",
                nodes: &[
                    NodeSpec { kind: "function_declaration", symbol_type: "function", name_field: "name" },
                    NodeSpec { kind: "method_declaration", symbol_type: "method", name_field: "name" },
                    NodeSpec { kind: "type_declaration", symbol_type: "type", name_field: "" },
                ],
            },
        ),
        "python" => (
            tree_sitter_python::LANGUAGE.into(),
            LangSpec {
                language: "python",
                nodes: &[
                    NodeSpec { kind: "function_definition", symbol_type: "function", name_field: "name" },
                    NodeSpec { kind: "class_definition", symbol_type: "class", name_field: "name" },
                    NodeSpec { kind: "decorated_definition", symbol_type: "function", name_field: "" },
                ],
            },
        ),
        "typescript" => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            LangSpec {
                language: "typescript",
                nodes: TS_JS_NODES,
            },
        ),
        "javascript" => (
            tree_sitter_javascript::LANGUAGE.into(),
            LangSpec {
                language: "javascript",
                nodes: TS_JS_NODES,
            },
        ),
        "java" => (
            tree_sitter_java::LANGUAGE.into(),
            LangSpec {
                language: "java",
                nodes: &[
                    NodeSpec { kind: "class_declaration", symbol_type: "class", name_field: "name" },
                    NodeSpec { kind: "interface_declaration", symbol_type: "interface", name_field: "name" },
                    NodeSpec { kind: "enum_declaration", symbol_type: "enum", name_field: "name" },
                    NodeSpec { kind: "record_declaration", symbol_type: "record", name_field: "name" },
                    NodeSpec { kind: "method_declaration", symbol_type: "method", name_field: "name" },
                ],
            },
        ),
        "c" => (
            tree_sitter_c::LANGUAGE.into(),
            LangSpec {
                language: "c",
                nodes: &[
                    NodeSpec { kind: "function_definition", symbol_type: "function", name_field: "" },
                    NodeSpec { kind: "struct_specifier", symbol_type: "struct", name_field: "name" },
                    NodeSpec { kind: "enum_specifier", symbol_type: "enum", name_field: "name" },
                    NodeSpec { kind: "union_specifier", symbol_type: "union", name_field: "name" },
                ],
            },
        ),
        "cpp" => (
            tree_sitter_cpp::LANGUAGE.into(),
            LangSpec {
                language: "cpp",
                nodes: &[
                    NodeSpec { kind: "function_definition", symbol_type: "function", name_field: "" },
                    NodeSpec { kind: "class_specifier", symbol_type: "class", name_field: "name" },
                    NodeSpec { kind: "struct_specifier", symbol_type: "struct", name_field: "name" },
                    NodeSpec { kind: "enum_specifier", symbol_type: "enum", name_field: "name" },
                    NodeSpec { kind: "namespace_definition", symbol_type: "namespace", name_field: "name" },
                ],
            },
        ),
        _ => return None,
    };
    Some(spec)
}

/// Shared node set for TypeScript and JavaScript (same grammar shapes).
const TS_JS_NODES: &[NodeSpec] = &[
    NodeSpec { kind: "function_declaration", symbol_type: "function", name_field: "name" },
    NodeSpec { kind: "generator_function_declaration", symbol_type: "function", name_field: "name" },
    NodeSpec { kind: "class_declaration", symbol_type: "class", name_field: "name" },
    NodeSpec { kind: "interface_declaration", symbol_type: "interface", name_field: "name" },
    NodeSpec { kind: "type_alias_declaration", symbol_type: "type", name_field: "name" },
    NodeSpec { kind: "enum_declaration", symbol_type: "enum", name_field: "name" },
    // `export function foo` / `export class Bar` wrap the decl in export_statement.
    NodeSpec { kind: "export_statement", symbol_type: "export", name_field: "" },
    // `const foo = () => {}` — lexical bindings holding a function/arrow.
    NodeSpec { kind: "lexical_declaration", symbol_type: "function", name_field: "" },
];

/// Chunk source code using tree-sitter. Returns `None` if the language has no
/// grammar or parsing fails, so the caller can fall back to the regex chunker.
/// Returns `Some(vec![])` only when the file parses but has no top-level
/// definitions (caller may then store the whole file).
pub fn chunk_code_ts(content: &str, language: &str) -> Option<Vec<CodeChunk>> {
    let (ts_lang, spec) = lang_for(language)?;
    let mut parser = Parser::new();
    parser.set_language(&ts_lang).ok()?;
    let tree = parser.parse(content, None)?;
    let root = tree.root_node();
    let src = content.as_bytes();

    let mut chunks = Vec::new();
    let mut cursor = root.walk();
    // Walk only top-level children of the root: chunks are file-level defs.
    for child in root.children(&mut cursor) {
        collect_chunk(child, &spec, src, content, &mut chunks);
    }

    if chunks.is_empty() {
        // Parsed fine but nothing chunkable — let the caller decide.
        return Some(vec![]);
    }
    Some(chunks)
}

/// Produce a chunk for `node` if its kind is chunkable under `spec`. For
/// wrapper kinds (`export_statement`, `decorated_definition`,
/// `lexical_declaration`) the inner declaration is used for the symbol name
/// but the wrapper's full span is kept as content.
fn collect_chunk(
    node: Node,
    spec: &LangSpec,
    src: &[u8],
    content: &str,
    out: &mut Vec<CodeChunk>,
) {
    let Some(nodespec) = spec.nodes.iter().find(|n| n.kind == node.kind()) else {
        return;
    };

    // Resolve the symbol name.
    let name = resolve_name(node, nodespec, src).unwrap_or_else(|| "anonymous".to_string());

    // For wrapper nodes, refine the symbol type from the inner declaration.
    let symbol_type = refine_symbol_type(node, nodespec, spec);

    let start_byte = node.start_byte();
    let end_byte = node.end_byte();
    let text = &content[start_byte..end_byte];

    // Tree-sitter rows are 0-based; convert to 1-based inclusive lines.
    let line_start = node.start_position().row + 1;
    let line_end = node.end_position().row + 1;

    out.push(CodeChunk {
        content: text.to_string(),
        language: spec.language.to_string(),
        symbol_type: symbol_type.to_string(),
        symbol_name: name,
        line_start,
        line_end,
    });

    // Do not recurse into a captured chunk — methods stay part of their
    // enclosing type/impl, matching the regex chunker's file-level behavior.
    let _ = src;
}

/// Read the identifier for `node` per its [`NodeSpec`]. Falls back to the
/// first identifier-like descendant when `name_field` is empty or absent.
fn resolve_name(node: Node, nodespec: &NodeSpec, src: &[u8]) -> Option<String> {
    if !nodespec.name_field.is_empty() {
        if let Some(name_node) = node.child_by_field_name(nodespec.name_field) {
            return node_text(name_node, src);
        }
    }
    // Fallback: first identifier-ish descendant.
    first_identifier(node, src)
}

/// Refine the symbol type for wrapper nodes by inspecting the inner decl.
fn refine_symbol_type<'a>(node: Node, nodespec: &'a NodeSpec, spec: &'a LangSpec) -> &'a str {
    match node.kind() {
        "export_statement" | "decorated_definition" | "lexical_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(inner) = spec.nodes.iter().find(|n| n.kind == child.kind()) {
                    if inner.kind != node.kind() {
                        return inner.symbol_type;
                    }
                }
            }
            nodespec.symbol_type
        }
        _ => nodespec.symbol_type,
    }
}

/// Extract UTF-8 text for a node from the source bytes.
fn node_text(node: Node, src: &[u8]) -> Option<String> {
    node.utf8_text(src).ok().map(|s| s.to_string())
}

/// Depth-first search for the first identifier-like node and return its text.
fn first_identifier(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let k = child.kind();
        if k == "identifier"
            || k == "type_identifier"
            || k == "field_identifier"
            || k == "property_identifier"
        {
            return node_text(child, src);
        }
        if let Some(found) = first_identifier(child, src) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_functions_and_structs_with_line_numbers() {
        let src = "\
use std::io;

struct Point {
    x: i32,
    y: i32,
}

fn add(a: i32, b: i32) -> i32 {
    a + b
}

impl Point {
    fn origin() -> Self {
        Point { x: 0, y: 0 }
    }
}
";
        let chunks = chunk_code_ts(src, "rust").expect("grammar present");
        let names: Vec<_> = chunks.iter().map(|c| c.symbol_name.as_str()).collect();
        assert!(names.contains(&"Point"), "names={names:?}");
        assert!(names.contains(&"add"), "names={names:?}");

        let point = chunks.iter().find(|c| c.symbol_name == "Point" && c.symbol_type == "struct").unwrap();
        assert_eq!(point.line_start, 3);
        assert_eq!(point.line_end, 6);

        let add = chunks.iter().find(|c| c.symbol_name == "add").unwrap();
        assert_eq!(add.symbol_type, "function");
        assert_eq!(add.line_start, 8);
        assert_eq!(add.line_end, 10);

        // impl is captured at file level; its method stays inside it.
        let imp = chunks.iter().find(|c| c.symbol_type == "impl").unwrap();
        assert_eq!(imp.symbol_name, "Point");
        assert!(imp.content.contains("fn origin"));
    }

    #[test]
    fn python_class_and_function() {
        let src = "\
import os

def top():
    return 1

class Widget:
    def __init__(self):
        self.n = 0
";
        let chunks = chunk_code_ts(src, "python").expect("grammar present");
        let top = chunks.iter().find(|c| c.symbol_name == "top").unwrap();
        assert_eq!(top.symbol_type, "function");
        assert_eq!(top.line_start, 3);
        let widget = chunks.iter().find(|c| c.symbol_name == "Widget").unwrap();
        assert_eq!(widget.symbol_type, "class");
        assert!(widget.content.contains("__init__"));
    }

    #[test]
    fn typescript_export_and_arrow() {
        let src = "\
export function greet(name: string): string {
    return `hi ${name}`;
}

const double = (n: number) => n * 2;

interface Shape {
    area(): number;
}
";
        let chunks = chunk_code_ts(src, "typescript").expect("grammar present");
        let names: Vec<_> = chunks.iter().map(|c| c.symbol_name.as_str()).collect();
        assert!(names.contains(&"greet"), "names={names:?}");
        assert!(names.contains(&"double"), "names={names:?}");
        assert!(names.contains(&"Shape"), "names={names:?}");
    }

    #[test]
    fn go_functions_and_types() {
        let src = "\
package main

func main() {
    println(\"hi\")
}

type Server struct {
    port int
}
";
        let chunks = chunk_code_ts(src, "go").expect("grammar present");
        let names: Vec<_> = chunks.iter().map(|c| c.symbol_name.as_str()).collect();
        assert!(names.contains(&"main"), "names={names:?}");
        assert!(names.contains(&"Server"), "names={names:?}");
    }

    #[test]
    fn unsupported_language_returns_none() {
        assert!(chunk_code_ts("x = 1", "cobol").is_none());
    }

    #[test]
    fn empty_defs_returns_some_empty() {
        // Valid Rust with no top-level defs -> Some(empty) so caller falls back.
        let chunks = chunk_code_ts("// just a comment\n", "rust").unwrap();
        assert!(chunks.is_empty());
    }
}
