//! Compile-fail tests: verify that malformed queries produce readable
//! error messages with correct spans.
//!
//! Each `tests/ui/*.rs` file is expected to fail compilation. The first
//! run creates `.stderr` snapshot files alongside them; subsequent runs
//! compare against the snapshots. Regenerate with
//! `TRYBUILD=overwrite cargo test --test ui`.

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
