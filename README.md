# oql

Readable, declarative query syntax for Rust iterators. Compiles down to
a plain iterator chain, so the macro disappears in release builds.

```rust
use oql::oql;

let numbers = vec![1, 2, 3, 4, 5, 6];

let squared_evens: Vec<i32> = oql! {
    from x in numbers
    where x % 2 == 0
    select x * x
}
.collect();

assert_eq!(squared_evens, vec![4, 16, 36]);
```

## Why

Rust's iterator chains are powerful and fast, but once you stack several
`.filter().map().flat_map()` calls, the intent gets buried under
mechanics. `oql!` takes the same pipeline and writes it top-to-bottom
like a paragraph. The macro expands to a straightforward iterator chain
with no runtime, no reflection, and no dependencies in the surface
crate.

## Clauses

| Clause                                  | Meaning                                                |
|-----------------------------------------|--------------------------------------------------------|
| `from x in source`                      | Starts the query; expands to `source.into_iter()`, so any `IntoIterator` is accepted |
| `let name = expr`                       | Adds a per-element binding; `expr` is re-evaluated for each element and can reference the range variable and prior bindings |
| `where cond`                            | Filters elements                                       |
| `orderby key`                           | Sorts ascending                                        |
| `orderby key desc`                      | Sorts descending                                       |
| `join y in src on a == b`               | Inner equality join (hash-join under the hood)         |
| `join_must y in src on a == b`          | Inner equality join that panics if an outer row has no match |
| `join_left y in src on a == b`          | Left equality join; missing inner fields project as `None` |
| `last_must y in src on a == b`          | Keep only the last inner row per key and panic if none matches |
| `zip y in src`                          | Pair rows by position, stopping when either side ends |
| `zip_must y in src`                     | Pair rows by position; panics if the two sources have different lengths |
| `join y in src on a == b into g`        | Group-join: `g` is a `Vec<Y>` of matches (empty if none) |
| `group elem by key into g`              | Group elements by key; `g.key` and `g.items` downstream  |
| `select expr`                           | Projects to the output type                            |

`from` and `select` are mandatory. Everything else is optional and may
appear multiple times in any order. Clause order equals execution
order. This is deliberate, so the pipeline reads linearly and behaves
exactly like a hand-written iterator chain would.

`group by` is a pipeline barrier like `orderby`: every upstream element
must be seen before the first group can be yielded, and after the
clause the environment collapses to just the group binding. Later
clauses see whole groups, not individual elements. `g.items` is a plain
`Vec<T>`, so every `Iterator` method (`sum`, `count`, `max`, `fold`, and
so on) is directly available.

The macro returns an `Iterator`. The caller decides whether to
`.collect()`, `.sum()`, `.take(10).collect()`, `.for_each(...)`, and so
on. This is why there is no `limit`, `offset`, `count`, or `distinct`
clause: Rust's `Iterator` trait already provides every terminal and
adapter operation you might want. The same applies to `union`,
`intersect`, and `except`. These belong in your follow-up code (for
example via `HashSet::union`).

## Examples

A filter plus projection:

```rust
use oql::oql;

let xs = vec![1, 2, 3, 4, 5, 6];
let out: Vec<i32> = oql! {
    from x in xs
    where x % 2 == 0
    select x * x
}
.collect();
assert_eq!(out, vec![4, 16, 36]);
```

Intermediate bindings:

```rust
use oql::oql;

struct Order { price: f64, quantity: u32, discount: f64, customer: String }

let orders: Vec<Order> = /* ... */;

let high_value: Vec<(String, f64)> = oql! {
    from o in orders
    let total = o.price * o.quantity as f64
    let net = total * (1.0 - o.discount)
    where net > 100.0
    orderby net desc
    select (o.customer, net)
}
.collect();
```

Inner join:

```rust
use oql::oql;

#[derive(Clone)]
struct Order { id: u32, customer_id: u32, amount: f64 }
#[derive(Clone)]
struct Customer { id: u32, name: &'static str, country: &'static str }

let orders: Vec<Order> = /* ... */;
let customers: Vec<Customer> = /* ... */;

let german_sales: Vec<(u32, &'static str, f64)> = oql! {
    from o in orders
    join c in customers on o.customer_id == c.id
    where c.country == "DE"
    orderby o.amount desc
    select (o.id, c.name, o.amount)
}
.collect();
```

Composite sort keys:

```rust
use oql::oql;

let items = vec![(1, 10), (2, 5), (1, 30), (2, 20), (1, 20)];
let out: Vec<(i32, i32)> = oql! {
    from pair in items
    let g = pair.0
    let v = pair.1
    orderby g         // primary, ascending
    orderby v desc    // secondary, descending
    select (g, v)
}
.collect();
assert_eq!(out, vec![(1, 30), (1, 20), (1, 10), (2, 20), (2, 5)]);
```

Group by with aggregation:

