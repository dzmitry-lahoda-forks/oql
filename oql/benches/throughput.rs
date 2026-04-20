//! Criterion benchmark comparing the hand-written pipeline against the
//! macro-expanded one for identical input and identical logic.
//!
//! The scenario is the before/after example scaled up: 10 000 orders
//! across 1 000 customers, filter to EU countries, compute net revenue,
//! sort descending, take top 3.

use std::collections::HashMap;
use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use oql::oql;

#[derive(Clone, Debug)]
struct Order {
    customer_id: u32,
    price_cents: u64,
    quantity: u32,
    discount_bp: u64,
}

#[derive(Clone, Debug)]
struct Customer {
    id: u32,
    name: String,
    country: &'static str,
}

fn make_data(n_orders: usize, n_customers: usize) -> (Vec<Order>, Vec<Customer>) {
    // Deterministic PRNG so runs are comparable. Simple LCG; we only need
    // cheap, repeatable pseudo-randomness, not crypto.
    let mut state: u64 = 0xc0ffee_1234_5678;
    let mut next = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        state
    };

    let countries = ["DE", "IT", "FR", "ES", "US", "GB", "NL", "PL"];
    let customers: Vec<Customer> = (0..n_customers)
        .map(|i| Customer {
            id: i as u32,
            name: format!("Customer{i}"),
            country: countries[(next() as usize) % countries.len()],
        })
        .collect();

    let orders: Vec<Order> = (0..n_orders)
        .map(|_| Order {
            customer_id: (next() as usize % n_customers) as u32,
            price_cents: (next() % 50_000) + 100,
            quantity: ((next() % 10) + 1) as u32,
            discount_bp: (next() % 3_000),
        })
        .collect();

    (orders, customers)
}

fn plain(orders: Vec<Order>, customers: Vec<Customer>) -> Vec<(String, u64)> {
    // Same hash-join shape the macro uses: HashMap<K, Vec<T>> with
    // per-outer Vec allocation for the matches. This keeps the comparison
    // fair; both variants do the same work for the same semantics (an
    // inner join that supports duplicate keys on the inner side).
    let mut by_id: HashMap<u32, Vec<Customer>> = HashMap::new();
    for c in customers {
        by_id.entry(c.id).or_insert_with(Vec::new).push(c);
    }

    let mut joined: Vec<(String, u64)> = orders
        .into_iter()
        .flat_map(|o| {
            let matches: Vec<(String, u64)> = match by_id.get(&o.customer_id) {
                None => Vec::new(),
                Some(cs) => {
                    let mut out = Vec::with_capacity(cs.len());
                    for c in cs {
                        if c.country != "DE" && c.country != "IT" {
                            continue;
                        }
                        let total = o.price_cents * o.quantity as u64;
                        let net = total * (10_000 - o.discount_bp) / 10_000;
                        out.push((c.name.clone(), net));
                    }
                    out
                }
            };
            matches.into_iter()
        })
        .collect();

    joined.sort_by(|a, b| b.1.cmp(&a.1));
    joined.into_iter().take(3).collect()
}

fn with_oql(orders: Vec<Order>, customers: Vec<Customer>) -> Vec<(String, u64)> {
    oql! {
        from o in orders
        join c in customers on o.customer_id == c.id
        where c.country == "DE" || c.country == "IT"
        let total = o.price_cents * o.quantity as u64
        let net = total * (10_000 - o.discount_bp) / 10_000
        orderby net desc
        select (c.name.clone(), net)
    }
    .take(3)
    .collect()
}

fn bench_pipelines(c: &mut Criterion) {
    let (orders, customers) = make_data(10_000, 1_000);

    // Sanity check: both produce identical results.
    let a = plain(orders.clone(), customers.clone());
    let b = with_oql(orders.clone(), customers.clone());
    assert_eq!(a, b, "plain and oql must produce identical results");

    let mut group = c.benchmark_group("top_eu_revenue");
    group.sample_size(200);

    group.bench_function("plain", |b_| {
        b_.iter_with_setup(
            || (orders.clone(), customers.clone()),
            |(o, cs)| black_box(plain(o, cs)),
        )
    });

    group.bench_function("oql", |b_| {
        b_.iter_with_setup(
            || (orders.clone(), customers.clone()),
            |(o, cs)| black_box(with_oql(o, cs)),
        )
    });

    group.finish();
}

criterion_group!(benches, bench_pipelines);
criterion_main!(benches);
