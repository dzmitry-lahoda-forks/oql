//! Tests for arbitrary ordering of middle clauses.
//!
//! A user can write clauses in any order that makes semantic sense. These
//! tests pin down the observable behaviour so we don't regress it. Every
//! ordering must work; the user is in charge, not the macro.

use oql::oql;

#[test]
fn where_before_let() {
    let xs = vec![1, 2, 3, 4, 5];
    let out: Vec<i32> = oql! {
        from x in xs
        where x > 2
        let doubled = x * 2
        select doubled
    }
    .collect();
    assert_eq!(out, vec![6, 8, 10]);
}

#[test]
fn let_before_where() {
    let xs = vec![1, 2, 3, 4, 5];
    let out: Vec<i32> = oql! {
        from x in xs
        let doubled = x * 2
        where doubled > 4
        select doubled
    }
    .collect();
    assert_eq!(out, vec![6, 8, 10]);
}

#[test]
fn orderby_before_where_filters_after_sort() {
    // Order all items, then filter. Not the most efficient ordering,
    // but the user's choice must be honoured.
    let xs = vec![5, 1, 3, 2, 4];
    let out: Vec<i32> = oql! {
        from x in xs
        orderby x
        where x > 2
        select x
    }
    .collect();
    assert_eq!(out, vec![3, 4, 5]);
}

#[test]
fn where_before_orderby_filters_before_sort() {
    // Filter first, then sort. The more efficient ordering.
    let xs = vec![5, 1, 3, 2, 4];
    let out: Vec<i32> = oql! {
        from x in xs
        where x > 2
        orderby x
        select x
    }
    .collect();
    assert_eq!(out, vec![3, 4, 5]);
}

#[test]
fn alternating_let_where() {
    let xs = vec![1, 2, 3, 4, 5, 6];
    let out: Vec<i32> = oql! {
        from x in xs
        let a = x * 2
        where a > 4
        let b = a + 1
        where b < 12
        select b
    }
    .collect();
    // x=3 → a=6,  b=7   (7 < 12 → kept)
    // x=4 → a=8,  b=9   (9 < 12 → kept)
    // x=5 → a=10, b=11  (11 < 12 → kept)
    // x=6 → a=12, b=13  (13 < 12 → dropped)
    assert_eq!(out, vec![7, 9, 11]);
}

#[test]
fn let_between_orderbys() {
    // `let` between two `orderby`s must flush the pending sorts. Otherwise
    // the let binding wouldn't see its predecessor in a composite sort.
    let xs = vec![3, 1, 2];
    let out: Vec<i32> = oql! {
        from x in xs
        orderby x
        let doubled = x * 2
        orderby doubled desc
        select doubled
    }
    .collect();
    // After the first sort: [1,2,3]. After let: [2,4,6]. After second sort desc: [6,4,2].
    assert_eq!(out, vec![6, 4, 2]);
}

#[test]
fn multiple_wheres_order_independent() {
    // Two independent `where`s should produce the same result regardless
    // of the order they appear in.
    let xs = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let forward: Vec<i32> = oql! {
        from x in xs.clone()
        where x > 3
        where x < 8
        select x
    }
    .collect();
    let reverse: Vec<i32> = oql! {
        from x in xs
        where x < 8
        where x > 3
        select x
    }
    .collect();
    assert_eq!(forward, reverse);
    assert_eq!(forward, vec![4, 5, 6, 7]);
}

#[test]
fn where_between_joins() {
    // `where` must be able to filter on the joined binding between two joins.
    #[derive(Clone)]
    struct A {
        k: u32,
        v: &'static str,
    }
    #[derive(Clone)]
    struct B {
        k: u32,
        flag: bool,
        k2: u32,
    }
    #[derive(Clone)]
    struct C {
        k2: u32,
        w: &'static str,
    }

    let aa = vec![A { k: 1, v: "a" }];
    let bb = vec![
        B {
            k: 1,
            flag: true,
            k2: 10,
        },
        B {
            k: 1,
            flag: false,
            k2: 11,
        },
    ];
    let cc = vec![C { k2: 10, w: "x" }, C { k2: 11, w: "y" }];

    let out: Vec<(&'static str, &'static str)> = oql! {
        from a in aa
        join b in bb on a.k == b.k
        where b.flag
        join c in cc on b.k2 == c.k2
        select (a.v, c.w)
    }
    .collect();

    // Only b with flag=true survives, so only its k2 (=10) is used for the
    // second join → match with c="x".
    assert_eq!(out, vec![("a", "x")]);
}
