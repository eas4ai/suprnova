//! Bounds on how deeply a hostile file can make a parser recurse. The Rust
//! and JavaScript parsers recurse once per nesting level: a bracket, a
//! prefix operator, an assignment, a closure or arrow, an `else if`. A file
//! built to nest deeper than the scan's stack holds is refused before it
//! reaches a parser, so validation fails with a finding rather than a
//! crash (REG-022).
//!
//! The count is only sound if this module lexes each file as its parser
//! does: text it wrongly reads as a string, a comment or a regular
//! expression is text it does not count. So each language gets its own
//! lexer: Rust's char literals, lifetimes, raw strings and nested block
//! comments, and JavaScript's template literals, line terminators and
//! regular expressions. Where JavaScript's grammar alone cannot tell a
//! regular expression from a division (`/` right after `}`, `++` or a
//! contextual keyword), and wherever a literal is malformed, the file is
//! refused rather than guessed at.

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

/// Reserved words after which a `/` begins a regular expression.
const REGEX_KEYWORDS: &[&str] = &[
    "return",
    "typeof",
    "case",
    "do",
    "else",
    "in",
    "instanceof",
    "new",
    "delete",
    "void",
    "throw",
    "yield",
    "await",
    "extends",
];

/// Words that are keywords only in some positions, and words a module
/// reserves: after one, the grammar alone cannot tell whether a `/` divides
/// or begins a regular expression.
const CONTEXTUAL_WORDS: &[&str] = &[
    "of",
    "let",
    "static",
    "async",
    "get",
    "set",
    "from",
    "as",
    "implements",
    "interface",
    "package",
    "private",
    "protected",
    "public",
    "enum",
    "accessor",
    "target",
    "meta",
    "type",
    "satisfies",
];

/// Why a file was refused, and on which line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TooDeep {
    /// The 1-based line.
    pub line: u32,
    /// What exceeded its bound, or which literal the lexer cannot read.
    pub message: String,
}

/// Checks a file against the nesting bounds.
pub(crate) fn check(text: &str, language: Language) -> Result<(), TooDeep> {
    let chars: Vec<char> = text.chars().collect();
    let mut lexer = Lexer {
        chars,
        index: 0,
        line: 1,
        counter: Counter::default(),
    };
    match language {
        Language::Rust => lexer.rust(),
        Language::Script => lexer.script(),
    }
}

/// The running nesting count.
#[derive(Default)]
struct Counter {
    /// The operator count of each enclosing segment.
    stack: Vec<usize>,
    /// The sum of `stack`.
    enclosing: usize,
    /// The operator count since the last `;` or `,` at this level.
    segment: usize,
}

struct Lexer {
    chars: Vec<char>,
    index: usize,
    line: u32,
    counter: Counter,
}

/// The token before a `/` in JavaScript, which decides whether the `/`
/// begins a regular expression.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Previous {
    /// A value: a `/` is division.
    Operand,
    /// An operator, an opening bracket, a separator or a reserved word: a
    /// `/` begins a regular expression.
    ExpressionStart,
    /// A token after which the grammar alone cannot decide: `}`, `++`,
    /// `--`, `.`, a contextual keyword or an escaped identifier.
    Ambiguous,
}

/// What a JavaScript bracket opened.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    /// `(`, and whether it holds an `if`, `while`, `for` or `with`
    /// condition, after which a `/` begins a regular expression.
    Paren { condition: bool },
    /// `[`.
    Bracket,
    /// `{`.
    Brace,
    /// `${` inside a template literal.
    Template,
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

