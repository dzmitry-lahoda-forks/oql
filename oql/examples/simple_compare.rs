//! Minimal before/after: hand-written vs oql! for a trivial filter+map.
//!
//! Run with `cargo expand --example simple_compare` to see the expanded
//! form of the oql invocation side-by-side with the hand-written version.

use oql::oql;

fn plain(nums: Vec<i32>) -> Vec<i32> {
    nums.into_iter()
        .filter(|n| *n > 2)
        .map(|n| n * 10)
        .collect()
}

fn with_oql(nums: Vec<i32>) -> Vec<i32> {
    oql! {
        from n in nums
        where n > 2
        let tentimes = n * 10
        select tentimes
    }
    .collect()
}

fn main() {
    let xs = vec![1, 2, 3, 4, 5];
    let a = plain(xs.clone());
    let b = with_oql(xs);
    assert_eq!(a, b);
    println!("{:?}", a);
}
