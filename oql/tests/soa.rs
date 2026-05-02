use oql::oql;

#[test]
fn join_emits_n_per_outer_on_n_matches_soa() {
    // Two customer records share the same id; each matching order must
    // produce two output rows (one per matching customer).
    #[derive(Clone)]
    struct Order {
        id: u32,
        customer_id: u32,
    }
    #[derive(Clone)]
    struct Customer {
        id: u32,
        name: &'static str,
    }

    let orders = vec![Order {
        id: 1,
        customer_id: 10,
    }];
    let customers = vec![
        Customer {
            id: 10,
            name: "Anna",
        },
        Customer {
            id: 10,
            name: "Ann",
        },
    ];

    #[derive(layout::SOA, derive_more::From)]
    struct Report {
        order_id: u32,
        customer_name: &'static str,
    }

    let out: ReportVec = oql! {
        from o in orders
        join c in customers on o.customer_id == c.id
        select (
            o.id,
            c.name,
        )
    }
    .collect();

    assert_eq!(out.order_id, vec![1, 1]);
    assert_eq!(out.customer_name, vec!["Anna", "Ann"]);
}
