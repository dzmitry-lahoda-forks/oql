use oql::oql;

fn main() {
    let xs = vec![1, 2, 3];
    let _: Vec<_> = oql! {
        where x > 1
        from x in xs
        select x
    }
    .collect();
}
