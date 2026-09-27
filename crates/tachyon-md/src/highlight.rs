//! Syntax highlighting for fenced code blocks: a small lexer that knows each language's keywords,
//! comments and string quotes, and marks keywords, strings, comments, numbers, function names and
//! type-like names. It runs as part of parsing a block (microseconds for a screenful of code), so
//! it adds no dependency and no second pass.

use std::ops::Range;

use crate::ir::Style;

struct Lang {
    keywords: &'static [&'static str],
    /// Line comment starts.
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// String quote characters; `"""` and `'''` also open multi-line strings where `triple`.
    quotes: &'static [u8],
    triple: bool,
    /// Capitalized names are types (not in shells or data formats).
    types: bool,
}

const C_COMMENTS: &[&str] = &["//"];
const HASH: &[&str] = &["#"];

const RUST: Lang = Lang {
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
        "true", "type", "unsafe", "use", "where", "while", "yield",
    ],
    line_comments: C_COMMENTS,
    block_comment: Some(("/*", "*/")),
    quotes: b"\"",
    triple: false,
    types: true,
};

const PYTHON: Lang = Lang {
    keywords: &[
        "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
        "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
        "if", "import", "in", "is", "lambda", "match", "case", "nonlocal", "not", "or", "pass",
        "raise", "return", "self", "try", "while", "with", "yield",
    ],
    line_comments: HASH,
    block_comment: None,
    quotes: b"\"'",
    triple: true,
    types: true,
};

const JAVASCRIPT: Lang = Lang {
    keywords: &[
        "abstract",
        "as",
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "declare",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "from",
        "function",
        "if",
        "implements",
        "import",
        "in",
        "instanceof",
        "interface",
        "keyof",
        "let",
        "new",
        "null",
        "of",
        "private",
        "protected",
        "public",
        "readonly",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "type",
        "typeof",
        "undefined",
        "var",
        "void",
        "while",
        "with",
        "yield",
    ],
    line_comments: C_COMMENTS,
    block_comment: Some(("/*", "*/")),
    quotes: b"\"'`",
    triple: false,
    types: true,
};

const C_LIKE: Lang = Lang {
    keywords: &[
        "abstract",
        "auto",
        "bool",
        "break",
        "case",
        "catch",
        "char",
        "class",
        "const",
        "constexpr",
        "continue",
        "default",
        "delete",
        "do",
        "double",
        "else",
        "enum",
        "explicit",
        "extends",
        "extern",
        "false",
        "final",
        "float",
        "for",
        "foreach",
        "func",
        "go",
        "goto",
        "if",
        "implements",
        "import",
        "in",
        "inline",
        "int",
        "interface",
        "internal",
        "long",
        "namespace",
        "new",
        "null",
        "nullptr",
        "override",
        "package",
        "private",
        "protected",
        "public",
        "return",
        "short",
        "signed",
        "sizeof",
        "static",
        "struct",
        "switch",
        "template",
        "this",
        "throw",
        "throws",
        "true",
        "try",
        "typedef",
        "typename",
        "union",
        "unsigned",
        "using",
        "var",
        "virtual",
        "void",
        "volatile",
        "while",
        "defer",
        "chan",
        "select",
        "range",
        "map",
        "fun",
        "val",
        "when",
        "object",
        "let",
        "guard",
        "self",
    ],
    line_comments: C_COMMENTS,
    block_comment: Some(("/*", "*/")),
    quotes: b"\"'",
    triple: false,
    types: true,
};

const SHELL: Lang = Lang {
    keywords: &[
        "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if",
        "in", "local", "return", "then", "until", "while", "echo", "cd", "sudo",
    ],
    line_comments: HASH,
    block_comment: None,
    quotes: b"\"'",
    triple: false,
    types: false,
};

const POWERSHELL: Lang = Lang {
    keywords: &[
        "begin", "break", "catch", "class", "continue", "do", "else", "elseif", "end", "exit",
        "filter", "finally", "for", "foreach", "function", "if", "in", "param", "process",
        "return", "switch", "throw", "trap", "try", "until", "while", "true", "false", "null",
    ],
    line_comments: HASH,
    block_comment: Some(("<#", "#>")),
    quotes: b"\"'",
    triple: false,
    types: false,
};

const SQL: Lang = Lang {
    keywords: &[
        "select", "from", "where", "and", "or", "not", "insert", "into", "values", "update", "set",
        "delete", "create", "table", "drop", "alter", "join", "left", "right", "inner", "outer",
        "on", "group", "by", "order", "having", "limit", "as", "null", "is", "in", "distinct",
        "union", "primary", "key", "index", "SELECT", "FROM", "WHERE", "AND", "OR", "NOT",
        "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE", "CREATE", "TABLE", "DROP", "ALTER",
        "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "ON", "GROUP", "BY", "ORDER", "HAVING", "LIMIT",
        "AS", "NULL", "IS", "IN", "DISTINCT", "UNION", "PRIMARY", "KEY",
    ],
    line_comments: &["--"],
    block_comment: Some(("/*", "*/")),
    quotes: b"'\"",
    triple: false,
    types: false,
};

const DATA: Lang = Lang {
    keywords: &["true", "false", "null", "yes", "no"],
    line_comments: HASH,
    block_comment: None,
    quotes: b"\"'",
    triple: false,
    types: false,
};

const JSON: Lang = Lang {
    keywords: &["true", "false", "null"],
    line_comments: C_COMMENTS,
    block_comment: None,
    quotes: b"\"",
    triple: false,
    types: false,
};