```rust
use oql::oql;

#[derive(Clone)]
struct Order { customer: &'static str, amount: u64 }

let orders: Vec<Order> = /* ... */;

// Revenue per customer.
let per_customer: Vec<(&'static str, u64)> = oql! {
    from o in orders
    group o by o.customer into g
    select (g.key, g.items.iter().map(|o| o.amount).sum::<u64>())
}
.collect();
```

Group-join:

```rust
use oql::oql;

#[derive(Clone)]
struct Customer { id: u32, name: &'static str }
#[derive(Clone)]
struct Order { customer_id: u32, amount: u64 }

let customers: Vec<Customer> = /* ... */;
let orders: Vec<Order> = /* ... */;

// Each customer with their full list of orders (possibly empty).
let report: Vec<(&'static str, Vec<Order>)> = oql! {
    from c in customers
    join o in orders on c.id == o.customer_id into o_group
    select (c.name, o_group)
}
.collect();
```

## How the expansion works

For every `oql!` invocation, the macro walks the clauses in order and
emits a plain iterator chain. Each clause maps to a familiar adapter:

- `where` becomes `.filter_map(|env| if cond { Some(env) } else { None })`.
  The macro uses `filter_map` rather than `filter` because new bindings
  introduced by `let` may change the shape of the environment tuple
  between steps.
- `let name = expr` adds `name` to the environment tuple so it is
  available in every subsequent clause (until it is dropped by
  liveness analysis or a `group by` resets the environment). For
  efficiency the computation is fused into the following step's
  closure instead of getting its own adapter, and multiple
  consecutive `let`s are baked into a single `.map()` closure
  together, not one adapter per binding.
- `orderby key` collects into a `Vec<(key, env)>`, sorts in place, then
  yields back as an iterator. Multiple consecutive `orderby` clauses
  merge into one sort with a composite key tuple. The key is cloned
  once per element (via `(&expr).clone()`). `Copy` types copy for free,
  `String`-like types clone exactly once, which is the minimum any
  stable sort needs.
- `join y in src on a == b` builds a `HashMap<K, Vec<T>>` from the
  inner source in a preamble, then a `.flat_map(...)` on the outer
  iterator probes it. `O(n + m)` instead of the naive `O(n · m)`
  nested loop. `join_must` uses the same expansion, but panics instead
  of dropping an outer row when the lookup has no match. `join_left`
  keeps unmatched outer rows and projects joined fields as `Option`s.
  `last_must` stores only the last inner row for each key, so it avoids
  allocating per-key match vectors when only the final match is needed.
- `zip y in src` pairs each current row with the next row from `src`,
  matching Rust's standard `Iterator::zip` by stopping when either side
  ends. Use `zip_must y in src` when the two sources must have the same
  length; it panics if either side ends first.
- `select` becomes `.map(|env| projection)`.

Between steps, the macro propagates an *environment tuple* of every
binding that will still be needed. A backwards live-variable analysis
(see `oql-macro/src/liveness.rs`) prunes bindings that no later clause
reads, so the tuple stays as narrow as the data actually demands. Dead
`let` bindings keep their value expression (for side-effect parity) but
don't enter the outgoing tuple.

You can inspect the expansion yourself:

```bash
cargo expand -p oql --example simple_compare
cargo expand -p oql --bench throughput
```

The `simple_compare` example contains a handwritten and an `oql!`
version of the same tiny pipeline side by side.

## Performance

The macro is a syntax rewriter, not a runtime. It inherits every
iterator-chain optimisation LLVM and the Rust optimizer already apply:
closures and environment tuples are inlined away in release, and the
compiled code is measured against a handwritten equivalent.

For simple queries (`from / where / let / select`), the generated code
is indistinguishable from a hand-written `.filter().map()` chain, within
measurement noise.

For a join-and-sort query with 10 000 orders × 1 000 customers
(`benches/throughput.rs`, `cargo bench`):

| Variant                                            | Time     |
|----------------------------------------------------|----------|
| Handwritten: hash-join + filter + collect + sort   | ≈ 755 µs |
| `oql!` expansion (equivalent semantics)            | ≈ 800 µs |

That is roughly **1.07× the cost of the handwritten pipeline** (median
of 10 runs, stddev 0.02). Most of the remaining gap comes from the
macro's slightly wider intermediate tuple during the sort step. The
handwritten version inlines the final projection into the `flat_map`
body, which lets it allocate only `(String, u64)` pairs by the time
the sort runs. The macro keeps the projection in its own `.map()`
stage so `.take(n)` after the query short-circuits correctly through
the sort, which means the sort payload is `(Customer, u64)` instead of
`(String, u64)`.

Where the handwritten version fuses a `where` after a `join` into the
same `flat_map` body, so does the macro. The safety check in
`liveness::bare_idents` makes sure no captures would silently be
pulled into the `move` closure as a side effect.

Measurements vary with system load. On a warm VM the ratio is stable
between 1.03× and 1.10×, and absolute numbers will differ on your
machine. Run `scripts/multi_bench.sh 10` to collect your own data.
