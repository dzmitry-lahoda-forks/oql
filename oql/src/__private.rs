//! Internal helpers referenced by generated macro code.
//!
//! Implementation detail. The types in here are used by the code that
//! `oql!` expands to, not by user code directly. This module is
//! `#[doc(hidden)]` because the contract between the macro and its
//! helpers is internal: the macro is the public API, and the macro's
//! observable behaviour stays stable across versions regardless of how
//! this module evolves.

/// Sorts a `Vec<(K, T)>` by `K` in place and returns an iterator over the
/// stripped `T`s. Used by `orderby` expansion.
///
/// The generated code maps each element to a `(key, element)` pair,
/// collects into a `Vec`, and hands it here. We sort by comparing keys
/// directly (one comparison per pair, no re-cloning of keys during the
/// sort), then drop the keys and yield the elements.
///
/// Composite keys are plain tuples; Rust's `Ord` for tuples is
/// lexicographic, so earlier `orderby` clauses naturally take priority.
/// `desc` is encoded by wrapping the relevant key part in
/// `core::cmp::Reverse`.
///
/// Allocation budget per element: exactly one key clone (a no-op for
/// `Copy` types) and the vec itself. Sorting is `O(n log n)` comparisons,
/// `O(1)` extra memory; matching `sort_by_cached_key` on a plain
/// `Vec<T>`, but with the key expression written at the call site (so we
/// handle composite keys and `desc` without a typed closure signature).
pub trait SortAndStrip: Sized {
    /// Inner element type (the `T` in `Vec<(K, T)>`).
    type Item;
    /// Iterator returned after sorting.
    type Out: Iterator<Item = Self::Item>;
    /// Sort by the first tuple component, then yield the second.
    fn __oql_sort_and_strip(self) -> Self::Out;
}

impl<K, T> SortAndStrip for ::std::vec::Vec<(K, T)>
where
    K: Ord,
{
    type Item = T;
    type Out = ::core::iter::Map<::std::vec::IntoIter<(K, T)>, fn((K, T)) -> T>;

    #[inline]
    fn __oql_sort_and_strip(mut self) -> Self::Out {
        self.sort_by(|a, b| a.0.cmp(&b.0));
        fn strip<K, T>((_k, t): (K, T)) -> T {
            t
        }
        self.into_iter().map(strip::<K, T>)
    }
}

/// Stack-allocated iterator for the 0/1/N match cases of a hash-join.
///
/// The hot path of a typical foreign-key join yields exactly one match
/// per outer element. Boxing a `dyn Iterator` for that case costs a heap
/// allocation per outer row, which dominates the macro's runtime on
/// join-heavy queries.
///
/// This enum holds all three outcomes inline:
///
/// * `Empty`     ; no match; zero allocations.
/// * `Once(Some(x))`; exactly one match; zero allocations.
/// * `Many(vec)` ; two or more matches; one Vec allocation.
///
/// Dispatch happens through a regular `match`; because the enum is a
/// concrete type with known variants, the compiler inlines the
/// `Iterator::next` implementation end-to-end and the branch on the
/// variant tag fuses with the surrounding pipeline code.
pub enum JoinMatches<T> {
    /// No matches for this outer element.
    Empty,
    /// Exactly one match; holds the element until consumed, then `None`.
    Once(::core::option::Option<T>),
    /// Two or more matches, iterated as an owning vec.
    Many(::std::vec::IntoIter<T>),
}

impl<T> ::core::iter::Iterator for JoinMatches<T> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> ::core::option::Option<T> {
        match self {
            JoinMatches::Empty => ::core::option::Option::None,
            JoinMatches::Once(slot) => slot.take(),
            JoinMatches::Many(iter) => iter.next(),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, ::core::option::Option<usize>) {
        match self {
            JoinMatches::Empty => (0, ::core::option::Option::Some(0)),
            JoinMatches::Once(::core::option::Option::Some(_)) => {
                (1, ::core::option::Option::Some(1))
            }
            JoinMatches::Once(::core::option::Option::None) => {
                (0, ::core::option::Option::Some(0))
            }
            JoinMatches::Many(iter) => iter.size_hint(),
        }
    }
}


/// The output of a `group by` clause.
///
/// `Group<K, T>` is what the `oql!` macro's `group by ... into g` produces
/// on every iteration: `g.key` is the grouping key, `g.items` is the vec
/// of elements that shared that key. Downstream clauses (`where`,
/// `orderby`, `select`) see the group, not the individual elements.
///
/// `items` is exposed as a `Vec<T>` on purpose, so the user can call any
/// `Iterator` method on `g.items.iter()` (`sum`, `count`, `max`, `fold`,
/// whatever). No dedicated aggregate keywords are baked in: Rust's
/// `Iterator` trait already covers every aggregate worth having.
///
/// Group ordering within the result is unspecified. Use `orderby` after
/// `group by` if you want deterministic iteration.
#[derive(Debug, Clone)]
pub struct Group<K, T> {
    /// The value shared by all items in this group.
    pub key: K,
    /// The items themselves, in the order they were encountered during
    /// grouping.
    pub items: ::std::vec::Vec<T>,
}
