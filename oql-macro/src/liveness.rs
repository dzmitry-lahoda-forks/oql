//! Live-variable analysis for OQL queries.
//!
//! The expander propagates an "environment tuple" of bindings through each
//! pipeline step. Without this analysis the tuple would keep every binding
//! introduced so far; even ones that are never referenced again. That
//! makes the sort payload larger than it needs to be and forces the
//! downstream pipeline to carry dead weight.
//!
//! This module computes, for each observable point in a query, which
//! bindings are *live* at that point: referenced in this clause or any
//! later clause. The expander uses that set to build the smallest possible
//! outgoing environment tuple at every step.
//!
//! # What counts as a read
//!
//! We inspect an expression's token stream and treat every identifier we
//! see as a potential read. That is a conservative over-approximation:
//! field names in `o.price`, method names in `v.clone()`, and macro paths
//! all count too. Over-approximation is always safe here; the worst case
//! is that we keep a binding alive that the generated code could have
//! dropped, which gives up a potential optimisation but never produces
//! incorrect code.
//!
//! # Observable points
//!
//! A query has these clause boundaries:
//!
//! ```text
//! from    (entry; initial env = from bindings)
//! c_0     let | where | orderby | join
//! c_1     ...
//! ...
//! c_{n-1}
//! select  (exit; reads go to projection)
//! ```
//!
//! For each clause boundary we return the set of bindings that are live
//! *just after* that clause completes. That is precisely the set the
//! expander should keep in the outgoing environment tuple emitted by that
//! clause's pipeline step.

use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::ToTokens;

use crate::ast::{MiddleClause, Query, SelectClause};

/// For each index `i` in `query.middle` (plus one extra entry for after
/// the final `select`), returns the set of binding names that are live
/// *just after* that clause finishes.
///
/// Slot layout of the returned vector:
///
/// ```text
/// index 0               ; live after middle[0]
/// index 1               ; live after middle[1]
/// …
/// index middle.len() - 1; live after the last middle clause
/// index middle.len()    ; live after select (always empty: select is the exit)
/// ```
///
/// For `query.middle.len() == 0` the vector still has one slot at index 0
/// for the post-select live set (empty).
///
/// # Why post-clause?
///
/// A clause's emitter produces the outgoing environment tuple. The tuple
/// flows into the *next* clause. So the set we want at emit time is "what
/// does anyone from here on still read?"; that is exactly the live set
/// *after* this clause.
pub fn compute_live_after(query: &Query) -> Vec<HashSet<String>> {
    let n = query.middle.len();
    let mut live_after: Vec<HashSet<String>> = vec![HashSet::new(); n + 1];

    // Start from the back. The final slot (after select) stays empty;
    // select produces a fresh value, nothing in the environment survives.
    //
    // Walk backwards through middle clauses. For each clause, the live set
    // *before* it equals `(live_after \ defined_here) ∪ reads_here`.
    // But what we store is the live set *after* the previous clause, which
    // is the same as "live before this clause".
    //
    // So: live_after[i]; for i in 0..n-1; is computed from live_after[i+1]
    // plus the reads/defs of middle[i+1]. The terminal slot live_after[n]
    // is derived from the select clause.
    //
    // Concretely, walking i from n-1 down to 0:
    //   live_after[i] = live_before(middle[i+1])
    //                 = (live_after[i+1] \ defs(middle[i+1])) ∪ reads(middle[i+1])
    //
    // And live_after[n-1] is the "live before select" set, which is just
    // reads(select).

    // Seed: reads of the select expression.
    let mut current: HashSet<String> = idents_in_tokens(&query.select.expr.to_token_stream());

    // The last middle clause's outgoing live set is exactly what select reads.
    if n > 0 {
        live_after[n - 1] = current.clone();
    }

    // Walk backwards through middle clauses. At iteration `i`, `current`
    // holds the live set just before middle[i] fires, i.e. the live set
    // just after middle[i-1] finishes. We store it in live_after[i-1].
    for i in (0..n).rev() {
        let clause = &query.middle[i];
        // Compute the live set *before* this clause: remove defs, add reads.
        current = step_backwards(&current, clause);
        if i > 0 {
            live_after[i - 1] = current.clone();
        }
        // When i == 0, `current` is the live set before middle[0], i.e.
        // just after the `from` clause. The expander doesn't need that
        // slot (the from bindings always enter the pipeline), so we
        // discard it.
    }

    live_after
}

