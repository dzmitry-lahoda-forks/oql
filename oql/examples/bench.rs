//! Small binary for inspecting the generated assembly.
//!
//! Run `cargo asm --bin bench oql_hand` / `cargo asm --bin bench oql_macro`
//! (with cargo-show-asm) to compare, or just `cargo rustc --release -- --emit=asm`
//! and grep for the fn names.

use oql::oql;

#[derive(Clone)]
pub struct User {
    pub name: &'static str,
    pub age: u32,
}

#[inline(never)]
pub fn oql_macro(users: Vec<User>) -> Vec<(&'static str, u32)> {
    oql! {
        from u in users
        where u.age >= 18
        orderby u.age
        select (u.name, u.age)
    }
    .collect()
}

#[inline(never)]
pub fn oql_hand(users: Vec<User>) -> Vec<(&'static str, u32)> {
    let mut filtered: Vec<User> = users.into_iter().filter(|u| u.age >= 18).collect();
    filtered.sort_by_key(|u| u.age);
    filtered.into_iter().map(|u| (u.name, u.age)).collect()
}

fn main() {
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
    let a = oql_macro(users.clone());
    let b = oql_hand(users);
    assert_eq!(a, b);
    println!("{:?}", a);
}
