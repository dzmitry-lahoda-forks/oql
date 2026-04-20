use oql::oql;

fn main() {
    #[derive(Clone)]
    struct A { k: u32 }
    let xs = vec![A { k: 1 }];
    let ys = vec![A { k: 1 }];
    let _: Vec<_> = oql! {
        from a in xs
        join b in ys on a.k > b.k
        select a.k
    }
    .collect();
}
