use oql::oql;

fn main() {
    let xs = vec![1, 2, 3];
    let _: Vec<_> = oql! {
        from x in xs
        wheere x > 1
        select x
    }
    .collect();
}
