//! Benchmarks for the time-expr pipeline: parse, eval, display, end-to-end.
//!
//! Run with: `cargo bench -p time-expr`

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use time_expr::ast::Env;
use time_expr::parser::parse;
use time_expr::value::{Duration, TimePoint, Value};

// =========================================================================
// Test expressions, ordered by complexity
// =========================================================================

const EXPR_SIMPLE_DURATION: &str = "2h15m";
const EXPR_SIMPLE_TIMEPOINT: &str = "14:30";
const EXPR_SCALAR_MUL: &str = "3h * 2.5";
const EXPR_TWO_SUBS: &str = "(17:48 - 10:10) + (16:41 - 08:19)";
const EXPR_CHAIN_5: &str = "1h + 2h + 3h + 4h + 5h";
const EXPR_MIXED_OPS: &str = "(23:00 + 3h) - 1:00:00:00 + 30m * 2";
const EXPR_NESTED_PARENS: &str = "((((1h + 2h) * 3) - 4h) / 2)";

/// Build a deeply chained expression: `1h + 2h + 3h + ... + Nh`
fn chain_expr(n: usize) -> String {
    (1..=n)
        .map(|i| format!("{i}h"))
        .collect::<Vec<_>>()
        .join(" + ")
}

// =========================================================================
// Parse benchmarks
// =========================================================================

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");

    for (name, expr) in [
        ("simple_dur", EXPR_SIMPLE_DURATION),
        ("simple_tp", EXPR_SIMPLE_TIMEPOINT),
        ("scalar_mul", EXPR_SCALAR_MUL),
        ("two_subs", EXPR_TWO_SUBS),
        ("chain_5", EXPR_CHAIN_5),
        ("mixed_ops", EXPR_MIXED_OPS),
        ("nested_parens", EXPR_NESTED_PARENS),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), expr, |b, src| {
            b.iter(|| parse(black_box(src)).unwrap());
        });
    }

    for n in [5, 10, 20, 50] {
        let expr = chain_expr(n);
        group.bench_with_input(BenchmarkId::new("chain", n), &expr, |b, src| {
            b.iter(|| parse(black_box(src)).unwrap());
        });
    }

    group.finish();
}

// =========================================================================
// Eval benchmarks — tree walk (pre-parsed)
// =========================================================================

fn bench_eval(c: &mut Criterion) {
    let mut group = c.benchmark_group("eval");
    let env = Env::new();

    for (name, expr_str) in [
        ("simple_dur", EXPR_SIMPLE_DURATION),
        ("two_subs", EXPR_TWO_SUBS),
        ("chain_5", EXPR_CHAIN_5),
        ("mixed_ops", EXPR_MIXED_OPS),
        ("nested_parens", EXPR_NESTED_PARENS),
    ] {
        let tree = parse(expr_str).unwrap();
        group.bench_with_input(BenchmarkId::from_parameter(name), &tree, |b, tree| {
            b.iter(|| black_box(tree).eval(&env).unwrap());
        });
    }

    for n in [5, 10, 20, 50] {
        let expr = chain_expr(n);
        let tree = parse(&expr).unwrap();
        group.bench_with_input(BenchmarkId::new("chain", n), &tree, |b, tree| {
            b.iter(|| black_box(tree).eval(&env).unwrap());
        });
    }

    group.finish();
}

// =========================================================================
// Display benchmarks — Value formatting
// =========================================================================

fn bench_display(c: &mut Criterion) {
    let mut group = c.benchmark_group("display");

    let cases: Vec<(&str, Value)> = vec![
        ("scalar", Value::Scalar(42.5)),
        ("dur_simple", Value::Duration(Duration::from_dhms(0, 2, 15, 0))),
        ("dur_full", Value::Duration(Duration::from_dhms(1, 2, 3, 4))),
        ("dur_negative", Value::Duration(Duration::from_secs(-(2 * 3600 + 30 * 60)))),
        ("dur_zero", Value::Duration(Duration::from_secs(0))),
        ("tp_hhmm", Value::TimePoint(TimePoint::from_dhms_strict(0, 14, 30, 0).unwrap())),
        ("tp_hhmmss", Value::TimePoint(TimePoint::from_dhms_strict(0, 14, 30, 15).unwrap())),
        ("tp_ddhhmmss", Value::TimePoint(TimePoint::from_dhms_strict(1, 14, 30, 0).unwrap())),
    ];

    for (name, val) in &cases {
        group.bench_with_input(BenchmarkId::from_parameter(name), val, |b, val| {
            b.iter(|| black_box(val).to_string());
        });
    }

    group.finish();
}

// =========================================================================
// End-to-end: parse + eval + display (the typical CLI path)
// =========================================================================

fn bench_end_to_end(c: &mut Criterion) {
    let mut group = c.benchmark_group("end_to_end");
    let env = Env::new();

    for (name, expr) in [
        ("simple_dur", EXPR_SIMPLE_DURATION),
        ("two_subs", EXPR_TWO_SUBS),
        ("chain_5", EXPR_CHAIN_5),
        ("mixed_ops", EXPR_MIXED_OPS),
    ] {
        group.bench_with_input(BenchmarkId::from_parameter(name), expr, |b, src| {
            b.iter(|| {
                let tree = parse(black_box(src)).unwrap();
                let val = tree.eval(&env).unwrap();
                val.to_string()
            });
        });
    }

    for n in [10, 20, 50] {
        let expr = chain_expr(n);
        group.bench_with_input(BenchmarkId::new("chain", n), &expr, |b, src| {
            b.iter(|| {
                let tree = parse(black_box(src)).unwrap();
                let val = tree.eval(&env).unwrap();
                val.to_string()
            });
        });
    }

    group.finish();
}

// =========================================================================

criterion_group!(benches, bench_parse, bench_eval, bench_display, bench_end_to_end);
criterion_main!(benches);
