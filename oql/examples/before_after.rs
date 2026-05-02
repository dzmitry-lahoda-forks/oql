//! Before/after demonstration: top-3 highest-paying European customers by
//! net revenue. Same problem, written twice.
//!
//! Money is in cents (u64) to keep the example focused; floating point
//! doesn't implement `Ord`, and sidestepping that with fixed-point math is
//! standard practice for financial data anyway.

use oql::oql;

#[derive(Clone, Debug)]
struct Order {
    customer_id: u32,
    price_cents: u64,
    quantity: u32,
    discount_bp: u64, // basis points: 100 = 1%, 1000 = 10%
}

#[derive(Clone, Debug)]
struct Customer {
    id: u32,
    name: &'static str,
    country: &'static str,
}

fn sample_data() -> (Vec<Order>, Vec<Customer>) {
    let orders = vec![
        Order {
            customer_id: 1,
            price_cents: 4990,
            quantity: 3,
            discount_bp: 1000,
        },
        Order {
            customer_id: 2,
            price_cents: 19900,
            quantity: 1,
            discount_bp: 0,
        },
        Order {
            customer_id: 1,
            price_cents: 1250,
            quantity: 10,
            discount_bp: 500,
        },
        Order {
            customer_id: 3,
            price_cents: 120000,
            quantity: 1,
            discount_bp: 2000,
        },
        Order {
            customer_id: 4,
            price_cents: 3500,
            quantity: 4,
            discount_bp: 0,
        },
        Order {
            customer_id: 2,
            price_cents: 50000,
            quantity: 2,
            discount_bp: 1500,
        },
    ];
    let customers = vec![
        Customer {
            id: 1,
            name: "Anna",
            country: "DE",
        },
        Customer {
            id: 2,
            name: "Ben",
            country: "DE",
        },
        Customer {
            id: 3,
            name: "Chiara",
            country: "IT",
        },
        Customer {
            id: 4,
            name: "Dmitri",
            country: "US",
        },
    ];
    (orders, customers)
}

// ─── Without oql ──────────────────────────────────────────────

fn top_european_revenue_plain(
    orders: Vec<Order>,
    customers: Vec<Customer>,
) -> Vec<(&'static str, u64)> {
    let by_id: std::collections::HashMap<u32, Customer> =
        customers.into_iter().map(|c| (c.id, c)).collect();

    let mut joined: Vec<(&'static str, u64)> = orders
        .into_iter()
        .filter_map(|o| {
            let c = by_id.get(&o.customer_id)?;
            if c.country != "DE" && c.country != "IT" {
                return None;
            }
            let total = o.price_cents * o.quantity as u64;
            let net = total * (10_000 - o.discount_bp) / 10_000;
            Some((c.name, net))
        })
        .collect();

    joined.sort_by(|a, b| b.1.cmp(&a.1));
    joined.into_iter().take(3).collect()
}

// ─── With oql ─────────────────────────────────────────────────

fn top_european_revenue_oql(
    orders: Vec<Order>,
    customers: Vec<Customer>,
) -> Vec<(&'static str, u64)> {
    oql! {
        from o in orders
        join c in customers on o.customer_id == c.id
        where c.country == "DE" || c.country == "IT"
        let total = o.price_cents * o.quantity as u64
        let net = total * (10_000 - o.discount_bp) / 10_000
        orderby net desc
        select (c.name, net)
    }
    .take(3)
    .collect()
}

fn main() {
    let (orders, customers) = sample_data();

    let plain = top_european_revenue_plain(orders.clone(), customers.clone());
    let fancy = top_european_revenue_oql(orders, customers);

    assert_eq!(plain, fancy);

    println!("Top 3 European customers by net revenue:");
    for (name, net_cents) in &fancy {
        println!("  {:<8} {:>10.2} €", name, *net_cents as f64 / 100.0);
    }
}