fn lang(name: &str) -> Option<&'static Lang> {
    Some(match name.to_ascii_lowercase().as_str() {
        "rust" | "rs" => &RUST,
        "python" | "py" | "python3" => &PYTHON,
        "javascript" | "js" | "jsx" | "mjs" | "cjs" | "typescript" | "ts" | "tsx" => &JAVASCRIPT,
        "c" | "h" | "cpp" | "c++" | "cc" | "cxx" | "hpp" | "java" | "cs" | "csharp" | "c#"
        | "go" | "golang" | "kotlin" | "kt" | "swift" | "scala" | "dart" | "php" => &C_LIKE,
        "bash" | "sh" | "shell" | "zsh" | "fish" | "console" => &SHELL,
        "powershell" | "ps1" | "pwsh" | "ps" => &POWERSHELL,
        "sql" => &SQL,
        "toml" | "yaml" | "yml" | "ini" | "conf" | "dockerfile" | "makefile" | "make" => &DATA,
        "json" | "jsonc" | "json5" => &JSON,
        _ => return None,
    })
}

/// Token styles for `code` written in `lang` (a fence's info string word), as byte ranges into
/// `code` in order. Empty for languages it does not know.
pub(crate) fn highlight(lang_name: &str, code: &str) -> Vec<(Range<usize>, Style)> {
    let Some(lang) = lang(lang_name) else { return Vec::new() };
    let bytes = code.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    let starts = |at: usize, token: &str| bytes[at..].starts_with(token.as_bytes());
    while at < bytes.len() {
        let b = bytes[at];
        if lang.line_comments.iter().any(|c| starts(at, c)) {
            let end = code[at..].find('\n').map_or(bytes.len(), |i| at + i);
            tokens.push((at..end, Style::COMMENT));
            at = end;
        } else if let Some((open, close)) = lang.block_comment.filter(|(open, _)| starts(at, open))
        {
            let body = at + open.len();
            let end = code[body..].find(close).map_or(bytes.len(), |i| body + i + close.len());
            tokens.push((at..end, Style::COMMENT));
            at = end;
        } else if lang.quotes.contains(&b) {
            let end = string_end(code, at, lang.triple);
            tokens.push((at..end, Style::STRING));
            at = end;
        } else if b.is_ascii_digit() {
            let end = at
                + bytes[at..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_' || **c == b'.')
                    .count();
            tokens.push((at..end, Style::NUMBER));
            at = end;
        } else if b.is_ascii_alphabetic() || b == b'_' {
            let end = at
                + bytes[at..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
                    .count();
            let word = &code[at..end];
            let next = bytes[end..].iter().find(|c| **c != b' ').copied();
            let style = if lang.keywords.contains(&word) {
                Some(Style::KEYWORD)
            } else if next == Some(b'(') || (next == Some(b'!') && std::ptr::eq(lang, &RUST)) {
                Some(Style::FUNCTION)
            } else if lang.types && word.starts_with(|c: char| c.is_ascii_uppercase()) {
                Some(Style::TYPE)
            } else {
                None
            };
            if let Some(style) = style {
                tokens.push((at..end, style));
            }
            at = end;
        } else {
            // Skip one character (whole UTF-8 sequence).
            at += code[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    tokens
}

/// End of the string starting with the quote at `start`: after the closing quote (backslash
/// escapes skip a character), or at the end of the line for an unclosed one. With `triple`, `"""`
/// and `'''` strings may span lines.
fn string_end(code: &str, start: usize, triple: bool) -> usize {
    let bytes = code.as_bytes();
    let quote = bytes[start];
    if triple && bytes[start..].starts_with(&[quote; 3]) {
        let body = start + 3;
        let close = [quote; 3];
        return bytes[body..]
            .windows(3)
            .position(|w| w == close)
            .map_or(bytes.len(), |i| body + i + 3);
    }
    let multiline = quote == b'`';
    let mut at = start + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'\n' if !multiline => return at,
            c if c == quote => return at + 1,
            _ => at += 1,
        }
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens<'a>(lang: &str, code: &'a str) -> Vec<(&'a str, Style)> {
        highlight(lang, code).into_iter().map(|(r, s)| (&code[r], s)).collect()
    }

    #[test]
    fn rust() {
        assert_eq!(
            tokens("rust", "fn main() { let x = \"a\\\"b\"; // hi\n    println!(\"{x}\", 42u8); }"),
            [
                ("fn", Style::KEYWORD),
                ("main", Style::FUNCTION),
                ("let", Style::KEYWORD),
                ("\"a\\\"b\"", Style::STRING),
                ("// hi", Style::COMMENT),
                ("println", Style::FUNCTION),
                ("\"{x}\"", Style::STRING),
                ("42u8", Style::NUMBER),
            ]
        );
    }

    #[test]
    fn python_triple_strings_span_lines_and_hash_comments() {
        assert_eq!(
            tokens("py", "def f():\n    \"\"\"doc\nmore\"\"\"\n    return None  # done"),
            [
                ("def", Style::KEYWORD),
                ("f", Style::FUNCTION),
                ("\"\"\"doc\nmore\"\"\"", Style::STRING),
                ("return", Style::KEYWORD),
                ("None", Style::KEYWORD),
                ("# done", Style::COMMENT),
            ]
        );
    }

    #[test]
    fn unclosed_strings_and_comments_end_sensibly() {
        assert_eq!(
            tokens("js", "let s = 'open\nlet t"),
            [("let", Style::KEYWORD), ("'open", Style::STRING), ("let", Style::KEYWORD),]
        );
        assert_eq!(tokens("c", "/* never closed"), [("/* never closed", Style::COMMENT)]);
    }

    #[test]
    fn unknown_languages_are_plain() {
        assert!(tokens("brainfuck", "fn main() {}").is_empty());
        assert!(tokens("", "fn main() {}").is_empty());
    }
}
