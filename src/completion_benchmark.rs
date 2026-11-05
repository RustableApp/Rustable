//! Opt-in completion CPU checks. No database or language-server requests.
use super::*;

#[test]
#[ignore = "local release-mode completion latency benchmark"]
fn completion_catalog_latency_benchmark() {
    for tables in [200, 2_000] {
        let objects: Vec<_> = (0..tables).map(|index| Object {
            name: format!("customers_{index:04}"), kind: "table".into(),
            columns: (0..24).map(|column| Column {
                name: if column == 0 { "id".into() } else { format!("column_{column:02}") },
                data_type: "TEXT".into(), not_null: false, default: String::new(), pk: column == 0, generated: false
            }).collect(), schema: None, display_name: None, metadata_error: None, ddl: String::new()
        }).collect();
        for (scenario, sql, cursor) in [
            ("scoped", "SELECT c.co FROM customers_0001 c", 11),
            ("table", "SELECT * FROM customer", 22),
            ("inferred-prefix", "SELECT co", 9),
            ("inferred-empty", "SELECT id, ", 11),
        ] {
            let mut completion = Completion::default();
            let mut samples = Vec::new();
            for index in 0..120 {
                let start = std::time::Instant::now();
                completion.refresh(std::hint::black_box(sql), cursor, &objects, &[], false, Dialect::Postgres);
                std::hint::black_box(&completion.items);
                if index >= 20 { samples.push(start.elapsed().as_secs_f64() * 1000.0); }
            }
            samples.sort_by(f64::total_cmp);
            crate::ui_benchmark::report(&format!("completion/{tables}/{scenario}"), &samples);
        }
    }
}
