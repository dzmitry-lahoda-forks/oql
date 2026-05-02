//! Integration tests covering every MVP feature.
//!
//! Each test is small and focused so a failure points straight at the
//! broken feature.

use oql::oql;

#[test]
fn just_from_and_select() {
    let xs = vec![1, 2, 3];
    let out: Vec<i32> = oql! {
        from x in xs
        select x
    }
    .collect();
    assert_eq!(out, vec![1, 2, 3]);
}

#[test]
fn select_transforms() {
    let xs = vec![1, 2, 3];
    let out: Vec<i32> = oql! {
        from x in xs
        select x * x
    }
    .collect();
    assert_eq!(out, vec![1, 4, 9]);
}

#[test]
fn where_filters() {
    let xs = vec![1, 2, 3, 4, 5, 6];
    let out: Vec<i32> = oql! {
        from x in xs
        where x % 2 == 0
        select x
    }
    .collect();
    assert_eq!(out, vec![2, 4, 6]);
}

#[test]
fn multiple_wheres_are_ands() {
    let xs = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let out: Vec<i32> = oql! {
        from x in xs
        where x >= 3
        where x <= 7
        where x % 2 == 1
        select x
    }
    .collect();
    assert_eq!(out, vec![3, 5, 7]);
}

#[test]
fn let_binding_is_visible_downstream() {
    let xs = vec![1, 2, 3, 4];
    let out: Vec<i32> = oql! {
        from x in xs
        let doubled = x * 2
        where doubled > 4
        select doubled
    }
    .collect();
    assert_eq!(out, vec![6, 8]);
}

#[test]
fn two_let_bindings_compose() {
    let xs = vec![1, 2, 3];
    let out: Vec<i32> = oql! {
        from x in xs
        let a = x * 2
        let b = a + 10
        select b
    }
    .collect();
    assert_eq!(out, vec![12, 14, 16]);
}

#[test]
fn orderby_ascending() {
    let xs = vec![3, 1, 4, 1, 5, 9, 2, 6];
    let out: Vec<i32> = oql! {
        from x in xs
        orderby x
        select x
    }
    .collect();
    assert_eq!(out, vec![1, 1, 2, 3, 4, 5, 6, 9]);
}

#[test]
fn orderby_descending() {
    let xs = vec![3, 1, 4, 1, 5, 9, 2, 6];
    let out: Vec<i32> = oql! {
        from x in xs
        orderby x desc
        select x
    }
    .collect();
    assert_eq!(out, vec![9, 6, 5, 4, 3, 2, 1, 1]);
}

#[test]
fn orderby_with_filter() {
    let xs = vec![5, 2, 8, 1, 9, 3];
    let out: Vec<i32> = oql! {
        from x in xs
        where x > 2
        orderby x desc
        select x
    }
    .collect();
    assert_eq!(out, vec![9, 8, 5, 3]);
}

#[test]
fn composite_orderby() {
    // Two keys: primary ascending by group, secondary descending by value.
    let items = vec![(1, 10), (2, 5), (1, 30), (2, 20), (1, 20)];
    let out: Vec<(i32, i32)> = oql! {
        from pair in items
        let g = pair.0
        let v = pair.1
        orderby g
        orderby v desc
        select (g, v)
    }
    .collect();
    assert_eq!(out, vec![(1, 30), (1, 20), (1, 10), (2, 20), (2, 5)]);
}

#[test]
fn struct_fields() {
    #[derive(Clone)]
    struct User {
        name: &'static str,
        age: u32,
    }
    let users = vec![
        User {
            name: "Anna",
            age: 30,
        },
        User {
            name: "Ben",
            age: 17,
        },
        User {
            name: "Carla",
            age: 42,
        },
    ];
    let adults: Vec<(&'static str, u32)> = oql! {
        from u in users
        where u.age >= 18
        orderby u.age
        select (u.name, u.age)
    }
    .collect();
    assert_eq!(adults, vec![("Anna", 30), ("Carla", 42)]);
}

#[test]
fn orderby_on_string_key_needs_no_clone() {
    // Sorting by a String key must not force the user to write `.clone()`.
    // The macro handles owning internally.
    let items = vec![
        ("charlie".to_string(), 3),
        ("alice".to_string(), 1),
        ("bob".to_string(), 2),
    ];
    let out: Vec<(String, i32)> = oql! {
        from item in items
        orderby item.0
        select item
    }
    .collect();
    assert_eq!(
        out,
        vec![
            ("alice".to_string(), 1),
            ("bob".to_string(), 2),
            ("charlie".to_string(), 3),
        ]
    );
}

#[test]
fn where_on_string_needs_no_deref() {
    // `where` conditions on non-Copy types work without `*` or `&` gymnastics.
    let names = vec!["anna".to_string(), "ben".to_string(), "clara".to_string()];
    let out: Vec<String> = oql! {
        from n in names
        where n.len() > 3
        select n
    }
    .collect();
    assert_eq!(out, vec!["anna".to_string(), "clara".to_string()]);
}

#[test]
fn chains_into_standard_iter_methods() {
    let xs = vec![1, 2, 3, 4, 5];
    let sum: i32 = oql! {
        from x in xs
        where x > 2
        select x
    }
    .sum();
    assert_eq!(sum, 12);
}
