//! # oql; Readable query syntax for Rust iterators
//!
//! `oql` is a macro that translates a declarative query syntax into plain
//! Rust iterator chains at compile time. The macro disappears in release
//! builds; the generated code is an ordinary iterator chain.
//!
//! ```rust
//! use oql::oql;
//!
//! let numbers = vec![1, 2, 3, 4, 5, 6];
//! let squared_evens: Vec<i32> = oql! {
//!     from x in numbers
//!     where x % 2 == 0
//!     select x * x
//! }.collect();
//!
//! assert_eq!(squared_evens, vec![4, 16, 36]);
//! ```
//!
//! The macro returns an `Iterator`. The caller decides what to do with it:
//! `.collect()`, `.sum()`, `.take(10).collect()`, `.for_each(...)`, and so
//! on. For simple queries the generated code is indistinguishable from a
//! hand-written `.filter(...).map(...)` chain. For complex queries (joins
//! with sorts and projections) the expansion carries some structural
//! overhead, measured in the crate's benchmark at roughly 1.07× the cost
//! of the equivalent handwritten pipeline. See the `Performance` section
//! in the repository README for details.
//!
//! ## Clauses
//!
//! | Clause                                  | Meaning |
//! |-----------------------------------------|---------|
//! | `from x in source`                      | Starts the query; expands to `source.into_iter()`, so any `IntoIterator` is accepted |
//! | `let name = expr`                       | Adds a per-element binding; `expr` is re-evaluated for each element and can reference the range variable and prior bindings |
//! | `where cond`                            | Filters elements |
//! | `orderby key`                           | Sorts ascending |
//! | `orderby key desc`                      | Sorts descending |
//! | `join y in src on a == b`               | Inner equality join (hash-join under the hood) |
//! | `join_must y in src on a == b`          | Inner equality join that panics if an outer row has no match |
//! | `join_left y in src on a == b`          | Left equality join; missing inner fields project as `None` |
//! | `last_must y in src on a == b`          | Keep only the last inner row per key and panic if none matches |
//! | `zip y in src`                          | Pair rows by position, stopping when either side ends |
//! | `zip_must y in src`                     | Pair rows by position; panics if the two sources have different lengths |
//! | `join y in src on a == b into g`        | Group-join: `g` is a `Vec<Y>` of matches (empty if none) |
//! | `group elem by key into g`              | Group elements by key; `g.key` and `g.items` downstream |
//! | `select expr`                           | Projects to the output type |
//!
//! `from` and `select` are mandatory. Everything else is optional and may
//! appear multiple times in any order. Clause order equals execution order,
//! so the pipeline reads linearly and behaves exactly like a hand-written
//! iterator chain would.

#![warn(missing_docs)]

#[doc(hidden)]
pub mod __private;

pub use oql_macro::oql;
