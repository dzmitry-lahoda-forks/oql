//! Tests for the `zip` clause.

use oql::oql;

#[test]
fn zip_pairs_rows_by_position() {
    #[derive(Clone, Debug, PartialEq)]
    struct User {
        id: u32,
    }
    #[derive(Clone, Debug, PartialEq)]
    struct Profile {
        name: &'static str,
    }

    let users = vec![User { id: 1 }, User { id: 2 }];
    let profiles = vec![Profile { name: "Anna" }, Profile { name: "Bob" }];

    let out: Vec<(u32, &'static str)> = oql! {
        from u in users
        zip p in profiles
        select (u.id, p.name)
    }
    .collect();

    assert_eq!(out, vec![(1, "Anna"), (2, "Bob")]);
}

#[test]
fn zip_stops_when_right_is_shorter() {
    let left = vec![1, 2, 3];
    let right = vec![10, 20];

    let out: Vec<_> = oql! {
        from l in left
        zip r in right
        select (l, r)
    }
    .collect();

    assert_eq!(out, vec![(1, 10), (2, 20)]);
}

#[test]
fn zip_stops_when_right_is_longer() {
    let left = vec![1, 2];
    let right = vec![10, 20, 30];

    let out: Vec<_> = oql! {
        from l in left
        zip r in right
        select (l, r)
    }
    .collect();

    assert_eq!(out, vec![(1, 10), (2, 20)]);
}

#[test]
#[should_panic(expected = "oql zip sources have different lengths")]
fn zip_must_panics_when_right_is_shorter() {
    let left = vec![1, 2, 3];
    let right = vec![10, 20];

    let _: Vec<_> = oql! {
        from l in left
        zip_must r in right
        select (l, r)
    }
    .collect();
}

#[test]
#[should_panic(expected = "oql zip sources have different lengths")]
fn zip_must_panics_when_right_is_longer() {
    let left = vec![1, 2];
    let right = vec![10, 20, 30];

    let _: Vec<_> = oql! {
        from l in left
        zip_must r in right
        select (l, r)
    }
    .collect();
}

#[test]
fn zip_composes_with_where_and_let() {
    let left = vec![1, 2, 3];
    let right = vec![10, 20, 30];

    let out: Vec<_> = oql! {
        from l in left
        zip r in right
        let sum = l + r
        where sum > 20
        select sum
    }
    .collect();

    assert_eq!(out, vec![22, 33]);
}
