//! Bounds on how deeply a hostile file can make a parser recurse. The Rust
//! and JavaScript parsers recurse once per nesting level: a bracket, a
//! prefix operator, an assignment, a closure or arrow, an `else if`. A file
//! built to nest deeper than the scan's stack holds is refused before it
//! reaches a parser, so validation fails with a finding rather than a
//! crash (REG-022).

/// The language of a file, for its comment, string and keyword rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Language {
    /// Rust.
    Rust,
    /// JavaScript.
    Script,
}

/// How deep brackets may nest.
pub(crate) const MAX_BRACKETS: usize = 128;

/// How many nesting operators and keywords one path through the file may
/// hold, counting every enclosing statement's.
pub(crate) const MAX_OPERATORS: usize = 2048;

const RUST_KEYWORDS: &[&str] = &[
    "return", "break", "move", "async", "yield", "box", "impl", "dyn", "if", "else", "let", "mut",
    "ref", "as", "await", "unsafe", "static",
];

const SCRIPT_KEYWORDS: &[&str] = &[
    "typeof",
    "void",
    "delete",
    "await",
    "new",
    "yield",
    "if",
    "else",
    "class",
    "extends",
    "async",
    "function",
    "return",
    "throw",
    "in",
    "of",
    "instanceof",
];

/// Why a file was refused, and on which line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TooDeep {
    /// The 1-based line.
    pub line: u32,
    /// What exceeded its bound.
    pub message: String,
}

/// Checks a file against the nesting bounds.
pub(crate) fn check(text: &str, language: Language) -> Result<(), TooDeep> {
    let keywords = match language {
        Language::Rust => RUST_KEYWORDS,
        Language::Script => SCRIPT_KEYWORDS,
    };
    let chars: Vec<char> = text.chars().collect();
    let mut line = 1u32;
    let mut index = 0;
    // The operator count of each enclosing segment, and of the current
    // one; a segment ends at `;` or `,`.
    let mut stack: Vec<usize> = Vec::new();
    let mut enclosing = 0usize;
    let mut segment = 0usize;
    let too_deep = |line: u32, message: String| Err(TooDeep { line, message });
    while index < chars.len() {
        let c = chars[index];
        let next = chars.get(index + 1).copied();
        match c {
            '\n' => {
                line = line.saturating_add(1);
                index += 1;
                continue;
            }
            '/' if next == Some('/') => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
                continue;
            }
            '/' if next == Some('*') => {
                index += 2;
                while index < chars.len()
                    && !(chars[index] == '*' && chars.get(index + 1) == Some(&'/'))
                {
                    if chars[index] == '\n' {
                        line = line.saturating_add(1);
                    }
                    index += 1;
                }
                index += 2;
                continue;
            }
            '"' | '`' => {
                index += 1;
                while index < chars.len() && chars[index] != c {
                    if chars[index] == '\\' {
                        index += 1;
                    } else if chars[index] == '\n' {
                        line = line.saturating_add(1);
                    } else if c == '`' && chars[index] == '$' && chars.get(index + 1) == Some(&'{')
                    {
                        // A template substitution is code: let the
                        // bracket count see its brace.
                        break;
                    }
                    index += 1;
                }
                if index < chars.len() && chars[index] == c {
                    index += 1;
                }
                continue;
            }
            '\'' if language == Language::Script => {
                index += 1;
                while index < chars.len() && chars[index] != '\'' && chars[index] != '\n' {
                    if chars[index] == '\\' {
                        index += 1;
                    }
                    index += 1;
                }
                index += 1;
                continue;
            }
            '(' | '[' | '{' => {
                stack.push(segment);
                enclosing += segment;
                segment = 0;
                if stack.len() > MAX_BRACKETS {
                    return too_deep(
                        line,
                        format!("brackets nest deeper than {MAX_BRACKETS} levels"),
                    );
                }
            }
            ')' | ']' | '}' => {
                if let Some(outer) = stack.pop() {
                    enclosing -= outer;
                    segment = outer;
                }
            }
            ';' | ',' => segment = 0,
            '!' | '-' | '+' | '~' | '*' | '&' | '|' | '=' | '<' | '?' | ':' | '>' | '.' | '^'
            | '%' => {
                segment += 1;
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = index;
                while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_')
                {
                    index += 1;
                }
                let word: String = chars[start..index].iter().collect();
                if keywords.contains(&word.as_str()) {
                    segment += 1;
                }
                if enclosing + segment > MAX_OPERATORS {
                    return too_deep(
                        line,
                        format!("more than {MAX_OPERATORS} nesting operators on one path"),
                    );
                }
                continue;
            }
            _ => {}
        }
        if enclosing + segment > MAX_OPERATORS {
            return too_deep(
                line,
                format!("more than {MAX_OPERATORS} nesting operators on one path"),
            );
        }
        index += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Language, check};

    #[test]
    fn ordinary_code_passes() {
        let rust = "fn main() { let x = (1 + 2) * 3; if x > 2 { println!(\"{x}\"); } }";
        assert!(check(rust, Language::Rust).is_ok());
        let script = "const a = (b) => b ? 1 : 2; for (const c of [1, 2]) { a(c); }";
        assert!(check(script, Language::Script).is_ok());
    }

    #[test]
    fn deep_brackets_and_long_operator_runs_are_refused() {
        assert!(check(&"(".repeat(200), Language::Script).is_err());
        assert!(check(&format!("x = {}1;", "!".repeat(5000)), Language::Rust).is_err());
        assert!(check(&format!("x = {}1;", "a => ".repeat(5000)), Language::Script).is_err());
        assert!(check(&"if x {} else ".repeat(5000), Language::Rust).is_err());
    }
}
