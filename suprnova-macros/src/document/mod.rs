//! `#[suprnova::document(collection = "...")]`: a document model stored in
//! a MongoDB collection (PAR-183). The framework side, which the emitted
//! code calls, is `suprnova::mongodb`.

mod emit;
mod parse;

use proc_macro2::TokenStream;
use syn::Result;

/// Expand the attribute: parse, validate, emit.
pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let input = parse::DocumentInput::parse(attr, item)?;
    Ok(emit::emit(&input))
}
