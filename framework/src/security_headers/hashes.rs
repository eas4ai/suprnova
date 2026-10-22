//! Hash sources come from executable inline scripts in the trusted document.

use std::cell::RefCell;
use std::collections::BTreeSet;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer};
use sha2::{Digest, Sha256};

use crate::FrameworkError;

#[derive(Default)]
struct Scripts {
    script: Option<(bool, String)>,
    hashes: BTreeSet<String>,
    template_depth: usize,
    nonce: bool,
    foreign: Vec<String>,
    foreign_script: bool,
}

#[derive(Default)]
struct ScriptSink(RefCell<Scripts>);

impl TokenSink for ScriptSink {
    type Handle = ();

    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        let mut state = self.0.borrow_mut();
        match token {
            Token::TagToken(tag) => {
                let name: &str = tag.name.as_ref();
                if tag.kind == TagKind::StartTag {
                    state.nonce |= tag.attrs.iter().any(|attr| &*attr.name.local == "nonce");
                    if matches!(name, "svg" | "math") && !tag.self_closing {
                        state.foreign.push(name.to_string());
                    }
                    if name == "template" {
                        state.template_depth += 1;
                    }
                    if name == "script" {
                        state.foreign_script |= !state.foreign.is_empty();
                        let attr = |name: &str| {
                            tag.attrs
                                .iter()
                                .find(|attr| &*attr.name.local == name)
                                .map(|attr| attr.value.as_ref())
                        };
                        let kind = attr("type").unwrap_or("").trim().to_ascii_lowercase();
                        let executable = matches!(
                            kind.as_str(),
                            "" | "module"
                                | "text/javascript"
                                | "application/javascript"
                                | "text/ecmascript"
                                | "application/ecmascript"
                                | "application/x-javascript"
                                | "text/jscript"
                                | "text/livescript"
                                | "text/x-javascript"
                                | "text/x-ecmascript"
                                | "application/x-ecmascript"
                        ) || matches!(
                            kind.as_str(),
                            "text/javascript1.0"
                                | "text/javascript1.1"
                                | "text/javascript1.2"
                                | "text/javascript1.3"
                                | "text/javascript1.4"
                                | "text/javascript1.5"
                        );
                        let enabled =
                            executable && attr("src").is_none() && state.template_depth == 0;
                        state.script = Some((enabled, String::new()));
                        return TokenSinkResult::RawData(RawKind::ScriptData);
                    }
                    if matches!(
                        name,
                        "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript"
                    ) {
                        return TokenSinkResult::RawData(RawKind::Rawtext);
                    }
                    if matches!(name, "textarea" | "title") {
                        return TokenSinkResult::RawData(RawKind::Rcdata);
                    }
                    if name == "plaintext" {
                        return TokenSinkResult::Plaintext;
                    }
                } else if matches!(name, "svg" | "math") {
                    if state.foreign.last().is_some_and(|open| open == name) {
                        state.foreign.pop();
                    }
                } else if name == "script" {
                    if let Some((true, script)) = state.script.take() {
                        state.hashes.insert(format!(
                            "'sha256-{}'",
                            STANDARD.encode(Sha256::digest(script.as_bytes()))
                        ));
                    }
                } else if name == "template" {
                    state.template_depth = state.template_depth.saturating_sub(1);
                }
            }
            Token::CharacterTokens(text) => {
                if let Some((true, script)) = state.script.as_mut() {
                    script.push_str(&text);
                }
            }
            Token::NullCharacterToken => {
                if let Some((true, script)) = state.script.as_mut() {
                    script.push('\u{fffd}');
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

/// Tokenization observes HTML raw-text rules and ignores comments and attributes.
pub(crate) fn trusted_script_hashes(document: &[u8]) -> Result<Vec<String>, FrameworkError> {
    let html = std::str::from_utf8(document)
        .map_err(|_| FrameworkError::internal("Public Inertia document is not UTF-8"))?;
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(html));
    let tokenizer = Tokenizer::new(ScriptSink::default(), Default::default());
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    let state = tokenizer.sink.0.into_inner();
    if state.foreign_script {
        return Err(FrameworkError::internal(
            "Public Inertia document contains a script in foreign content",
        ));
    }
    if state.nonce {
        return Err(FrameworkError::internal(
            "Public Inertia document contains a nonce attribute",
        ));
    }
    Ok(state.hashes.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pub_001_hashes_only_inline_executable_scripts() -> Result<(), FrameworkError> {
        let html = br#"<!-- <script>comment()</script> --><textarea><script>text()</script></textarea><template><script>template()</script></template><script type=application/json>{"secret":"<script>fake()</script>"}</script><script src=/app.js>ignored()</script><script>allowed()</script><script type=module>module()</script>"#;
        let mut expected = ["allowed()", "module()"]
            .map(|script| {
                format!(
                    "'sha256-{}'",
                    STANDARD.encode(Sha256::digest(script.as_bytes()))
                )
            })
            .to_vec();
        expected.sort();
        assert_eq!(trusted_script_hashes(html)?, expected);
        Ok(())
    }

    #[test]
    fn pub_001_nonce_attributes_refuse_public_document() {
        assert!(trusted_script_hashes(br#"<script nonce=private>run()</script>"#).is_err());
    }
}
