//! AST data structures for an OQL query.
//!
//! A query is linear: exactly one `from` at the start, exactly one `select`
//! at the end, and zero or more `let`, `where`, `orderby`, `join` clauses in
//! any order in between. The order is semantically meaningful; it determines
//! the execution order of the generated pipeline.

use syn::{Expr, Ident, Pat};

/// A complete query: `from ... select ...`.
pub struct Query {
    /// The mandatory leading `from` clause.
    pub from: FromClause,
    /// Zero or more middle clauses, applied in source order.
    pub middle: Vec<MiddleClause>,
    /// The mandatory trailing `select` clause.
    pub select: SelectClause,
}

/// `from <pat> in <expr>`.
///
/// `pat` is a full Rust pattern, so destructuring forms like
/// `from (a, b) in pairs` or `from Point { x, y } in points` work for free
/// via reuse of `syn`'s pattern parser.
pub struct FromClause {
    /// The pattern binding the range variable.
    pub pat: Pat,
    /// The expression producing the source iterable.
    pub source: Expr,
}

/// Every clause permitted between `from` and `select`.
///
/// Clauses are applied *in the order they appear*. Writing `where` before
/// `let` reads differently from `let` before `where`, and that is by design:
/// the user controls execution order, just as with a hand-written iterator
/// chain.
///
/// The `Join` variant is boxed because its payload is significantly larger
/// than the others; boxing keeps every element of `Vec<MiddleClause>` small
/// regardless of which variant it holds.
pub enum MiddleClause {
    /// `let <n> = <expr>`; intermediate binding, visible to all later clauses.
    Let {
        /// The name of the new binding.
        name: Ident,
        /// The expression producing the bound value.
        value: Expr,
    },
    /// `where <cond>`.
    Where(Expr),
    /// `orderby <key>` or `orderby <key> desc`.
    OrderBy {
        /// The expression producing the sort key.
        key: Expr,
        /// Whether the key should be sorted in descending order.
        descending: bool,
    },
    /// `join <n> in <source> on <outer_key> == <inner_key>`, optionally
    /// `into <g>` for a group-join.
    Join(Box<JoinClause>),
    /// `group <element> by <key> into <name>`.
    ///
    /// Collects every element seen so far into groups keyed by `key`, then
    /// yields one environment per group where `name` binds a `Group<K, T>`
    /// with `.key` and `.items: Vec<T>`. Bindings from before this clause
    /// are no longer available afterwards; only `name` is.
    GroupBy {
        /// The per-element binding expression; what we group. Usually the
        /// range variable, but can be a projection like `o.amount`.
        element: Expr,
        /// The key expression; what the grouping key is.
        key: Expr,
        /// The name of the outgoing group binding.
        name: Ident,
    },
}

/// Payload for a `join` clause. Kept as a separate type to avoid inflating
/// the size of every `MiddleClause` variant.
///
/// Inner equi-join. The macro expands this into a hash-join: a `HashMap`
/// from the inner key to matching inner elements is built once, then each
/// outer element probes the map.
///
/// After the clause, `name` is a new binding in the environment alongside
/// the existing ones.
pub struct JoinClause {
    /// The name of the new binding produced by the join.
    pub name: Ident,
    /// The inner source expression.
    pub source: Expr,
    /// The outer-side key expression (evaluated against the outer env).
    pub outer_key: Expr,
    /// The inner-side key expression (evaluated against each inner item).
    pub inner_key: Expr,
    /// If `Some(g)`, this is a group-join: instead of producing one
    /// environment per `(outer, inner)` match, produce exactly one
    /// environment per outer element, with `g` bound to a `Vec<Inner>` of
    /// all matches (empty if none).
    pub into_group: Option<Ident>,
}

/// `select <expr>`.
pub struct SelectClause {
    /// The projection expression.
    pub expr: Expr,
}
