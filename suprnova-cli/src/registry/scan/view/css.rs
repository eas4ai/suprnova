//! The CSS scan (REG-031): stylesheets, `style` elements and `style`
//! attributes are tokenized with `cssparser`, which undoes CSS escapes, so
//! `@\69mport` and `\75rl(` are read as `@import` and `url(`. No
//! `@import` is admitted, and every resource the CSS names (`url()`,
//! `src()`, the strings inside `image-set()` and its relatives, a custom
//! property's value included) must stay on the application's origin.

use cssparser::{Parser, ParserInput, Token};

use super::super::url::check_constant;
use super::markup::Sink;

/// How deep blocks and functions may nest before the scan refuses the CSS.
const MAX_DEPTH: usize = 64;

/// What the function the walker is inside of makes of a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    /// Plain CSS.
    Plain,
    /// The argument of `url()` or `src()`: a string is a URL.
    Url,
    /// The arguments of `image-set()`, `image()` or `cross-fade()`: a
    /// string is a URL.
    Image,
}

/// Checks CSS text that starts on `first_line`: a stylesheet, a `style`
/// element's content or a `style` attribute's declarations, which tokenize
/// alike.
pub(crate) fn check(text: &str, first_line: u32, sink: &mut Sink) {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    walk(&mut parser, Context::Plain, 0, first_line, sink);
}

fn line(parser: &Parser<'_, '_>, first_line: u32) -> u32 {
    first_line.saturating_add(parser.current_source_location().line)
}

fn walk(
    parser: &mut Parser<'_, '_>,
    context: Context,
    depth: usize,
    first_line: u32,
    sink: &mut Sink,
) {
    if depth > MAX_DEPTH {
        sink.refuse(
            "view-css",
            line(parser, first_line),
            format!("CSS nests deeper than {MAX_DEPTH} levels"),
        );
        return;
    }
    loop {
        let at = line(parser, first_line);
        let token = match parser.next_including_whitespace_and_comments() {
            Ok(token) => token.clone(),
            Err(_) => return,
        };
        match token {
            Token::AtKeyword(name) if name.eq_ignore_ascii_case("import") => {
                sink.refuse(
                    "css-import",
                    at,
                    "`@import` loads a stylesheet the scan cannot read".to_string(),
                );
            }
            Token::UnquotedUrl(url) => check_url(&url, at, sink),
            Token::BadUrl(_) => {
                sink.refuse(
                    "css-url",
                    at,
                    "a malformed `url()` the scan cannot read".to_string(),
                );
            }
            Token::QuotedString(value) if context != Context::Plain => check_url(&value, at, sink),
            Token::Function(name) => {
                let lower = name.to_ascii_lowercase();
                let inner = match lower.as_str() {
                    "url" | "src" => Context::Url,
                    "image-set" | "-webkit-image-set" | "image" | "cross-fade"
                    | "-webkit-cross-fade" => Context::Image,
                    "var" | "env" | "attr" if context != Context::Plain => {
                        sink.refuse(
                            "css-url",
                            at,
                            format!("`{lower}()` inside a resource function names a URL the scan cannot read"),
                        );
                        Context::Plain
                    }
                    "attr" => {
                        let mut names_url = false;
                        let _ = parser.parse_nested_block(|nested| {
                            while let Ok(token) = nested.next() {
                                if let Token::Ident(ident) | Token::Function(ident) = token
                                    && (ident.eq_ignore_ascii_case("url")
                                        || ident.eq_ignore_ascii_case("type"))
                                {
                                    names_url = true;
                                }
                            }
                            Ok::<(), cssparser::ParseError<'_, ()>>(())
                        });
                        if names_url {
                            sink.refuse(
                                "css-url",
                                at,
                                "`attr()` read as a URL names a resource the scan cannot read"
                                    .to_string(),
                            );
                        }
                        continue;
                    }
                    _ => context,
                };
                let _ = parser.parse_nested_block(|nested| {
                    walk(nested, inner, depth + 1, first_line, sink);
                    Ok::<(), cssparser::ParseError<'_, ()>>(())
                });
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                let inner = if matches!(token, Token::CurlyBracketBlock) {
                    Context::Plain
                } else {
                    context
                };
                let _ = parser.parse_nested_block(|nested| {
                    walk(nested, inner, depth + 1, first_line, sink);
                    Ok::<(), cssparser::ParseError<'_, ()>>(())
                });
            }
            _ => {}
        }
    }
}

fn check_url(url: &str, line: u32, sink: &mut Sink) {
    if let Err(refusal) = check_constant(url) {
        sink.refuse(
            "css-url",
            line,
            format!("`url({url})`: {}", refusal.describe()),
        );
    }
}
