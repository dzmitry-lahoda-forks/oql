//! Tests that verify the macro works from unusual scopes.
//!
//! The expansion uses absolute paths (`::core::`, `::std::`, `::oql::`) so
//! it must work regardless of the call site. These tests put the macro into
//! unusual positions; nested modules, functions, const-like contexts; to
//! catch any place where we accidentally rely on what's in scope at the
//! call site.

use oql::oql;

mod nested {
    use oql::oql;

    pub fn run() -> Vec<i32> {
        let xs = vec![1, 2, 3];
        oql! {
            from x in xs
            where x > 1
            select x * 10
        }
        .collect()
    }
}

mod doubly {
    pub mod nested {
        use oql::oql;

        pub fn run() -> Vec<(i32, i32)> {
            let xs = vec![(1, "a"), (2, "b"), (3, "c")];
            oql! {
                from item in xs
                orderby item.0 desc
                select (item.0, item.0 * item.0)
            }
            .collect()
        }
    }
}

#[test]
fn macro_works_inside_nested_module() {
    assert_eq!(nested::run(), vec![20, 30]);
}

#[test]
fn macro_works_inside_deeply_nested_module() {
    assert_eq!(doubly::nested::run(), vec![(3, 9), (2, 4), (1, 1)]);
}

#[test]
fn macro_works_inside_closure() {
    let query = |data: Vec<i32>| -> Vec<i32> {
        oql! {
            from x in data
            where x % 2 == 0
            select x
        }
        .collect()
    };
    assert_eq!(query(vec![1, 2, 3, 4, 5]), vec![2, 4]);
}

#[test]
fn two_independent_queries_in_one_function() {
    // Two macro expansions in the same scope must not collide on any
    // generated identifier (e.g. the `__oql_join_map_N` names).
    let xs = vec![1, 2, 3];
    let ys = vec![4, 5, 6];
    let sum_x: i32 = oql! {
        from x in xs
        select x
    }
    .sum();
    let sum_y: i32 = oql! {
        from y in ys
        select y
    }
    .sum();
    assert_eq!(sum_x, 6);
    assert_eq!(sum_y, 15);
}
