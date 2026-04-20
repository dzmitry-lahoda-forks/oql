//! Proc-macro implementation for `oql!`.
//!
//! Module layout:
//! - `ast`   ; data structures for a parsed query.
//! - `parse` ; `syn::Parse` impls. TokenStream → AST.
//! - `expand`; AST → Rust code (TokenStream).
//!
//! The only public item is the `oql!` macro.

#![warn(missing_docs)]

use proc_macro::TokenStream;
use syn::parse_macro_input;

mod ast;
mod expand;
mod liveness;
mod parse;

/// See the crate-level documentation of `oql`.
#[proc_macro]
pub fn oql(input: TokenStream) -> TokenStream {
    let query = parse_macro_input!(input as ast::Query);
    expand::expand(&query).into()
}
