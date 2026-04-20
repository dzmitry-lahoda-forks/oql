//! Tests for the `group by` and group-join clauses.
//!
//! `group by` is a pipeline barrier like `orderby`: every upstream element
//! must be observed before any group can be yielded. After the clause, the
//! environment collapses to just the group binding; earlier bindings are
//! no longer in scope, because they were per-element and groups are
//! per-key aggregations.
//!
//! Group-join (`join … into g`) produces exactly one outer-environment
//! tuple per outer row, with `g` bound to a `Vec<Inner>` of matches.
//! An empty group means the outer row has no matches, giving left-join
//! semantics.

use oql::oql;

// --- group by ----------------------------------------------------------------

#[test]
fn group_by_counts_per_key() {
    let nums = vec![1, 2, 3, 4, 5, 6, 7, 8];

    // Group by parity, produce (parity, count) pairs.
    let mut out: Vec<(u32, usize)> = oql! {
        from n in nums
        group n by n % 2 into g
        select (g.key, g.items.len())
    }
    .collect();

    out.sort_by_key(|(k, _)| *k);
    assert_eq!(out, vec![(0, 4), (1, 4)]);
}

#[test]
fn group_by_with_aggregate_via_iterator() {
    #[derive(Clone)]
    struct Order { customer: &'static str, amount: u64 }

    let orders = vec![
        Order { customer: "A", amount: 10 },
        Order { customer: "B", amount: 20 },
        Order { customer: "A", amount: 30 },
        Order { customer: "B", amount: 5 },
        Order { customer: "A", amount: 15 },
    ];

    let mut out: Vec<(&'static str, u64)> = oql! {
        from o in orders
        group o by o.customer into g
        select (g.key, g.items.iter().map(|o| o.amount).sum::<u64>())
    }
    .collect();

    out.sort_by_key(|(k, _)| *k);
    assert_eq!(out, vec![("A", 55), ("B", 25)]);
}

#[test]
fn group_by_after_where() {
    // Filter first, then group. The filtered-out elements never reach
    // the grouping stage; we verify this by watching the group sizes.
    let nums = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

    let mut out: Vec<(u32, usize)> = oql! {
        from n in nums
        where n > 5
        group n by n % 2 into g
        select (g.key, g.items.len())
    }
    .collect();

    out.sort_by_key(|(k, _)| *k);
    // After filtering to {6,7,8,9,10}: even → 3, odd → 2.
    assert_eq!(out, vec![(0, 3), (1, 2)]);
}

#[test]
fn where_after_group_by_filters_groups() {
    // `where` after `group by` filters groups, not elements.
    let nums = vec![1, 2, 3, 4, 5, 6, 7];

    let mut out: Vec<(u32, usize)> = oql! {
        from n in nums
        group n by n % 3 into g
        where g.items.len() >= 3
        select (g.key, g.items.len())
    }
    .collect();

    out.sort_by_key(|(k, _)| *k);
    // Groups by n % 3:
    //   key 0 → {3, 6}    size 2; filtered out
    //   key 1 → {1, 4, 7} size 3; kept
    //   key 2 → {2, 5}    size 2; filtered out
    assert_eq!(out, vec![(1, 3)]);
}

#[test]
fn orderby_after_group_by_sorts_groups() {
    let nums = vec![5, 1, 5, 2, 5, 3, 3, 2];

    let out: Vec<(u32, usize)> = oql! {
        from n in nums
        group n by n into g
        orderby g.items.len() desc
        select (g.key, g.items.len())
    }
    .collect();

    // Groups: 5→3, 2→2, 3→2, 1→1. Sorted by size desc.
    // Ties (2 and 3) may appear in either order; check length ordering.
    assert_eq!(out[0], (5, 3));
    assert_eq!(out[3], (1, 1));
    assert!(out[1].1 == 2 && out[2].1 == 2);
}

#[test]
fn group_by_after_join() {
    // Group after a join: bindings from the join environment
    // (outer + inner) are all lost at the group-by barrier;
    // only the group survives.
    #[derive(Clone)]
    struct Order { customer_id: u32, amount: u64 }
    #[derive(Clone)]
    struct Customer { id: u32, country: &'static str }

    let orders = vec![
        Order { customer_id: 1, amount: 100 },
        Order { customer_id: 2, amount: 50 },
        Order { customer_id: 1, amount: 30 },
        Order { customer_id: 3, amount: 200 },
    ];
    let customers = vec![
        Customer { id: 1, country: "DE" },
        Customer { id: 2, country: "DE" },
        Customer { id: 3, country: "IT" },
    ];

    // Total revenue per country.
    let mut out: Vec<(&'static str, u64)> = oql! {
        from o in orders
        join c in customers on o.customer_id == c.id
        group o by c.country into g
        select (g.key, g.items.iter().map(|o| o.amount).sum::<u64>())
    }
    .collect();

    out.sort_by_key(|(k, _)| *k);
    assert_eq!(out, vec![("DE", 180), ("IT", 200)]);
}

#[test]
fn group_by_on_projection() {
    // Group an expression rather than the range variable itself.
    // Here we group amounts (the projection) by customer.
    #[derive(Clone)]
    struct Order { customer: &'static str, amount: u64 }

    let orders = vec![
        Order { customer: "A", amount: 10 },
        Order { customer: "B", amount: 20 },
        Order { customer: "A", amount: 30 },
    ];

    let mut out: Vec<(&'static str, Vec<u64>)> = oql! {
        from o in orders
        group o.amount by o.customer into g
        select (g.key, g.items.clone())
    }
    .collect();

    out.sort_by_key(|(k, _)| *k);
    assert_eq!(out[0].0, "A");
    assert_eq!({ let mut v = out[0].1.clone(); v.sort(); v }, vec![10, 30]);
    assert_eq!(out[1], ("B", vec![20]));
}

// --- group-join --------------------------------------------------------------

#[test]
fn group_join_basic() {
    #[derive(Clone)]
    struct Customer { id: u32, name: &'static str }
    #[derive(Clone)]
    struct Order { customer_id: u32 }

    let customers = vec![
        Customer { id: 1, name: "Anna" },
        Customer { id: 2, name: "Ben" },
    ];
    let orders = vec![
        Order { customer_id: 1 },
        Order { customer_id: 1 },
        Order { customer_id: 2 },
    ];

    let mut out: Vec<(&'static str, usize)> = oql! {
        from c in customers
        join o in orders on c.id == o.customer_id into o_group
        select (c.name, o_group.len())
    }
    .collect();

    out.sort_by_key(|(k, _)| *k);
    assert_eq!(out, vec![("Anna", 2), ("Ben", 1)]);
}

#[test]
fn group_join_empty_groups_preserved() {
    // Group-join has left-join semantics: outer rows with no match
    // still produce output, with an empty group.
    #[derive(Clone)]
    struct Customer { id: u32, name: &'static str }
    #[derive(Clone)]
    struct Order { customer_id: u32 }

    let customers = vec![
        Customer { id: 1, name: "Anna" },
        Customer { id: 2, name: "Ben" },
        Customer { id: 3, name: "Cleo" },
    ];
    // No orders for customer 2.
    let orders = vec![
        Order { customer_id: 1 },
        Order { customer_id: 3 },
        Order { customer_id: 1 },
    ];

    let mut out: Vec<(&'static str, usize)> = oql! {
        from c in customers
        join o in orders on c.id == o.customer_id into o_group
        select (c.name, o_group.len())
    }
    .collect();

    out.sort_by_key(|(k, _)| *k);
    assert_eq!(out, vec![("Anna", 2), ("Ben", 0), ("Cleo", 1)]);
}

#[test]
fn group_join_then_where() {
    // Filter outer rows by a property of their group; e.g. keep only
    // customers with at least one order.
    #[derive(Clone)]
    struct Customer { id: u32, name: &'static str }
    #[derive(Clone)]
    struct Order { customer_id: u32 }

    let customers = vec![
        Customer { id: 1, name: "Anna" },
        Customer { id: 2, name: "Ben" },
        Customer { id: 3, name: "Cleo" },
    ];
    let orders = vec![
        Order { customer_id: 1 },
        Order { customer_id: 3 },
    ];

    let mut out: Vec<&'static str> = oql! {
        from c in customers
        join o in orders on c.id == o.customer_id into o_group
        where !o_group.is_empty()
        select c.name
    }
    .collect();

    out.sort();
    assert_eq!(out, vec!["Anna", "Cleo"]);
}

#[test]
fn group_join_with_sum_on_group() {
    // Classic "orders per customer" report.
    #[derive(Clone)]
    struct Customer { id: u32, name: &'static str }
    #[derive(Clone)]
    struct Order { customer_id: u32, amount: u64 }

    let customers = vec![
        Customer { id: 1, name: "Anna" },
        Customer { id: 2, name: "Ben" },
    ];
    let orders = vec![
        Order { customer_id: 1, amount: 50 },
        Order { customer_id: 1, amount: 75 },
        Order { customer_id: 2, amount: 100 },
        Order { customer_id: 1, amount: 25 },
    ];

    let mut out: Vec<(&'static str, u64)> = oql! {
        from c in customers
        join o in orders on c.id == o.customer_id into o_group
        select (c.name, o_group.iter().map(|o| o.amount).sum::<u64>())
    }
    .collect();

    out.sort_by_key(|(k, _)| *k);
    assert_eq!(out, vec![("Anna", 150), ("Ben", 100)]);
}
