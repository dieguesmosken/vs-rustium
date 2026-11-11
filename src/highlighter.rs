use glyphon::Color;
use tree_sitter::{Language, Parser, Query, QueryCursor};

// Query canônica e 100% compatível com a gramática do tree-sitter-rust
const RUST_HIGHLIGHT_QUERY: &str = r#"
    [
        "as" "async" "await" "break" "const" "continue" "dyn" "else"
        "enum" "fn" "for" "if" "impl" "in" "let" "loop" "match"
        "mod" "move" "pub" "ref" "return" "static" "struct" "trait"
        "type" "unsafe" "use" "where" "while"
    ] @keyword

    (mutable_specifier) @keyword

    [
        (boolean_literal)
        (primitive_type)
    ] @type.builtin

    (type_identifier) @type

    (function_item name: (identifier) @function)
    (call_expression function: (identifier) @function)
    (call_expression function: (field_expression field: (field_identifier) @function))

    [
        (string_literal)
        (raw_string_literal)
        (char_literal)
    ] @string

    [
        (line_comment)
        (block_comment)
    ] @comment

    (integer_literal) @number
    (float_literal) @number

    (macro_invocation macro: (identifier) @macro)
"#;

pub struct HighlightSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub color: Color,
}

pub struct SyntaxHighlighter {
    parser: Parser,
    query: Query,
}

impl SyntaxHighlighter {
    pub fn new() -> Self {
        let language: Language = tree_sitter_rust::language();
        let mut parser = Parser::new();
        parser
            .set_language(language)
            .expect("Erro ao carregar a gramática Rust");

        let query = Query::new(language, RUST_HIGHLIGHT_QUERY)
            .expect("Erro ao compilar a query do Tree-sitter");

        Self { parser, query }
    }

    pub fn highlight(&mut self, source_code: &str) -> Vec<HighlightSpan> {
        let tree = match self.parser.parse(source_code, None) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(&self.query, tree.root_node(), source_code.as_bytes());

        let mut spans = Vec::new();

        for m in matches {
            for capture in m.captures {
                let capture_name = &self.query.capture_names()[capture.index as usize];
                let color = match capture_name.as_str() {
                    "keyword" => Color::rgb(86, 156, 214),       // #569CD6 (Azul VS Code)
                    "type" | "type.builtin" => Color::rgb(78, 201, 176), // #4EC9B0 (Ciano)
                    "function" | "macro" => Color::rgb(220, 220, 170), // #DCDCAA (Amarelo)
                    "string" => Color::rgb(206, 145, 120),       // #CE9178 (Laranja)
                    "comment" => Color::rgb(106, 153, 85),       // #6A9955 (Verde)
                    "number" => Color::rgb(181, 206, 168),       // #B5CEA8 (Verde claro)
                    _ => Color::rgb(220, 220, 220),              // #D4D4D4 (Branco padrão)
                };

                spans.push(HighlightSpan {
                    start_byte: capture.node.start_byte(),
                    end_byte: capture.node.end_byte(),
                    color,
                });
            }
        }

        spans.sort_by_key(|s| s.start_byte);
        spans
    }
}