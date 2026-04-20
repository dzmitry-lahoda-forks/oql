#!/usr/bin/env bash
# Runs the throughput benchmark N times. For each run, extract the median
# µs reported by criterion for both plain and oql. Print as tab-separated
# rows: run_number, plain_median_us, oql_median_us.
#
# Criterion prints times as either "XXX.YY µs" or "X.YYYY ms". We
# normalise both to µs here so the downstream stats script can treat
# everything as a single number type.

set -euo pipefail

N="${1:-10}"
cd "$(dirname "$0")"

# The first run on a cold VM measures noisier than the rest: the bench
# binary hasn't been paged in, the CPU governor hasn't ramped up, the
# glibc heap is fresh, and Criterion's own internal calibration costs
# more before its target function has been inlined into cache. Nothing
# JIT-related (Rust is AOT); it's ordinary cold-start noise. We do one
# warmup run and throw its output away so the N reported runs all
# measure in the same steady state.
echo "# warming up..." >&2
cargo bench --bench throughput >/dev/null 2>&1 || true

parse_time_us() {
    # $1: something like "[845.12 µs 858.68 µs 872.73 µs]" or
    #     "[1.0003 ms 1.0028 ms 1.0061 ms]". We want the middle
    # number (the median). If units are ms, multiply by 1000.
    local line="$1"
    local median unit
    # Take the second number + its unit.
    median=$(echo "$line" | awk -F'[][]' '{print $2}' | awk '{print $3}')
    unit=$(echo "$line" | awk -F'[][]' '{print $2}' | awk '{print $4}')
    if [[ "$unit" == "ms" ]]; then
        awk -v x="$median" 'BEGIN{printf "%.2f", x * 1000}'
    else
        awk -v x="$median" 'BEGIN{printf "%.2f", x}'
    fi
}

echo -e "run\tplain_us\toql_us"
for i in $(seq 1 "$N"); do
    out=$(cargo bench --bench throughput 2>&1 | grep -E "^top_eu_revenue/(plain|oql) *time:")
    plain_line=$(echo "$out" | grep "/plain" || true)
    oql_line=$(echo "$out" | grep "/oql" || true)
    plain_us=$(parse_time_us "$plain_line")
    oql_us=$(parse_time_us "$oql_line")
    echo -e "${i}\t${plain_us}\t${oql_us}"
done
