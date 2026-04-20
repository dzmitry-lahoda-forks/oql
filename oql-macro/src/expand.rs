//! Expander: AST → Rust code.
//!
//! ## Strategy
//!
//! After `from`, the range variable is visible. Each `let` adds another
//! binding. To give every downstream clause access to all currently visible
//! bindings, we propagate them as an *environment tuple* through the
//! iterator chain.
//!
//! ```text
//! oql! {
//!     from x in xs
//!     let y = x * 2
//!     where y > 5
//!     select (x, y)
//! }
//! ```
//!
//! expands (roughly) to:
//!
//! ```text
//! ::core::iter::IntoIterator::into_iter(xs)
//!     .map(|x| (x,))
//!     .map(|(x,)| { let y = x * 2; (x, y) })
//!     .filter_map(|(x, y)| if y > 5 { Some((x, y)) } else { None })
//!     .map(|(x, y)| (x, y))
//! ```
//!
//! The compiler inlines tuple construction and destructuring completely.
//! In a release build, none of it shows up in the emitted machine code.
//!
//! ## `where`
//!
//! We use `filter_map` instead of `filter` so the user's condition operates
//! on owned bindings. `Iterator::filter` would pass `&T` and force the user
//! to write `*x > 5` instead of `x > 5`, which is inconsistent with `let`
//! and `select`. In release, LLVM inlines the `Option` construction away.
//!
//! ## `orderby`
//!
//! Sorting forces a `collect → sort → into_iter` round-trip. Multiple
//! consecutive `orderby` clauses fold into a single sort with a composite
//! key tuple (Rust sorts tuples lexicographically, so first clause has
//! highest priority). `desc` wraps the key part in `core::cmp::Reverse`.
//!
//! Key extraction uses a reference-shadowed block so that non-`Copy` keys
//! (e.g. `String`) don't require the user to write `.clone()` explicitly;
//! we clone internally. For `Copy` keys, the clone is a no-op.

use proc_macro2::TokenStream;
use quote::quote;
use std::collections::HashSet;
use syn::{Expr, Ident, Pat};

use crate::ast::{FromClause, MiddleClause, Query, SelectClause};
use crate::liveness::compute_live_after;

