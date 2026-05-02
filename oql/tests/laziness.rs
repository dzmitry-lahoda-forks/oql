//! Laziness and evaluation-count tests.
//!
//! The macro is documented as expanding to iterator chains; and iterator
//! chains are *lazy*: downstream operators like `.take(n)` pull only as
//! many elements as they need, and upstream closures run only for the
//! elements that are actually consumed.
//!
//! These tests pin that contract down. They verify that:
//!   - `select` runs at most once per consumed element, not once per
//!     source element;
//!   - `where` and `let` do not trigger gratuitous extra evaluations;
//!   - pairing the macro with `.take(n)` really short-circuits.
//!
//! If a future optimisation were to, say, fuse `select` into a sort
//! preamble, these tests would catch the resulting extra evaluations.

use std::cell::Cell;

use oql::oql;

#[test]
fn select_not_called_past_take_limit() {
    let select_calls = Cell::new(0);
    let xs: Vec<i32> = (0..100).collect();
    let out: Vec<i32> = oql! {
        from x in xs
        select {
            select_calls.set(select_calls.get() + 1);
            x * 2
        }
    }
    .take(3)
    .collect();

    assert_eq!(out, vec![0, 2, 4]);
    // With true lazy evaluation, select runs exactly three times.
    // A value larger than 3 would mean the macro eagerly materialised
    // elements that take(3) never asked for.
    assert_eq!(
        select_calls.get(),
        3,
        "select must only run for the elements take(3) actually consumed",
    );
}

#[test]
fn select_not_called_when_take_is_zero() {
    let select_calls = Cell::new(0);
    let xs = vec![1, 2, 3];
    let out: Vec<i32> = oql! {
        from x in xs
        select {
            select_calls.set(select_calls.get() + 1);
            x
        }
    }
    .take(0)
    .collect();

    assert!(out.is_empty());
    assert_eq!(
        select_calls.get(),
        0,
        "select must not run if nothing is consumed"
    );
}

#[test]
fn where_short_circuits_select() {
    let select_calls = Cell::new(0);
    let xs: Vec<i32> = (0..10).collect();
    let _: Vec<i32> = oql! {
        from x in xs
        where x >= 5
        select {
            select_calls.set(select_calls.get() + 1);
            x
        }
    }
    .collect();

    // Only five elements (5..=9) pass the filter, so select runs five times.
    assert_eq!(select_calls.get(), 5);
}

#[test]
fn orderby_forces_full_consumption_but_select_still_respects_take() {
    // `orderby` has to see every element before it can yield the first;
    // that's inherent to sorting. But `select` runs AFTER the sort and
    // should still only fire for the elements that are actually pulled.
    let select_calls = Cell::new(0);
    let xs: Vec<i32> = (0..100).collect();
    let out: Vec<i32> = oql! {
        from x in xs
        orderby x desc
        select {
            select_calls.set(select_calls.get() + 1);
            x
        }
    }
    .take(3)
    .collect();

    assert_eq!(out, vec![99, 98, 97]);
    assert_eq!(
        select_calls.get(),
        3,
        "select must run lazily after the sort, not once per sorted element",
    );
}

#[test]
fn where_after_join_runs_once_per_match() {
    // A where-clause after a join must see each joined (outer, inner)
    // pair exactly once. This pins the evaluation count so optimisations
    // that fuse where into flat_map don't accidentally skip or duplicate
    // calls.
    let where_calls = Cell::new(0);
    let select_calls = Cell::new(0);

    #[derive(Clone)]
    struct A {
        k: u32,
    }
    #[derive(Clone)]
    struct B {
        k: u32,
        flag: bool,
    }

    let aa = vec![A { k: 1 }, A { k: 2 }, A { k: 3 }];
    let bb = vec![
        B { k: 1, flag: true },
        B { k: 1, flag: false },
        B { k: 2, flag: true },
        B { k: 4, flag: true }, // no match on a.k
    ];

    let out: Vec<u32> = oql! {
        from a in aa
        join b in bb on a.k == b.k
        where {
            where_calls.set(where_calls.get() + 1);
            b.flag
        }
        select {
            select_calls.set(select_calls.get() + 1);
            a.k
        }
    }
    .collect();

    // a=1 matches b.k=1 twice → 2 where calls (both rows)
    // a=2 matches b.k=2 once → 1 where call
    // a=3 matches nothing    → 0 where calls
    // total = 3 where calls
    assert_eq!(where_calls.get(), 3);
    // Select runs only on rows passing the filter:
    // a=1, b.flag=true  → keep
    // a=1, b.flag=false → drop
    // a=2, b.flag=true  → keep
    // total = 2 select calls
    assert_eq!(select_calls.get(), 2);
    assert_eq!(out, vec![1, 2]);
}
