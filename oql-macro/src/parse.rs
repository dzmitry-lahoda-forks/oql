//! Parser for OQL queries.
//!
//! Keywords are contextual (not reserved). We recognise them as identifiers
//! and compare the name. `where` and `let` are Rust keywords and are matched
//! with `Token![where]` / `Token![let]`.
//!
//! Clauses use no separators (no commas, no semicolons). Whitespace between
//! them is enough; we look at the next token and dispatch to the
//! appropriate sub-parser. Reads like a paragraph.

use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, Pat, Token};

use crate::ast::{FromClause, JoinClause, MiddleClause, Query, SelectClause};

impl Parse for Query {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let from = input.parse::<FromClause>()?;

        let mut middle = Vec::new();
        loop {
            // Decide based on the next token which clause comes next.
            // `select` terminates the middle section.
            if peek_keyword(input, "select") {
                break;
            }
            middle.push(input.parse::<MiddleClause>()?);
        }

        let select = input.parse::<SelectClause>()?;

        if !input.is_empty() {
            return Err(input.error(
                "unexpected tokens after `select`; `select` must be the last clause",
            ));
        }

        Ok(Query { from, middle, select })
    }
}

impl Parse for FromClause {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        expect_keyword(input, "from")?;
        let pat = Pat::parse_single(input)?;
        input.parse::<Token![in]>().map_err(|e| {
            syn::Error::new(e.span(), "expected `in` after pattern in `from` clause")
        })?;
        // Expression up to the next clause. `Expr::parse` is greedy but stops
        // at unknown tokens; and our contextual keywords look like unknown
        // identifiers to the expression parser.
        let source = input.parse::<Expr>()?;
        Ok(FromClause { pat, source })
    }
}

impl Parse for MiddleClause {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(Token![let]) {
            input.parse::<Token![let]>()?;
            let name: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let value: Expr = input.parse()?;
            Ok(MiddleClause::Let { name, value })
        } else if input.peek(Token![where]) {
            input.parse::<Token![where]>()?;
            let cond: Expr = input.parse()?;
            Ok(MiddleClause::Where(cond))
        } else if peek_keyword(input, "orderby") {
            eat_keyword(input, "orderby")?;
            let key: Expr = input.parse()?;
            let descending = if peek_keyword(input, "desc") {
                eat_keyword(input, "desc")?;
                true
            } else if peek_keyword(input, "asc") {
                eat_keyword(input, "asc")?;
                false
            } else {
                false
            };
            Ok(MiddleClause::OrderBy { key, descending })
        } else if peek_keyword(input, "join") {
            parse_join(input)
        } else if peek_keyword(input, "group") {
            parse_group_by(input)
        } else {
            Err(input.error(
                "expected `let`, `where`, `orderby`, `join`, `group`, or `select`",
            ))
        }
    }
}

/// Parses `join <name> in <source> on <outer_key> == <inner_key>`.
///
/// The `on` clause is parsed as a single `Expr`, then split on a top-level
/// `==`. This avoids the need to guess where the outer-key expression ends;
/// the expression parser handles nesting and operator precedence for us.
fn parse_join(input: ParseStream) -> syn::Result<MiddleClause> {
    eat_keyword(input, "join")?;
    let name: Ident = input.parse()?;
    input.parse::<Token![in]>().map_err(|e| {
        syn::Error::new(e.span(), "expected `in` after name in `join` clause")
    })?;
    let source: Expr = input.parse()?;
    expect_keyword(input, "on")?;
    let condition: Expr = input.parse()?;

    let (outer_key, inner_key) = split_on_eq(condition)?;

    // Optional group-join: `... on a == b into g`. The `into` keyword must
    // come after the equality condition, so we peek after parsing the
    // condition expression. `into` is a Rust keyword but is accepted as an
    // identifier in this position; we match via peek_keyword below, which
    // parses it as an `Ident`.
    let into_group = if peek_keyword(input, "into") {
        eat_keyword(input, "into")?;
        let g: Ident = input.parse()?;
        Some(g)
    } else {
        None
    };

    Ok(MiddleClause::Join(Box::new(JoinClause {
        name,
        source,
        outer_key,
        inner_key,
        into_group,
    })))
}

/// Parses `group <element> by <key> into <name>`.
///
/// The `element` is the expression that will be collected into each group's
/// `items` vec. Typically the range variable itself (e.g. `group o by ...`)
/// but a projection is also allowed (`group o.amount by ...`).
///
/// The `key` is what elements are grouped by. Must be `Eq + Hash`.
///
/// The `name` is the outgoing binding; a `Group<K, T>` struct where
/// `K` is the key type and `T` is the element type. It exposes `.key` and
/// `.items`. After this clause, *only* `name` is in scope; no bindings
/// from before `group by` survive, because they'd be ambiguous (which
/// element's value would they hold?).
fn parse_group_by(input: ParseStream) -> syn::Result<MiddleClause> {
    eat_keyword(input, "group")?;
    let element: Expr = input.parse()?;
    expect_keyword(input, "by")?;
    let key: Expr = input.parse()?;
    expect_keyword(input, "into")?;
    let name: Ident = input.parse()?;
    Ok(MiddleClause::GroupBy { element, key, name })
}

/// Splits an equality expression into its left and right sides.
///
/// Only accepts `a == b` at the top level. Anything else is an error,
/// because the `on` clause must be an equi-join; no inequalities, no
/// complex conditionals. Composite keys can be written as tuples:
/// `on (a.x, a.y) == (b.x, b.y)`.
fn split_on_eq(expr: Expr) -> syn::Result<(Expr, Expr)> {
    use syn::{BinOp, ExprBinary};
    match expr {
        Expr::Binary(ExprBinary {
            left,
            op: BinOp::Eq(_),
            right,
            ..
        }) => Ok((*left, *right)),
        other => Err(syn::Error::new_spanned(
            other,
            "`join` requires an equality condition of the form `on a == b`",
        )),
    }
}

impl Parse for SelectClause {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        expect_keyword(input, "select")?;
        let expr: Expr = input.parse()?;
        Ok(SelectClause { expr })
    }
}

// --- Contextual keyword helpers -----------------------------------------------

/// Checks (without consuming) whether the next token is an identifier with
/// the given name.
fn peek_keyword(input: ParseStream, kw: &str) -> bool {
    input
        .fork()
        .parse::<Ident>()
        .map(|ident| ident == kw)
        .unwrap_or(false)
}

/// Consumes the keyword or returns an error with an appropriate span.
fn expect_keyword(input: ParseStream, kw: &str) -> syn::Result<()> {
    let ident: Ident = input
        .parse()
        .map_err(|_| syn::Error::new(input.span(), format!("expected `{kw}`")))?;
    if ident == kw {
        Ok(())
    } else {
        Err(syn::Error::new(
            ident.span(),
            format!("expected `{kw}`, found `{ident}`"),
        ))
    }
}

/// Like `expect_keyword`, but assumes the caller already peeked.
fn eat_keyword(input: ParseStream, kw: &str) -> syn::Result<()> {
    expect_keyword(input, kw)
}