pub fn expand(query: &Query) -> TokenStream {
    let (initial_pat_tokens, initial_env) = analyze_from(&query.from);
    let source = &query.from.source;

    // Live-variable analysis: for each middle clause index, the set of
    // binding names that are still read by this or any later clause
    // (including select). We use this to drop dead bindings from the
    // environment tuple flowing between pipeline steps; the same
    // optimisation a hand-written pipeline would naturally do by not
    // carrying intermediate values past the point where they're consumed.
    let live_after: Vec<HashSet<String>> = compute_live_after(query);
    // For the "live before select" set (which is "live after the last
    // middle clause"), see liveness::compute_live_after doc.

    let mut pipeline = TokenStream::new();
    let mut preamble = TokenStream::new();
    let mut env = initial_env.clone();
    let mut pending_sorts: Vec<(Expr, bool)> = Vec::new();
    // Sequence of `let name = value` that have been queued up since the
    // last real iterator operation. They'll be fused into the next
    // `where`/`join`/`orderby`/`select` closure so we emit one `.map()`
    // per group of consecutive `let`s instead of one per `let`.
    //
    // Each let carries a `live` flag: when true, the let's name must make
    // it into the outgoing environment tuple. When false, the let is only
    // emitted as a statement (for side-effect preservation and so later
    // lets that read it still see it), but dropped from the outgoing
    // tuple. This is the dead-binding optimisation in action.
    let mut pending_lets: Vec<(Ident, Expr, bool, bool)> = Vec::new();
    let mut join_counter: usize = 0;

    let mut i = 0;
    while i < query.middle.len() {
        let clause = &query.middle[i];
        // live_after[i] = bindings still needed after middle[i] finishes.
        // That is exactly what middle[i]'s emitter should put into the
        // outgoing environment tuple.
        let live = &live_after[i];
        // live_before[i] = bindings needed entering middle[i]. For flushes
        // that happen BEFORE middle[i] fires, the sort output must still
        // contain every binding this clause is about to read, so we use
        // live_before, not live_after. Computed as live_after of the
        // previous clause, or the reads of middle[0] when i==0.
        let live_before: HashSet<String> = if i == 0 {
            // For i==0 there's no previous clause; live_before is simply
            // this clause's reads plus whatever lives after it.
            let mut s = live_after[0].clone();
            match clause {
                MiddleClause::Let { value, .. } => {
                    s.extend(crate::liveness::reads_of_expr(value));
                }
                MiddleClause::Where(cond) => {
                    s.extend(crate::liveness::reads_of_expr(cond));
                }
                MiddleClause::OrderBy { key, .. } => {
                    s.extend(crate::liveness::reads_of_expr(key));
                }
                MiddleClause::Join(j) => {
                    s.extend(crate::liveness::reads_of_expr(&j.outer_key));
                }
                MiddleClause::GroupBy { element, key, .. } => {
                    s.extend(crate::liveness::reads_of_expr(element));
                    s.extend(crate::liveness::reads_of_expr(key));
                }
            }
            s
        } else {
            live_after[i - 1].clone()
        };
        match clause {
            MiddleClause::Let { name, value } => {
                // If there are pending sorts, they must be committed
                // before introducing a new binding. The new binding is not
                // part of the pipeline state during the sort, so folding
                // the sort into a later stage would give the wrong
                // semantics (the sort would see the new name's value
                // instead of the original element ordering).
                //
                // Use live_BEFORE for the sort flush: the sort's output
                // must still contain every binding this let is about to
                // read, so we can't drop them yet. live_after would be
                // too aggressive and would kick out bindings that the
                // let's value expression is about to consume.
                env = flush_sorts(&mut pipeline, &env, &mut pending_sorts, &mut pending_lets, &live_before);
                // Defer: we'll bake this into the next real step's closure.
                // If no real step follows, the tail flush emits a single
                // combined `.map()`.
                //
                // Liveness of the let name itself: only carry it forward
                // in the outgoing env if some later clause reads it.
                // Dead lets are still emitted as statements (side-effect
                // preservation) but don't enter the outgoing tuple.
                let is_live = live.contains(&name.to_string());
                // When the let name shadows an existing env binding
                // (e.g. `from x in xs` then `let x = x * 10`), the
                // outgoing tuple already contains a slot for this name.
                // We record the shadow flag so that the pending-name
                // filter used by emitters knows not to treat this as a
                // "new pending binding" to remove from incoming_env.
                let name_s = name.to_string();
                let is_shadow = env.iter().any(|e| e.to_string() == name_s);
                pending_lets.push((name.clone(), value.clone(), is_live, is_shadow));
                if is_live && !is_shadow {
                    env.push(name.clone());
                }
                // Do NOT filter `env` here; the let's value expression
                // still needs to read its input bindings, which happens
                // inside the next real step's closure. Filtering will
                // happen there, informed by that step's live_after.
            }
            MiddleClause::Where(cond) => {
                // Same live_before logic as in Let: the sort must keep
                // everything the `where` condition reads.
                env = flush_sorts(&mut pipeline, &env, &mut pending_sorts, &mut pending_lets, &live_before);
                // After a filter, the outgoing env is `env ∩ live`.
                let outgoing_env: Vec<Ident> =
                    env.iter().filter(|id| live.contains(&id.to_string())).cloned().collect();
                emit_where(&mut pipeline, &env, &outgoing_env, cond, &mut pending_lets);
                env = outgoing_env;
            }
            MiddleClause::OrderBy { key, descending } => {
                // orderby doesn't change the env shape; it's queued for the
                // next real step. Liveness is applied at flush time.
                pending_sorts.push((key.clone(), *descending));
            }
            MiddleClause::Join(join) => {
                // Same live_before logic: the sort must still keep the
                // bindings this join's outer_key reads.
                env = flush_sorts(&mut pipeline, &env, &mut pending_sorts, &mut pending_lets, &live_before);

                if let Some(group_name) = &join.into_group {
                    // Group-join: yield exactly one environment per outer
                    // element, with `group_name` bound to a Vec of all
                    // inner matches (empty if none). No fusion here;
                    // where/orderby after a group-join filter/sort whole
                    // outer rows, and those rows carry the full group.
                    //
                    // `join.name` (the per-match binding) does NOT survive
                    // a group-join; there isn't a single inner row per
                    // outer, the whole point is the group. Any downstream
                    // reference to `name` is almost certainly a closure
                    // parameter shadow (`o_group.iter().map(|o| ...)`) that
                    // the over-approximate liveness analysis mistook for
                    // a real read. We drop it from the outgoing env
                    // unconditionally.
                    let is_group_live = live.contains(&group_name.to_string());
                    let mut outgoing_env: Vec<Ident> = env
                        .iter()
                        .filter(|id| live.contains(&id.to_string()))
                        .cloned()
                        .collect();
                    if is_group_live {
                        outgoing_env.push(group_name.clone());
                    }
                    emit_group_join(
                        &mut preamble,
                        &mut pipeline,
                        &env,
                        &outgoing_env,
                        &join.name,
                        group_name,
                        &join.source,
                        &join.outer_key,
                        &join.inner_key,
                        join_counter,
                        &mut pending_lets,
                    );
                    env = outgoing_env;
                    join_counter += 1;
                    i += 1;
                    continue;
                }

                // Peek ahead: if the very next clause is a `where` with no
                // intervening reorders (no pending_sorts here; we just
                // flushed), we fuse it into the join's flat_map body.
                // That drops one iterator stage for the common
                // join-then-filter pattern and matches what a handwriter
                // would naturally do.
                //
                // Fusion conditions:
                //   - Next clause exists and is `Where`.
                //   - The where's condition uses ONLY bindings from the
                //     join's environment (env + join.name). If the cond
                //     reads any other identifier, it must come from an
                //     outer-scope capture; fusing would then force those
                //     captures into the join's `move` closure, which
                //     silently breaks types like `Cell<T>` that the user
                //     wants to reference after the query runs. So we
                //     skip fusion in that case and keep the separate
                //     `.filter_map()` stage, which captures by shared
                //     reference.
                //   - No `let` between join and where.
                //   - No pending_sorts (already ensured by the flush above).
                //
                // When fused, the outgoing env is the one the `where`
                // would produce: (env+name) intersected with
                // `live_after[i+1]`, i.e. the live set after the where.
                let fused = if i + 1 < query.middle.len() {
                    match &query.middle[i + 1] {
                        MiddleClause::Where(cond) => {
                            // Safety check: every bare ident in `cond`
                            // must resolve to a binding we already have.
                            let allowed: HashSet<String> = env
                                .iter()
                                .map(|id| id.to_string())
                                .chain(::core::iter::once(join.name.to_string()))
                                .collect();
                            let bare = crate::liveness::bare_idents(
                                &quote::ToTokens::to_token_stream(cond),
                            );
                            if bare.iter().all(|name| allowed.contains(name)) {
                                ::core::option::Option::Some(cond)
                            } else {
                                ::core::option::Option::None
                            }
                        }
                        _ => ::core::option::Option::None,
                    }
                } else {
                    ::core::option::Option::None
                };

                let (active_live, consumed) = if fused.is_some() {
                    // Use the live set AFTER the where (live_after[i+1])
                    // as the join's outgoing-live set, since the where's
                    // filter is now part of the join step.
                    (&live_after[i + 1], true)
                } else {
                    (live, false)
                };

                // Outgoing env after join = `(env + join.name) ∩ active_live`.
                let is_name_live = active_live.contains(&join.name.to_string());
                let mut outgoing_env: Vec<Ident> = env
                    .iter()
                    .filter(|id| active_live.contains(&id.to_string()))
                    .cloned()
                    .collect();
                if is_name_live {
                    outgoing_env.push(join.name.clone());
                }
                emit_join(
                    &mut preamble,
                    &mut pipeline,
                    &env,
                    &outgoing_env,
                    &join.name,
                    &join.source,
                    &join.outer_key,
                    &join.inner_key,
                    join_counter,
                    &mut pending_lets,
                    fused,
                );
                env = outgoing_env;
                join_counter += 1;
                if consumed {
                    // We swallowed the next middle clause (the where).
                    // Advance the index an extra step so the main loop
                    // doesn't process it again.
                    i += 1;
                }
            }
            MiddleClause::GroupBy { element, key, name } => {
                // group by is a pipeline barrier like orderby: we must
                // see every element before we can yield a single group.
                // Commit any pending sorts first (using live_before so
                // the sort output still contains what `element`/`key`
                // read), then emit the group-by stage.
                env = flush_sorts(
                    &mut pipeline, &env, &mut pending_sorts, &mut pending_lets, &live_before,
                );
                // After group-by the only surviving binding is `name`;
                // everything else dies at this barrier. So outgoing env
                // is just `[name]` if name is live, else empty.
                let mut outgoing_env: Vec<Ident> = Vec::new();
                if live.contains(&name.to_string()) {
                    outgoing_env.push(name.clone());
                }
                emit_group_by(
                    &mut pipeline,
                    &env,
                    &outgoing_env,
                    name,
                    element,
                    key,
                    &mut pending_lets,
                );
                env = outgoing_env;
            }
        }
        i += 1;
    }
    // Final flush before select. For the tail flush, the "live set" is the
    // reads of the select expression; the last entry in live_after is
    // unused (always empty), so we derive this set directly.
    let live_before_select: HashSet<String> =
        crate::liveness::reads_of_select(&query.select);
    env = flush_sorts(
        &mut pipeline,
        &env,
        &mut pending_sorts,
        &mut pending_lets,
        &live_before_select,
    );

    // select: destructure the (now minimal) env tuple, project to output.
    // The env here is already the shape of the tuple flowing in from the
    // last emitted step; it includes pending-let names that are live.
    // emit_select removes the pending names from incoming_env internally.
    emit_select(
        &mut pipeline,
        &env,
        &query.select,
        &mut pending_lets,
    );

    // Wrapper: source → into_iter → initial env tuple → pipeline.
    //
    // For the single-binding case (the vastly most common `from x in …`
    // shape) the initial env tuple IS the range variable, so the initial
    // `.map(|x| x)` would be pure identity. Skip it; one fewer closure
    // for the optimiser to fold away, and a cleaner `cargo expand` output.
    let initial_tuple_tokens = tuple_build(&initial_env);
    let initial_map = if initial_env.len() == 1 {
        // Single binding: `from x in …` already yields `x` per element.
        // No wrap, no destructure, nothing.
        quote!()
    } else {
        quote!( .map(|#initial_pat_tokens| #initial_tuple_tokens) )
    };
    quote! {
        {
            #[allow(unused_imports)]
            use ::oql::__private::SortAndStrip as _;
            #preamble
            ::core::iter::IntoIterator::into_iter(#source)
                #initial_map
                #pipeline
        }
    }
}

// --- from analysis ------------------------------------------------------------

/// Returns (pattern tokens for the initial map, initial environment bindings).
///
/// For `from x in xs` → (`x`, `[x]`).
/// For non-identifier patterns we emit a compile error and return a dummy,
/// because downstream clauses would have no name to reference. The user can
/// always destructure in the body with `let`.
fn analyze_from(from: &FromClause) -> (TokenStream, Vec<Ident>) {
    match &from.pat {
        Pat::Ident(pi) => {
            let id = pi.ident.clone();
            (quote!(#id), vec![id])
        }
        other => {
            let span = syn::spanned::Spanned::span(other);
            let err = syn::Error::new(
                span,
                "`from` requires a plain identifier pattern. Use `let` inside the \
                 query body for destructuring.",
            )
            .to_compile_error();
            (
                err,
                vec![Ident::new("__oql_err", proc_macro2::Span::call_site())],
            )
        }
    }
}

// --- clause emitters ----------------------------------------------------------

/// Builds a sequence of `let` statements from pending lets, in source order.
/// After this emits, the caller's closure sees each name in scope; the
/// `live` flag only controls whether the name ends up in the outgoing
/// environment tuple, not whether the let itself is emitted. Side effects
/// of the let expression are always preserved.
fn let_statements(pending: &[(Ident, Expr, bool, bool)]) -> TokenStream {
    let lines = pending.iter().map(|(n, v, _live, _shadow)| quote!( let #n = #v; ));
    quote!( #( #lines )* )
}

/// Returns a `TokenStream` concatenating every expression in `pending` so
/// `touch_unused` can scan it for identifier usage. Pending let values often
/// reference earlier bindings and keep them "alive" for this step.
fn pending_expr_tokens(pending: &[(Ident, Expr, bool, bool)]) -> TokenStream {
    pending.iter().flat_map(|(_, v, _, _)| quote!(#v)).collect()
}

fn emit_where(
    pipeline: &mut TokenStream,
    env: &[Ident],
    outgoing_env: &[Ident],
    cond: &Expr,
    pending_lets: &mut Vec<(Ident, Expr, bool, bool)>,
) {
    // The closure destructures the *incoming* env (without the pending lets),
    // executes the pending lets, then evaluates the condition. The rebuilt
    // tuple flowing downstream uses the *outgoing* env; which only contains
    // bindings that are still live after this step.
    //
    // Incoming env = env with pending-let names removed. We filter by
    // identifier name rather than by count because a live flag + env
    // filtering in earlier steps may have produced counts that don't
    // align one-to-one with position.
    let pending_names: HashSet<String> = pending_lets
        .iter()
        .filter(|(_n, _, live, shadow)| *live && !*shadow)
        .map(|(n, _, _, _)| n.to_string())
        .collect();
    let incoming_env: Vec<Ident> = env
        .iter()
        .filter(|id| !pending_names.contains(&id.to_string()))
        .cloned()
        .collect();
    let pat = tuple_pat(&incoming_env);
    let rebuild = tuple_build(outgoing_env);
    let lets = let_statements(pending_lets);
    let cond_tokens = quote!(#cond);
    let all_used_tokens = {
        let mut t = cond_tokens.clone();
        t.extend(pending_expr_tokens(pending_lets));
        t
    };
    let touch = touch_unused(&incoming_env, &all_used_tokens);
    pending_lets.clear();
    pipeline.extend(quote! {
        .filter_map(|#pat| {
            #touch
            #lets
            if #cond {
                ::core::option::Option::Some(#rebuild)
            } else {
                ::core::option::Option::None
            }
        })
    });
}

fn emit_select(
    pipeline: &mut TokenStream,
    env: &[Ident],
    select: &SelectClause,
    pending_lets: &mut Vec<(Ident, Expr, bool, bool)>,
) {
    // For select, the env passed in is already live-filtered (by expand).
    // Pending lets are emitted as statements so they're in scope for the
    // projection, whether or not the projection reads them (side effects).
    //
    // Incoming env = env with pending-let names removed. Filter by name
    // rather than by count, because earlier steps may have pruned bindings
    // by liveness and the counts no longer align with positions.
    let pending_names: HashSet<String> = pending_lets
        .iter()
        .filter(|(_n, _, live, shadow)| *live && !*shadow)
        .map(|(n, _, _, _)| n.to_string())
        .collect();
    let incoming_env: Vec<Ident> = env
        .iter()
        .filter(|id| !pending_names.contains(&id.to_string()))
        .cloned()
        .collect();
    let pat = tuple_pat(&incoming_env);
    let proj = &select.expr;
    let lets = let_statements(pending_lets);
    let proj_tokens = quote!(#proj);
    let all_used_tokens = {
        let mut t = proj_tokens.clone();
        t.extend(pending_expr_tokens(pending_lets));
        t
    };
    let touch = touch_unused(&incoming_env, &all_used_tokens);
    pending_lets.clear();
    pipeline.extend(quote! {
        .map(|#pat| {
            #touch
            #lets
            #proj
        })
    });
}

// --- join ---------------------------------------------------------------------

/// Emits an inner equi-join as a hash-join.
///
/// Two code sites are produced:
///
/// * **Preamble** (once, before the pipeline runs): consumes the inner
///   source, groups it by the inner key into a `HashMap<K, Vec<T>>`. Buckets
///   handle duplicate keys correctly; SQL `INNER JOIN` semantics emits one
///   output row per matching pair, so we need to keep all matches.
///
/// * **Pipeline step**: for each outer element, compute the outer key and
///   do a hash lookup. The lookup yields `&Vec<T>`; we iterate it, clone
///   each inner item (to get an owned `T` for the downstream environment),
///   and produce one new environment tuple per match. Outer elements with
///   no match drop out (inner-join semantics).
///
/// Complexity: O(n_inner) to build the map, then O(1) average per outer
/// element plus O(matches) to emit. Overall O(n + m + matches), which
/// beats the naive cartesian O(n·m).
///
/// Requirements: the key type must be `Hash + Eq`. The inner element type
/// must be `Clone` because a single inner item may match multiple outer
/// items and vice versa.
#[allow(clippy::too_many_arguments)]
fn emit_join(
    preamble: &mut TokenStream,
    pipeline: &mut TokenStream,
    env: &[Ident],
    outgoing_env: &[Ident],
    name: &Ident,
    source: &Expr,
    outer_key: &Expr,
    inner_key: &Expr,
    counter: usize,
    pending_lets: &mut Vec<(Ident, Expr, bool, bool)>,
    // If `Some(cond)`, the following `where` clause is fused into this
    // join's flat_map body: matches whose (outer, inner) pair fails
    // `cond` are skipped before emission, no separate `.filter_map()`
    // stage is generated. This matches exactly what a hand-written
    // join+filter loop does, and saves one iterator adapter on the
    // critical path.
    fused_where: ::core::option::Option<&Expr>,
) {
    let map_ident = Ident::new(
        &format!("__oql_join_map_{counter}"),
        proc_macro2::Span::call_site(),
    );

    // Build the hash map once, before the pipeline starts.
    //
    // We preallocate the HashMap using the inner iterator's `size_hint`.
    // For typed collections (`Vec<T>`, slices, etc.) the lower bound is
    // exact, so we avoid the handful of realloc/rehash cycles that
    // `HashMap::new()` would otherwise do as it grows. For open-ended
    // iterators `size_hint` returns `(0, …)` and we fall back to the
    // default-sized map; same behaviour as before, no regression.
    preamble.extend(quote! {
        let #map_ident = {
            let __oql_inner_source = #source;
            let __oql_inner_iter = ::core::iter::IntoIterator::into_iter(__oql_inner_source);
            let __oql_hint = ::core::iter::Iterator::size_hint(&__oql_inner_iter).0;
            let mut __oql_map: ::std::collections::HashMap<_, ::std::vec::Vec<_>> =
                ::std::collections::HashMap::with_capacity(__oql_hint);
            for #name in __oql_inner_iter {
                let __oql_k = (&(#inner_key)).clone();
                __oql_map.entry(__oql_k).or_insert_with(::std::vec::Vec::new).push(#name);
            }
            __oql_map
        };
    });

    // `env` contains the live pending-let names already (they were
    // pushed as they were parsed, for those that are live after their
    // definition point). For the outer pattern of the `flat_map` we need
    // the env *without* the live pending lets; those will be re-executed
    // inside the closure. This saves us emitting a separate `.map()` step
    // just to introduce the let bindings.
    //
    // Dead pending lets (live=false) are NOT in env, so they don't affect
    // the count here; they're emitted as plain statements for side
    // effects but never make it into the environment tuple.
    // Incoming env = env with live pending-let names removed. Filter by
    // name rather than by count; earlier live-filtering may have left
    // the env shorter than a simple count suggests.
    let pending_names: HashSet<String> = pending_lets
        .iter()
        .filter(|(_n, _, live, shadow)| *live && !*shadow)
        .map(|(n, _, _, _)| n.to_string())
        .collect();
    let incoming_env: Vec<Ident> = env
        .iter()
        .filter(|id| !pending_names.contains(&id.to_string()))
        .cloned()
        .collect();
    let outer_pat = tuple_pat(&incoming_env);
    // The outgoing tuple contains only the bindings the live-analysis
    // said are still needed (minus dead intermediates).
    let new_tuple = tuple_build(outgoing_env);
    let lets = let_statements(pending_lets);
    let all_used = {
        let mut t = quote!(#outer_key);
        t.extend(pending_expr_tokens(pending_lets));
        if let ::core::option::Option::Some(cond) = fused_where {
            t.extend(quote!(#cond));
        }
        t
    };
    let touch = touch_unused(&incoming_env, &all_used);
    pending_lets.clear();

    // flat_map: for each outer, look up matches, emit one env per match.
    //
    // Semantics of inner join:
    //   0 matches → drop the outer element (no output).
    //   1 match   → emit exactly one environment tuple.
    //   N matches → emit N environment tuples, all sharing the outer
    //               bindings. This requires each outer binding to be `Clone`
    //               so the bindings can be duplicated into every emitted
    //               tuple. We force owned duplicates explicitly below.
    //
    // For each outer element we walk the matching bucket. Common case in
    // practice: exactly one match (foreign-key style join against a table
    // where the key is unique on the inner side). We special-case that
    // path to avoid a heap allocation per outer element:
    //
    //   matches.len() == 0  → JoinMatches::Empty
    //   matches.len() == 1  → JoinMatches::Once(Some(...)) ; no alloc
    //   matches.len() >= 2  → JoinMatches::Many(vec)       ; one Vec
    //
    // `JoinMatches` is a stack-allocated enum-iterator, so the dispatch is
    // a single branch on the variant tag; no vtable, no heap for the
    // iterator itself. The optimiser inlines the whole `next()` through
    // the pipeline on the single-match path.
    //
    // When a `where` immediately follows this join, it gets fused into
    // the emit path here: each match is checked against the condition
    // before push, exactly like a hand-written for-loop would. Saves one
    // iterator stage for the common join+filter pattern.
    //
    // We only clone bindings that end up in the outgoing tuple; dead
    // bindings don't need to be duplicated across matches.
    let clones_vec: Vec<TokenStream> = outgoing_env
        .iter()
        .filter(|i| i != &name)
        .map(|i| quote!( let #i = ::core::clone::Clone::clone(&#i); ))
        .collect();
    // Where-check: emitted either as an early-return (Once path, where
    // failing the filter means 0 matches now) or as a `continue` (Many
    // path, skipping this particular inner while still iterating the
    // bucket).
    let (once_where_check, many_where_check) = match fused_where {
        ::core::option::Option::Some(cond) => (
            quote! {
                if !(#cond) {
                    return ::oql::__private::JoinMatches::Empty;
                }
            },
            quote! {
                if !(#cond) {
                    continue;
                }
            },
        ),
        ::core::option::Option::None => (quote!(), quote!()),
    };
    pipeline.extend(quote! {
        .flat_map(move |#outer_pat| {
            #touch
            #lets
            let __oql_outer_key = (&(#outer_key)).clone();
            let __oql_matches = match #map_ident.get(&__oql_outer_key) {
                ::core::option::Option::None => {
                    return ::oql::__private::JoinMatches::Empty;
                }
                ::core::option::Option::Some(__m) => __m,
            };
            if __oql_matches.len() == 1 {
                let #name = ::core::clone::Clone::clone(&__oql_matches[0]);
                #( #clones_vec )*
                #once_where_check
                return ::oql::__private::JoinMatches::Once(
                    ::core::option::Option::Some(#new_tuple),
                );
            }
            let mut __oql_emit: ::std::vec::Vec<_> =
                ::std::vec::Vec::with_capacity(__oql_matches.len());
            for __oql_inner in __oql_matches.iter() {
                let #name = ::core::clone::Clone::clone(__oql_inner);
                #( #clones_vec )*
                #many_where_check
                __oql_emit.push(#new_tuple);
            }
            ::oql::__private::JoinMatches::Many(__oql_emit.into_iter())
        })
    });
}

// --- group-join ---------------------------------------------------------------

/// Emits a group-join (`join … into g`).
///
/// Like the inner join above it builds a `HashMap<K, Vec<Inner>>` from
/// the inner source in the preamble. But where the inner join does a
/// `flat_map` that emits N outputs per outer row, a group-join does a
/// plain `map` that emits exactly one output per outer row, with the
/// match list itself bound into the environment as a `Vec<Inner>`.
///
/// Left-join semantics: an outer row with no matches still produces an
/// output; the group vec is simply empty.
///
/// The `name` binding (the per-match inner row) technically doesn't
/// make sense in a group-join because there isn't a single match. We
/// still produce a `let` for it to keep the AST shape uniform, but the
/// user is not expected to reference it after `into g`; they should
/// reach through `g.iter()` etc. instead. If they do reference `name`,
/// they get an unused-binding warning at best, a use-after-move at
/// worst. The safety check in `expand` excludes `name` from the
/// outgoing env unless the liveness analysis says the user actually
/// read it; in which case that's on them.
#[allow(clippy::too_many_arguments)]
fn emit_group_join(
    preamble: &mut TokenStream,
    pipeline: &mut TokenStream,
    env: &[Ident],
    outgoing_env: &[Ident],
    name: &Ident,
    group_name: &Ident,
    source: &Expr,
    outer_key: &Expr,
    inner_key: &Expr,
    counter: usize,
    pending_lets: &mut Vec<(Ident, Expr, bool, bool)>,
) {
    let map_ident = Ident::new(
        &format!("__oql_gjoin_map_{counter}"),
        proc_macro2::Span::call_site(),
    );

    // Preamble: build the same HashMap<K, Vec<Inner>> as the inner join.
    // Re-uses the exact pattern for consistency and so `size_hint`
    // preallocation applies here too.
    preamble.extend(quote! {
        let #map_ident = {
            let __oql_inner_source = #source;
            let __oql_inner_iter = ::core::iter::IntoIterator::into_iter(__oql_inner_source);
            let __oql_hint = ::core::iter::Iterator::size_hint(&__oql_inner_iter).0;
            let mut __oql_map: ::std::collections::HashMap<_, ::std::vec::Vec<_>> =
                ::std::collections::HashMap::with_capacity(__oql_hint);
            for #name in __oql_inner_iter {
                let __oql_k = (&(#inner_key)).clone();
                __oql_map.entry(__oql_k).or_insert_with(::std::vec::Vec::new).push(#name);
            }
            __oql_map
        };
    });

    // Incoming env = env minus any live pending-lets (they'll be
    // re-emitted inside the closure). Same logic as emit_join.
    let pending_names: HashSet<String> = pending_lets
        .iter()
        .filter(|(_n, _, live, shadow)| *live && !*shadow)
        .map(|(n, _, _, _)| n.to_string())
        .collect();
    let incoming_env: Vec<Ident> = env
        .iter()
        .filter(|id| !pending_names.contains(&id.to_string()))
        .cloned()
        .collect();
    let outer_pat = tuple_pat(&incoming_env);
    let new_tuple = tuple_build(outgoing_env);
    let lets = let_statements(pending_lets);
    let all_used = {
        let mut t = quote!(#outer_key);
        t.extend(pending_expr_tokens(pending_lets));
        t
    };
    let touch = touch_unused(&incoming_env, &all_used);
    pending_lets.clear();

    // `.map(move |outer| { … (outer…, group) })`:
    //   - compute the outer key
    //   - look up the bucket; if missing, synthesise an empty Vec
    //   - clone the bucket so the HashMap survives for later outer rows
    //     (taking ownership would move items out on the first read)
    //   - bind `group_name` and emit the outgoing tuple
    //
    // Unlike the inner-join emit, there is no per-match binding here:
    // the caller strips `name` from the outgoing env before calling us
    // (a group-join has no single match, so the binding would be
    // meaningless; it's just a closure-parameter shadow mis-classified
    // by the over-approximate liveness analysis).
    let _ = name; // unused in the group-join emit path; name carries no runtime value
    pipeline.extend(quote! {
        .map(move |#outer_pat| {
            #touch
            #lets
            let __oql_outer_key = (&(#outer_key)).clone();
            let #group_name: ::std::vec::Vec<_> = match #map_ident.get(&__oql_outer_key) {
                ::core::option::Option::None => ::std::vec::Vec::new(),
                ::core::option::Option::Some(__m) => ::core::clone::Clone::clone(__m),
            };
            #new_tuple
        })
    });
}

// --- group by -----------------------------------------------------------------

/// Emits a `group by` clause.
///
/// Like `orderby`, grouping is a pipeline barrier: every upstream
/// element must be seen before the first group can be yielded. We
/// therefore `.collect()` the incoming tuple stream into a
/// `HashMap<K, Vec<T>>`, then drain it back out as an iterator of
/// `Group<K, T>` values.
///
/// After this stage the environment shrinks to just `[name]`. The
/// user writes `select (g.key, g.items.len())` or similar; anything
/// about the pre-group-by bindings is gone; they would have been
/// per-element, and the group is a per-key aggregation.
fn emit_group_by(
    pipeline: &mut TokenStream,
    env: &[Ident],
    outgoing_env: &[Ident],
    name: &Ident,
    element: &Expr,
    key: &Expr,
    pending_lets: &mut Vec<(Ident, Expr, bool, bool)>,
) {
    // Same incoming-env computation as the other emitters.
    let pending_names: HashSet<String> = pending_lets
        .iter()
        .filter(|(_n, _, live, shadow)| *live && !*shadow)
        .map(|(n, _, _, _)| n.to_string())
        .collect();
    let incoming_env: Vec<Ident> = env
        .iter()
        .filter(|id| !pending_names.contains(&id.to_string()))
        .cloned()
        .collect();
    let pat = tuple_pat(&incoming_env);
    let lets = let_statements(pending_lets);
    let all_used = {
        let mut t = quote!(#element);
        t.extend(quote!(#key));
        t.extend(pending_expr_tokens(pending_lets));
        t
    };
    let touch = touch_unused(&incoming_env, &all_used);
    pending_lets.clear();

    // The group stream's environment is just `[name]` if the
    // group-by's output is used, or empty if it's not. We always bind
    // `name` for the downstream stages; if unused, Rust will warn,
    // which is the correct signal.
    let is_name_live = outgoing_env.iter().any(|i| i == name);
    let out_tuple = if is_name_live {
        tuple_build(::std::slice::from_ref(name))
    } else {
        // Nothing downstream references the group; still emit a
        // unit-ish tuple so the pipeline shape is consistent. This
        // case is rare; it means the user wrote `group … into g` and
        // then `select 42` or similar.
        quote!(())
    };

    pipeline.extend(quote! {
        .fold(
            ::std::collections::HashMap::<_, ::std::vec::Vec<_>>::new(),
            |mut __oql_groups, #pat| {
                #touch
                #lets
                let __oql_k = (&(#key)).clone();
                let __oql_e = #element;
                __oql_groups
                    .entry(__oql_k)
                    .or_insert_with(::std::vec::Vec::new)
                    .push(__oql_e);
                __oql_groups
            },
        )
        .into_iter()
        .map(|(__oql_k, __oql_items)| {
            let #name = ::oql::__private::Group {
                key: __oql_k,
                items: __oql_items,
            };
            #out_tuple
        })
    });
}

// --- orderby flushing ---------------------------------------------------------

/// Folds all pending `orderby` clauses into a single sort.
///
/// The user writes `orderby item.name` and expects `item` to remain
/// available to subsequent clauses. The generated code therefore:
/// 1. Computes the key without consuming parts of the element.
/// 2. Stores the key next to the element as `(key, env)`.
/// 3. Sorts that vec by comparing keys directly (no extra cloning).
/// 4. Drops the key and yields the untouched environment tuple.
///
/// Key extraction uses `(&expr).clone()`: method-call auto-deref turns a
/// value expression into a borrow, and `.clone()` produces an owned key
/// without moving the source. For `Copy` types (`i32`, `&str`, …) the clone
/// compiles to a direct copy. For `String`, `Vec<T: Clone>`, … it is one
/// real clone per element; the minimum any sort implementation needs.
///
/// Composite keys: multiple `orderby` clauses become a tuple key. Rust sorts
/// tuples lexicographically, so source order equals priority order. `desc`
/// wraps the affected key part in `core::cmp::Reverse`.
///
/// Allocation budget per element, per `orderby` block:
/// - `Copy` key: zero heap allocations.
/// - Non-`Copy` key: exactly one clone per element. No caching, no dual
///   clone; the key is computed once and then compared by reference
///   during sorting.
///
/// Returns the updated environment after the flush. If there were no
/// pending sorts, `env` is returned unchanged; otherwise the env is
/// live-filtered (bindings not in `live_after` are dropped), matching
/// the shape of the tuple the sort step now emits.
fn flush_sorts(
    pipeline: &mut TokenStream,
    env: &[Ident],
    pending: &mut Vec<(Expr, bool)>,
    pending_lets: &mut Vec<(Ident, Expr, bool, bool)>,
    live_after: &HashSet<String>,
) -> Vec<Ident> {
    if pending.is_empty() {
        // Even without sorts: pending lets stay queued for the next step
        // (where/join/select) to consume. Env shape unchanged.
        return env.to_vec();
    }

    // `env` includes the live pending-let names already; the sort-key
    // closure destructures the incoming env and re-executes the lets
    // inside. Filter by name rather than count.
    let pending_names: HashSet<String> = pending_lets
        .iter()
        .filter(|(_n, _, live, shadow)| *live && !*shadow)
        .map(|(n, _, _, _)| n.to_string())
        .collect();
    let incoming_env: Vec<Ident> = env
        .iter()
        .filter(|id| !pending_names.contains(&id.to_string()))
        .cloned()
        .collect();
    let owned_pat = tuple_pat(&incoming_env);
    // The outgoing tuple only carries bindings that survive the sort;
    // anything not in `live_after` is dropped. This shrinks the
    // `(Key, Env)` pairs the sort has to shuffle around, which is the
    // biggest payoff of live analysis for top-N-style queries.
    let outgoing_env: Vec<Ident> = env
        .iter()
        .filter(|id| live_after.contains(&id.to_string()))
        .cloned()
        .collect();
    let rebuild = tuple_build(&outgoing_env);
    let lets = let_statements(pending_lets);

    // Build composite key. Each part becomes an owned value via
    // `(&(expr)).clone()`; method-call auto-deref turns the value into a
    // borrow and `.clone()` produces an owned key. Copy types compile to a
    // plain copy; non-Copy types (like String) clone exactly once per
    // element, which is the minimum any stable sort needs.
    let key_items: Vec<TokenStream> = pending
        .iter()
        .map(|(expr, desc)| {
            let owned = quote!( (&(#expr)).clone() );
            if *desc {
                quote!( ::core::cmp::Reverse(#owned) )
            } else {
                owned
            }
        })
        .collect();
    // Single-key case: skip the 1-tuple wrap to keep the expanded code
    // readable and avoid an unnecessary destructure.
    let key_tuple = if key_items.len() == 1 {
        let only = &key_items[0];
        quote!( #only )
    } else {
        quote!( ( #( #key_items ),* ) )
    };

    let all_keys: TokenStream = pending
        .iter()
        .flat_map(|(e, _)| quote!(#e))
        .chain(pending_expr_tokens(pending_lets))
        .collect();
    let touch = touch_unused(&incoming_env, &all_keys);

    pending.clear();
    pending_lets.clear();

    pipeline.extend(quote! {
        .map(|#owned_pat| {
            #touch
            #lets
            let __oql_key = #key_tuple;
            (__oql_key, #rebuild)
        })
        .collect::<::std::vec::Vec<_>>()
        .__oql_sort_and_strip()
    });

    // The sort step emits tuples shaped by `outgoing_env`, so that is the
    // new env the downstream pipeline sees.
    outgoing_env
}

// --- tuple helpers ------------------------------------------------------------
//
// For a single binding (very common: just `from x in …` with no `let`s yet)
// we skip the tuple wrap entirely and use the bare identifier. This avoids
// a pile of `(x,)` / destructuring pairs that the compiler would otherwise
// have to fold away. For two or more bindings we use a real tuple.

fn tuple_pat(env: &[Ident]) -> TokenStream {
    match env.len() {
        0 => quote!(()),
        1 => {
            let only = &env[0];
            quote!(#only)
        }
        _ => {
            let items = env.iter();
            quote!( ( #( #items ),* ) )
        }
    }
}

fn tuple_build(env: &[Ident]) -> TokenStream {
    match env.len() {
        0 => quote!(()),
        1 => {
            let only = &env[0];
            quote!(#only)
        }
        _ => {
            let items = env.iter();
            quote!( ( #( #items ),* ) )
        }
    }
}

/// Emits `let _ = &x;` only for bindings that are *not* referenced in
/// `expr`. This keeps the generated code small: the compiler has fewer
/// no-ops to fold away, and in many queries (`select x` with only one
/// binding) the touch block is empty.
///
/// The scan is conservative: any token matching a binding's name counts as
/// usage, even if it's actually a field name in a struct literal or similar.
/// That only means we occasionally skip a `let _ = &x;` we could have
/// emitted; the worst case is an `unused_variables` warning, which we
/// never trigger because we wrap the block in a scope that consumes the
/// bindings via the tuple destructure anyway. In practice: zero warnings,
/// less generated code.
fn touch_unused(env: &[Ident], expr_tokens: &TokenStream) -> TokenStream {
    let used = collect_idents(expr_tokens);
    let lines = env.iter().filter_map(|i| {
        if used.iter().any(|u| u == &i.to_string()) {
            None
        } else {
            Some(quote!( let _ = &#i; ))
        }
    });
    quote!( #( #lines )* )
}

/// Collects every identifier that appears in a token stream.
///
/// Walks `TokenTree` recursively. We deliberately accept false positives
/// (identifiers in struct field positions, method names, etc.); overcounting
/// only means we skip emitting an `unused` marker, which is always safe.
fn collect_idents(ts: &TokenStream) -> Vec<String> {
    use proc_macro2::TokenTree;
    let mut out = Vec::new();
    for tt in ts.clone() {
        match tt {
            TokenTree::Ident(i) => out.push(i.to_string()),
            TokenTree::Group(g) => out.extend(collect_idents(&g.stream())),
            _ => {}
        }
    }
    out
}
