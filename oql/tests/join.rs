//! Tests for the `join` clause.
//!
//! Covers inner-join semantics: hit → emit, miss → drop, N matches → N emits.

use oql::oql;

#[test]
fn basic_join_on_id() {
    #[derive(Clone)]
    struct Order { id: u32, customer_id: u32, amount: f64 }
    #[derive(Clone)]
    struct Customer { id: u32, name: &'static str }

    let orders = vec![
        Order { id: 1, customer_id: 10, amount: 50.0 },
        Order { id: 2, customer_id: 20, amount: 75.0 },
        Order { id: 3, customer_id: 10, amount: 30.0 },
    ];
    let customers = vec![
        Customer { id: 10, name: "Anna" },
        Customer { id: 20, name: "Ben" },
    ];

    let out: Vec<(u32, &'static str, f64)> = oql! {
        from o in orders
        join c in customers on o.customer_id == c.id
        select (o.id, c.name, o.amount)
    }
    .collect();

    assert_eq!(
        out,
        vec![
            (1, "Anna", 50.0),
            (2, "Ben", 75.0),
            (3, "Anna", 30.0),
        ]
    );
}

#[test]
fn join_drops_non_matching_outer() {
    // Order 2 has customer_id 99 which is not in the customers list; it must
    // disappear entirely (inner-join semantics).
    #[derive(Clone)]
    struct Order { id: u32, customer_id: u32 }
    #[derive(Clone)]
    struct Customer { id: u32, name: &'static str }

    let orders = vec![
        Order { id: 1, customer_id: 10 },
        Order { id: 2, customer_id: 99 },
        Order { id: 3, customer_id: 10 },
    ];
    let customers = vec![Customer { id: 10, name: "Anna" }];

    let out: Vec<(u32, &'static str)> = oql! {
        from o in orders
        join c in customers on o.customer_id == c.id
        select (o.id, c.name)
    }
    .collect();

    assert_eq!(out, vec![(1, "Anna"), (3, "Anna")]);
}

#[test]
fn join_emits_n_per_outer_on_n_matches() {
    // Two customer records share the same id; each matching order must
    // produce two output rows (one per matching customer).
    #[derive(Clone)]
    struct Order { id: u32, customer_id: u32 }
    #[derive(Clone)]
    struct Customer { id: u32, name: &'static str }

    let orders = vec![Order { id: 1, customer_id: 10 }];
    let customers = vec![
        Customer { id: 10, name: "Anna" },
        Customer { id: 10, name: "Ann" },
    ];

    let out: Vec<(u32, &'static str)> = oql! {
        from o in orders
        join c in customers on o.customer_id == c.id
        select (o.id, c.name)
    }
    .collect();

    assert_eq!(out, vec![(1, "Anna"), (1, "Ann")]);
}

#[test]
fn join_then_where() {
    #[derive(Clone)]
    struct Order { id: u32, customer_id: u32, amount: f64 }
    #[derive(Clone)]
    struct Customer { id: u32, country: &'static str }

    let orders = vec![
        Order { id: 1, customer_id: 10, amount: 50.0 },
        Order { id: 2, customer_id: 20, amount: 75.0 },
        Order { id: 3, customer_id: 10, amount: 200.0 },
    ];
    let customers = vec![
        Customer { id: 10, country: "DE" },
        Customer { id: 20, country: "US" },
    ];

    let out: Vec<u32> = oql! {
        from o in orders
        join c in customers on o.customer_id == c.id
        where c.country == "DE"
        where o.amount > 100.0
        select o.id
    }
    .collect();

    assert_eq!(out, vec![3]);
}

#[test]
fn two_joins_in_a_row() {
    #[derive(Clone)]
    struct A { k1: u32, v: &'static str }
    #[derive(Clone)]
    struct B { k1: u32, k2: u32 }
    #[derive(Clone)]
    struct C { k2: u32, w: &'static str }

    let aa = vec![A { k1: 1, v: "a1" }];
    let bb = vec![B { k1: 1, k2: 10 }];
    let cc = vec![C { k2: 10, w: "c1" }];

    let out: Vec<(&'static str, &'static str)> = oql! {
        from a in aa
        join b in bb on a.k1 == b.k1
        join c in cc on b.k2 == c.k2
        select (a.v, c.w)
    }
    .collect();

    assert_eq!(out, vec![("a1", "c1")]);
}

#[test]
fn join_with_string_key() {
    #[derive(Clone)]
    struct User { id: u32, name: String }
    #[derive(Clone)]
    struct Greeting { who: String, text: &'static str }

    let users = vec![
        User { id: 1, name: "alice".to_string() },
        User { id: 2, name: "bob".to_string() },
    ];
    let greetings = vec![
        Greeting { who: "alice".to_string(), text: "hi" },
        Greeting { who: "bob".to_string(), text: "hey" },
    ];

    let out: Vec<(u32, &'static str)> = oql! {
        from u in users
        join g in greetings on u.name == g.who
        select (u.id, g.text)
    }
    .collect();

    assert_eq!(out, vec![(1, "hi"), (2, "hey")]);
}