/// Given the live set *after* a clause, returns the live set *before* it.
fn step_backwards(live_after: &HashSet<String>, clause: &MiddleClause) -> HashSet<String> {
    match clause {
        MiddleClause::Let { name, value } => {
            // `let name = value;`; defines `name`, reads idents in `value`.
            let mut out: HashSet<String> = live_after
                .iter()
                .filter(|s| s.as_str() != name.to_string().as_str())
                .cloned()
                .collect();
            out.extend(idents_in_tokens(&value.to_token_stream()));
            out
        }
        MiddleClause::Where(cond) => {
            // Defines nothing, reads cond.
            let mut out = live_after.clone();
            out.extend(idents_in_tokens(&cond.to_token_stream()));
            out
        }
        MiddleClause::OrderBy { key, .. } => {
            // Defines nothing, reads key.
            let mut out = live_after.clone();
            out.extend(idents_in_tokens(&key.to_token_stream()));
            out
        }
        MiddleClause::Join(j) => {
            // `join name in source on outer_key == inner_key [into g]`.
            // Defines `name` (and optionally `g` for group-join). Reads
            // `outer_key` on the outer env; source and inner_key are
            // evaluated *once* in the preamble against the inner source
            // only, so they do NOT affect liveness of the outer
            // environment. We therefore don't scan them here.
            let name_s = j.name.to_string();
            let group_s = j.into_group.as_ref().map(|g| g.to_string());
            let mut out: HashSet<String> = live_after
                .iter()
                .filter(|s| s.as_str() != name_s.as_str())
                .filter(|s| {
                    group_s
                        .as_ref()
                        .map(|g| g.as_str() != s.as_str())
                        .unwrap_or(true)
                })
                .cloned()
                .collect();
            out.extend(idents_in_tokens(&j.outer_key.to_token_stream()));
            out
        }
        MiddleClause::Zip { name, .. } => {
            // `zip name in source` / `zip_must name in source`; defines
            // `name`. The source expression is an independent iterator,
            // not evaluated against the current row environment.
            live_after
                .iter()
                .filter(|s| s.as_str() != name.to_string().as_str())
                .cloned()
                .collect()
        }
        MiddleClause::GroupBy { element, key, name } => {
            // `group <element> by <key> into <name>`.
            // Grouping is a full environment reset: only `name` survives
            // (bindings from before group-by are not meaningful per-group
            //; they would have been per-element).
            //
            // So the live set *before* this clause is exactly the reads
            // of `element` and `key`. Whatever was live after the clause
            // (minus `name` itself) is not carried backwards, because
            // those bindings don't survive the barrier.
            let _ = live_after; // intentionally dropped
            let _ = name; // name is a fresh binding, not a read
            let mut out = HashSet::new();
            out.extend(idents_in_tokens(&element.to_token_stream()));
            out.extend(idents_in_tokens(&key.to_token_stream()));
            out
        }
    }
}

/// Returns the set of identifier names read by the select projection.
/// Used by the expander to decide what the tail environment tuple must
/// contain just before `emit_select`.
pub fn reads_of_select(select: &SelectClause) -> HashSet<String> {
    idents_in_tokens(&select.expr.to_token_stream())
}

/// Returns the set of identifier names read by an expression. Used by the
/// expander to compute live-before sets at clause entry.
pub fn reads_of_expr(expr: &syn::Expr) -> HashSet<String> {
    idents_in_tokens(&expr.to_token_stream())
}

/// Extracts every identifier from a token stream. Over-approximates reads
///; field names, method names, etc. all count. That is deliberate: false
/// positives only mean a binding stays alive a bit longer than strictly
/// necessary, which is always a valid (if suboptimal) code shape.
fn idents_in_tokens(ts: &TokenStream) -> HashSet<String> {
    use proc_macro2::TokenTree;
    let mut out = HashSet::new();
    for tt in ts.clone() {
        match tt {
            TokenTree::Ident(i) => {
                out.insert(i.to_string());
            }
            TokenTree::Group(g) => {
                out.extend(idents_in_tokens(&g.stream()));
            }
            _ => {}
        }
    }
    out
}

/// Like `idents_in_tokens` but filters out identifiers that look like
/// field accesses (`.name`) or method calls (`.name(…)`). Still an
/// over-approximation; any bare identifier that a user writes counts.
///
/// Used by the expander when deciding whether a `where` clause is safe
/// to fuse into the preceding join's `move` closure: if the condition
/// references any identifier that isn't part of the environment tuple
/// (i.e. it must be captured from the enclosing scope), fusing would
/// move that value into the closure; which silently changes the
/// semantics for types like `Cell`, `RefCell`, or `&T`. So we only
/// fuse when every bare ident resolves to an env binding.
pub fn bare_idents(ts: &TokenStream) -> HashSet<String> {
    let mut out = HashSet::new();
    collect_bare(ts, &mut out, false);
    out
}

fn collect_bare(ts: &TokenStream, out: &mut HashSet<String>, mut after_dot: bool) {
    use proc_macro2::TokenTree;
    // Track whether the previous non-whitespace char was a `.` (field/method
    // access; skip this ident), a `:` (could be part of `::` path segment,
    // skip), or something else (ident is a real bare reference).
    let mut after_colon = false;
    for tt in ts.clone() {
        match tt {
            TokenTree::Punct(p) if p.as_char() == '.' => {
                after_dot = true;
                after_colon = false;
            }
            TokenTree::Punct(p) if p.as_char() == ':' => {
                after_colon = true;
                after_dot = false;
            }
            TokenTree::Ident(i) => {
                // Bare = not after `.` (field/method) and not after `:` or
                // `::` (path segment). Path segments may lead to a
                // module/type, not a closure capture, so skipping them is
                // safe; the caller treats "all idents bare" as "must be in
                // env", so skipping idents we know aren't captures only
                // enables more fusions, never fewer.
                if !after_dot && !after_colon {
                    out.insert(i.to_string());
                }
                after_dot = false;
                after_colon = false;
            }
            TokenTree::Group(g) => {
                collect_bare(&g.stream(), out, false);
                after_dot = false;
                after_colon = false;
            }
            _ => {
                after_dot = false;
                after_colon = false;
            }
        }
    }
}
