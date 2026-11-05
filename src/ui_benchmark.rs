//! Machine-readable output for opt-in CPU rendering benchmarks, not presentation timing.
pub(crate) fn report(name: &str, sorted_ms: &[f64]) {
    assert!(!sorted_ms.is_empty());
    assert!(sorted_ms
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0));
    let percentile = |fraction: f64| {
        sorted_ms[((sorted_ms.len() as f64 * fraction) as usize).min(sorted_ms.len() - 1)]
    };
    println!(
        "RUSTABLE_UI_BENCH {}",
        serde_json::json!({
            "name": name, "unit": "ms", "measurement": "cpu", "samples": sorted_ms.len(),
            "p50": percentile(0.50), "p95": percentile(0.95), "p99": percentile(0.99),
            "max": sorted_ms[sorted_ms.len()-1], "sorted_samples_ms": sorted_ms,
        })
    );
}
