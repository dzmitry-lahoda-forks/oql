//! Tests that `let` can shadow an existing binding, mirroring regular
//! Rust semantics where `let x = x * 2;` inside a scope rebinds `x` to
//! a new value computed from the old one.

use oql::oql;

#[test]
fn let_shadows_from_binding() {
    let xs = vec![1, 2, 3];
    let out: Vec<i32> = oql! {
        from x in xs
        let x = x * 10
        select x
    }.collect();
    assert_eq!(out, vec![10, 20, 30]);
}

#[test]
fn let_shadows_earlier_let() {
    let xs = vec![1, 2, 3];
    let out: Vec<i32> = oql! {
        from n in xs
        let y = n * 2
        let y = y + 100
        select y
    }.collect();
    assert_eq!(out, vec![102, 104, 106]);
}

#[test]
fn shadowing_let_then_where() {
    let xs = vec![1, 2, 3, 4, 5];
    let out: Vec<i32> = oql! {
        from x in xs
        let x = x * 10
        where x > 20
        select x
    }.collect();
    assert_eq!(out, vec![30, 40, 50]);
}

#[test]
fn shadowing_let_then_orderby() {
    let xs = vec![1, 2, 3];
    let out: Vec<i32> = oql! {
        from x in xs
        let x = x * 10
        orderby x desc
        select x
    }.collect();
    assert_eq!(out, vec![30, 20, 10]);
}

#[test]
fn multiple_consecutive_shadows() {
    let xs = vec![1, 2, 3];
    let out: Vec<i32> = oql! {
        from x in xs
        let x = x + 1
        let x = x * 2
        let x = x - 1
        select x
    }.collect();
    // (1+1)*2-1 = 3, (2+1)*2-1 = 5, (3+1)*2-1 = 7
    assert_eq!(out, vec![3, 5, 7]);
}

#[test]
fn non_shadowing_let_still_works() {
    // Regression check: a normal (non-shadowing) let must still carry its
    // name through the pipeline correctly.
    let xs = vec![1, 2, 3];
    let out: Vec<(i32, i32)> = oql! {
        from n in xs
        let doubled = n * 2
        let tripled = n * 3
        select (doubled, tripled)
    }.collect();
    assert_eq!(out, vec![(2, 3), (4, 6), (6, 9)]);
}