impl Lexer {
    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.index + offset).copied()
    }

    fn fail<T>(&self, message: &str) -> Result<T, TooDeep> {
        Err(TooDeep {
            line: self.line,
            message: message.to_string(),
        })
    }

    fn bounds(&self) -> Result<(), TooDeep> {
        if self.counter.stack.len() > MAX_BRACKETS {
            return self.fail(&format!("brackets nest deeper than {MAX_BRACKETS} levels"));
        }
        if self.counter.enclosing + self.counter.segment > MAX_OPERATORS {
            return self.fail(&format!(
                "more than {MAX_OPERATORS} nesting operators on one path"
            ));
        }
        Ok(())
    }

    fn open(&mut self) -> Result<(), TooDeep> {
        self.counter.stack.push(self.counter.segment);
        self.counter.enclosing += self.counter.segment;
        self.counter.segment = 0;
        self.bounds()
    }

    fn close(&mut self) {
        if let Some(outer) = self.counter.stack.pop() {
            self.counter.enclosing -= outer;
            self.counter.segment = outer;
        }
    }

    fn operator(&mut self) -> Result<(), TooDeep> {
        self.counter.segment += 1;
        self.bounds()
    }

    /// Advances past one character, counting a newline.
    fn bump(&mut self) {
        if self.peek(0) == Some('\n') {
            self.line = self.line.saturating_add(1);
        }
        self.index += 1;
    }

    fn word(&mut self) -> String {
        let start = self.index;
        while self
            .peek(0)
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '\\'))
        {
            self.index += 1;
        }
        self.chars[start..self.index].iter().collect()
    }

    // ----- Rust ---------------------------------------------------------

    fn rust(&mut self) -> Result<(), TooDeep> {
        if self.peek(0) == Some('#') && self.peek(1) == Some('!') && self.peek(2) != Some('[') {
            self.skip_line();
        }
        while let Some(c) = self.peek(0) {
            match c {
                '/' if self.peek(1) == Some('/') => self.skip_line(),
                '/' if self.peek(1) == Some('*') => self.rust_block_comment()?,
                '"' => self.rust_string()?,
                '\'' => self.rust_quote()?,
                c if c.is_alphabetic() || c == '_' => {
                    let word = self.word();
                    if matches!(word.as_str(), "r" | "br" | "cr")
                        && matches!(self.peek(0), Some('"' | '#'))
                    {
                        self.rust_raw_string()?;
                    } else if RUST_KEYWORDS.contains(&word.as_str()) {
                        self.operator()?;
                    }
                }
                c if c.is_ascii_digit() => {
                    while self
                        .peek(0)
                        .is_some_and(|c| c.is_alphanumeric() || c == '_')
                    {
                        self.index += 1;
                    }
                }
                '(' | '[' | '{' => {
                    self.index += 1;
                    self.open()?;
                }
                ')' | ']' | '}' => {
                    self.index += 1;
                    self.close();
                }
                ';' | ',' => {
                    self.index += 1;
                    self.counter.segment = 0;
                }
                '!' | '-' | '+' | '~' | '*' | '&' | '|' | '=' | '<' | '?' | ':' | '>' | '.'
                | '^' | '%' | '/' => {
                    self.index += 1;
                    self.operator()?;
                }
                _ => self.bump(),
            }
        }
        Ok(())
    }

    fn skip_line(&mut self) {
        while self.peek(0).is_some_and(|c| c != '\n') {
            self.index += 1;
        }
    }

    /// A block comment, which nests in Rust.
    fn rust_block_comment(&mut self) -> Result<(), TooDeep> {
        self.index += 2;
        let mut depth = 1usize;
        while depth > 0 {
            match (self.peek(0), self.peek(1)) {
                (Some('/'), Some('*')) => {
                    depth += 1;
                    self.index += 2;
                }
                (Some('*'), Some('/')) => {
                    depth -= 1;
                    self.index += 2;
                }
                (Some(_), _) => self.bump(),
                (None, _) => return self.fail("an unterminated block comment"),
            }
        }
        Ok(())
    }

    fn rust_string(&mut self) -> Result<(), TooDeep> {
        self.index += 1;
        loop {
            match self.peek(0) {
                Some('"') => {
                    self.index += 1;
                    return Ok(());
                }
                Some('\\') => {
                    self.index += 1;
                    if self.peek(0).is_some() {
                        self.bump();
                    }
                }
                Some(_) => self.bump(),
                None => return self.fail("an unterminated string literal"),
            }
        }
    }

    /// A raw string after its `r`, `br` or `cr`: `#`s, a quote, and the
    /// content up to a quote followed by as many `#`s. `r#ident` is a raw
    /// identifier, not a string.
    fn rust_raw_string(&mut self) -> Result<(), TooDeep> {
        let mut hashes = 0usize;
        while self.peek(hashes) == Some('#') {
            hashes += 1;
        }
        if self.peek(hashes) != Some('"') {
            // A raw identifier: its name is lexed as a word next.
            self.index += hashes;
            return Ok(());
        }
        self.index += hashes + 1;
        loop {
            match self.peek(0) {
                Some('"') if (1..=hashes).all(|offset| self.peek(offset) == Some('#')) => {
                    self.index += 1 + hashes;
                    return Ok(());
                }
                Some(_) => self.bump(),
                None => return self.fail("an unterminated raw string literal"),
            }
        }
    }

    /// A `'`: a char literal (`'x'`, `'\''`, `'\u{1F600}'`), or a lifetime
    /// or label (`'a`, `'static`), whose name is lexed as a word next.
    fn rust_quote(&mut self) -> Result<(), TooDeep> {
        match (self.peek(1), self.peek(2)) {
            (Some('\\'), _) => {
                // An escape: `\` and its character, then up to the closing
                // quote, which a `\u{...}` escape puts a few characters on.
                self.index += 3;
                for _ in 0..10 {
                    match self.peek(0) {
                        Some('\'') => {
                            self.index += 1;
                            return Ok(());
                        }
                        Some(c) if !is_line_terminator(c) => self.index += 1,
                        _ => break,
                    }
                }
                self.fail("a malformed char literal")
            }
            (Some(c), Some('\'')) if !is_line_terminator(c) => {
                self.index += 3;
                Ok(())
            }
            (Some(c), _) if c.is_alphabetic() || c == '_' => {
                self.index += 1;
                Ok(())
            }
            _ => self.fail("a malformed char literal"),
        }
    }

    // ----- JavaScript -----------------------------------------------------

    fn script(&mut self) -> Result<(), TooDeep> {
        if self.peek(0) == Some('#') && self.peek(1) == Some('!') {
            self.skip_script_line();
        }
        let mut previous = Previous::ExpressionStart;
        let mut opens: Vec<Open> = Vec::new();
        // The last two reserved words or identifiers, for `if (`, `for (`,
        // `for await (`.
        let mut last_words: [Option<String>; 2] = [None, None];
        // Whether the last token was `.` or `?.`, after which a word is a
        // property name, never a keyword.
        let mut after_dot = false;
        while let Some(c) = self.peek(0) {
            let mut word_token = None;
            let mut dot = false;
            match c {
                c if is_line_terminator(c) || c.is_whitespace() => {
                    self.bump();
                    continue;
                }
                '/' if self.peek(1) == Some('/') => {
                    self.skip_script_line();
                    continue;
                }
                '/' if self.peek(1) == Some('*') => {
                    self.script_block_comment()?;
                    continue;
                }
                '/' => match previous {
                    Previous::ExpressionStart => {
                        self.script_regex()?;
                        previous = Previous::Operand;
                    }
                    Previous::Operand => {
                        self.index += 1;
                        if self.peek(0) == Some('=') {
                            self.index += 1;
                        }
                        self.operator()?;
                        previous = Previous::ExpressionStart;
                    }
                    Previous::Ambiguous => {
                        return self.fail(
                            "a `/` the grammar alone cannot read as a division or a regular expression",
                        );
                    }
                },
                '"' | '\'' => {
                    self.script_string(c)?;
                    previous = Previous::Operand;
                }
                '`' => {
                    self.index += 1;
                    if self.script_template(&mut opens)? {
                        previous = Previous::ExpressionStart;
                    } else {
                        previous = Previous::Operand;
                    }
                }
                '#' if self
                    .peek(1)
                    .is_some_and(|c| c.is_alphabetic() || matches!(c, '_' | '$')) =>
                {
                    self.index += 1;
                    self.word();
                    previous = Previous::Operand;
                }
                c if c.is_alphabetic() || matches!(c, '_' | '$' | '\\') || !c.is_ascii() => {
                    let word = self.word();
                    if word.is_empty() {
                        self.index += 1;
                        continue;
                    }
                    if after_dot {
                        previous = Previous::Operand;
                    } else {
                        if SCRIPT_KEYWORDS.contains(&word.as_str()) {
                            self.operator()?;
                        }
                        previous =
                            if word.contains('\\') || CONTEXTUAL_WORDS.contains(&word.as_str()) {
                                Previous::Ambiguous
                            } else if REGEX_KEYWORDS.contains(&word.as_str()) {
                                Previous::ExpressionStart
                            } else {
                                Previous::Operand
                            };
                        word_token = Some(word);
                    }
                }
                c if c.is_ascii_digit()
                    || (c == '.' && self.peek(1).is_some_and(|d| d.is_ascii_digit())) =>
                {
                    while self
                        .peek(0)
                        .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.'))
                    {
                        self.index += 1;
                    }
                    previous = Previous::Operand;
                }
                '(' => {
                    let condition = matches!(
                        last_words[1].as_deref(),
                        Some("if" | "while" | "for" | "with")
                    ) || (last_words[1].as_deref() == Some("await")
                        && last_words[0].as_deref() == Some("for"));
                    self.index += 1;
                    opens.push(Open::Paren { condition });
                    self.open()?;
                    previous = Previous::ExpressionStart;
                }
                '[' | '{' => {
                    self.index += 1;
                    opens.push(if c == '[' { Open::Bracket } else { Open::Brace });
                    self.open()?;
                    previous = Previous::ExpressionStart;
                }
                ')' | ']' | '}' => {
                    self.index += 1;
                    let expected = match c {
                        ')' => "paren",
                        ']' => "bracket",
                        _ => "brace",
                    };
                    let Some(open) = opens.pop() else {
                        return self.fail("a closing bracket without its opening one");
                    };
                    self.close();
                    previous = match (open, expected) {
                        (Open::Paren { condition: true }, "paren") => Previous::ExpressionStart,
                        (Open::Paren { condition: false }, "paren")
                        | (Open::Bracket, "bracket") => Previous::Operand,
                        (Open::Brace, "brace") => Previous::Ambiguous,
                        (Open::Template, "brace") => {
                            if self.script_template(&mut opens)? {
                                Previous::ExpressionStart
                            } else {
                                Previous::Operand
                            }
                        }
                        _ => return self.fail("brackets that do not match"),
                    };
                }
                ';' | ',' => {
                    self.index += 1;
                    self.counter.segment = 0;
                    previous = Previous::ExpressionStart;
                }
                '+' | '-' if self.peek(1) == Some(c) => {
                    self.index += 2;
                    self.operator()?;
                    previous = Previous::Ambiguous;
                }
                '.' if self.peek(1) == Some('.') && self.peek(2) == Some('.') => {
                    self.index += 3;
                    self.operator()?;
                    previous = Previous::ExpressionStart;
                }
                '.' => {
                    self.index += 1;
                    self.operator()?;
                    previous = Previous::Ambiguous;
                    dot = true;
                }
                '?' if self.peek(1) == Some('.')
                    && !self.peek(2).is_some_and(|d| d.is_ascii_digit()) =>
                {
                    self.index += 2;
                    self.operator()?;
                    previous = Previous::Ambiguous;
                    dot = true;
                }
                '!' | '-' | '+' | '~' | '*' | '&' | '|' | '=' | '<' | '?' | ':' | '>' | '^'
                | '%' | '@' => {
                    self.index += 1;
                    self.operator()?;
                    previous = Previous::ExpressionStart;
                }
                _ => {
                    self.index += 1;
                    previous = Previous::Ambiguous;
                }
            }
            last_words = [last_words[1].take(), word_token];
            after_dot = dot;
        }
        if !opens.is_empty() {
            return self.fail("brackets left open at the end of the file");
        }
        Ok(())
    }

    fn skip_script_line(&mut self) {
        while self.peek(0).is_some_and(|c| !is_line_terminator(c)) {
            self.index += 1;
        }
    }

    fn script_block_comment(&mut self) -> Result<(), TooDeep> {
        self.index += 2;
        loop {
            match (self.peek(0), self.peek(1)) {
                (Some('*'), Some('/')) => {
                    self.index += 2;
                    return Ok(());
                }
                (Some(_), _) => self.bump(),
                (None, _) => return self.fail("an unterminated block comment"),
            }
        }
    }

    /// A `'` or `"` string, which no line terminator but U+2028 and U+2029
    /// may end unescaped.
    fn script_string(&mut self, quote: char) -> Result<(), TooDeep> {
        self.index += 1;
        loop {
            match self.peek(0) {
                Some(c) if c == quote => {
                    self.index += 1;
                    return Ok(());
                }
                Some('\\') => {
                    self.index += 1;
                    if self.peek(0) == Some('\r') && self.peek(1) == Some('\n') {
                        self.index += 1;
                    }
                    if self.peek(0).is_some() {
                        self.bump();
                    }
                }
                Some('\n' | '\r') | None => return self.fail("an unterminated string literal"),
                Some(_) => self.index += 1,
            }
        }
    }

    /// The rest of a template literal, from after its opening backquote or
    /// a substitution's closing brace. Returns `true` when it stops at a
    /// `${`, which it pushes as an opening bracket, and `false` at its end.
    fn script_template(&mut self, opens: &mut Vec<Open>) -> Result<bool, TooDeep> {
        loop {
            match self.peek(0) {
                Some('`') => {
                    self.index += 1;
                    return Ok(false);
                }
                Some('\\') => {
                    self.index += 1;
                    if self.peek(0).is_some() {
                        self.bump();
                    }
                }
                Some('$') if self.peek(1) == Some('{') => {
                    self.index += 2;
                    opens.push(Open::Template);
                    self.open()?;
                    return Ok(true);
                }
                Some(_) => self.bump(),
                None => return self.fail("an unterminated template literal"),
            }
        }
    }

    /// A regular expression literal, which no line terminator may end and
    /// whose `/` inside a character class does not close it. Its brackets
    /// and operators are counted as code's would be, escaped ones included,
    /// so a division this lexer mistook for a regular expression still has
    /// its nesting counted.
    fn script_regex(&mut self) -> Result<(), TooDeep> {
        self.index += 1;
        let mut class = false;
        loop {
            match self.peek(0) {
                Some('\\') => {
                    self.index += 1;
                    match self.peek(0) {
                        Some(c) if !is_line_terminator(c) => self.regex_char(c)?,
                        _ => return self.fail("an unterminated regular expression"),
                    }
                }
                Some('[') => {
                    class = true;
                    self.regex_char('[')?;
                }
                Some(']') => {
                    class = false;
                    self.regex_char(']')?;
                }
                Some('/') if !class => {
                    self.index += 1;
                    while self
                        .peek(0)
                        .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '$'))
                    {
                        self.index += 1;
                    }
                    return Ok(());
                }
                Some(c) if !is_line_terminator(c) => self.regex_char(c)?,
                _ => return self.fail("an unterminated regular expression"),
            }
        }
    }

    /// One character of a regular expression's body, counted as code.
    fn regex_char(&mut self, c: char) -> Result<(), TooDeep> {
        self.index += 1;
        match c {
            '(' | '[' | '{' => self.open(),
            ')' | ']' | '}' => {
                self.close();
                Ok(())
            }
            '!' | '-' | '+' | '~' | '*' | '&' | '|' | '=' | '<' | '?' | ':' | '>' | '.' | '^'
            | '%' => self.operator(),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Language, MAX_BRACKETS, check};

    fn deep(depth: usize) -> String {
        format!("{}1{}", "(".repeat(depth), ")".repeat(depth))
    }

    #[test]
    fn ordinary_code_passes() {
        let rust = "fn main<'a>(x: &'a str) -> &'a str { let c = '\\''; let s = r#\"a\"b\"#; let t = b'x'; if x.len() > 2 { println!(\"{x}\"); } x }";
        assert_eq!(check(rust, Language::Rust), Ok(()));
        let script = "const a = (b) => b ? 1 : 2; for (const c of [1, 2]) { a(c); }\nconst r = /[/\"]+/g; const t = `x${a(1)}y${`z${2}`}`; if (t) /re/.test(t); const d = (1 + 2) / 3;";
        assert_eq!(check(script, Language::Script), Ok(()));
    }

    #[test]
    fn deep_brackets_and_long_operator_runs_are_refused() {
        assert!(check(&"(".repeat(200), Language::Script).is_err());
        assert!(check(&format!("x = {}1;", "!".repeat(5000)), Language::Rust).is_err());
        assert!(check(&format!("x = {}1;", "a => ".repeat(5000)), Language::Script).is_err());
        assert!(check(&"if x {} else ".repeat(5000), Language::Rust).is_err());
    }

    #[test]
    fn brackets_after_a_quote_in_a_char_literal_are_counted() {
        let depth = MAX_BRACKETS + 1;
        for literal in [
            "'\"'",
            "'\\\"'",
            "'\\''",
            "b'\"'",
            "r#\"a\"b\"#",
            "br##\"\"#\"##",
            "/* /* */ \" */",
        ] {
            let text = format!("fn f() {{ let _c = {literal}; let _x = {}; }}", deep(depth));
            assert!(check(&text, Language::Rust).is_err(), "{literal}");
        }
    }

    #[test]
    fn brackets_after_script_literals_that_hold_quotes_are_counted() {
        let depth = MAX_BRACKETS + 1;
        for prefix in [
            "const t = `${1}\"`;",
            "const r = /\"/;",
            "const r = /[/\"]/;",
            "// a comment\u{2028}",
            "const s = '\\'\"';",
        ] {
            let text = format!("{prefix} const x = {};", deep(depth));
            assert!(check(&text, Language::Script).is_err(), "{prefix}");
        }
    }

    #[test]
    fn a_slash_the_grammar_cannot_place_is_refused() {
        for text in [
            "if (r) {} /\"/.test(r);",
            "a++ /\"/;",
            "x = of /\"/;",
            "x = a. /b/;",
            "x = \\u0072eturn /\"/;",
        ] {
            assert!(check(text, Language::Script).is_err(), "{text}");
        }
        for text in [
            "x = a / b / c;",
            "x = (a) / 2;",
            "return /re/;",
            "x = [1] / 2;",
            "x = y.if(a) / 2;",
        ] {
            assert_eq!(check(text, Language::Script), Ok(()), "{text}");
        }
    }

    #[test]
    fn nesting_inside_a_regular_expression_is_counted() {
        let text = format!("const r = /{}/;", "(".repeat(MAX_BRACKETS + 1));
        assert!(check(&text, Language::Script).is_err());
    }

    #[test]
    fn malformed_literals_are_refused() {
        assert!(check("let s = \"unterminated", Language::Rust).is_err());
        assert!(check("let s = r#\"unterminated\"", Language::Rust).is_err());
        assert!(check("/* /* */", Language::Rust).is_err());
        assert!(check("const s = 'a\nb';", Language::Script).is_err());
        assert!(check("const r = /a\n/;", Language::Script).is_err());
        assert!(check("const t = `a", Language::Script).is_err());
    }
}
