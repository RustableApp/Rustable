//! Bounded retained results from MySQL text/prepared query streams.
//! The transport still drains unretained rows to keep the connection synchronized;
//! cancellation and socket deadlines belong to the owning driver.
use super::{Grid, ScriptResult, Value, BYTE_LIMIT, QUERY_LIMIT};
use std::{sync::Arc, time::Instant};
use ::mysql::{QueryResult, Value as Wire, prelude::Protocol};

impl<T: Protocol> From<QueryResult<'_, '_, '_, T>> for ScriptResult {
    fn from(results: QueryResult<'_, '_, '_, T>) -> Self {
        collect(results, BYTE_LIMIT, QUERY_LIMIT)
    }
}

fn collect<T: Protocol>(mut results: QueryResult<'_, '_, '_, T>, budget: usize, row_limit: usize) -> ScriptResult {
    let started = Instant::now();
    let mut report = ScriptResult::default();
    let mut remaining = budget;
    while let Some(mut set) = results.iter() {
        // Include structural overhead: a script of empty result sets must also
        // have a bound. Do not misrepresent omitted results as a successful run.
        if report.grids.len() >= 128 {
            report.error.get_or_insert_with(|| "Only the first 128 result sets are retained.".into());
            for row in set.by_ref() { if let Err(error) = row { report.error = Some(error.to_string()); } }
            continue;
        }
        let columns = set.columns().as_ref().to_vec();
        let mut grid = Grid {
            columns: columns.iter().map(|c| c.name_str().into_owned()).collect(),
            affected: usize::try_from(set.affected_rows()).unwrap_or(usize::MAX),
            ..Default::default()
        };
        let header_bytes = grid.columns.iter().fold(std::mem::size_of::<Grid>(), |sum, name|
            sum.saturating_add(std::mem::size_of::<String>()).saturating_add(name.len()));
        if header_bytes > remaining {
            report.error.get_or_insert_with(|| "Result headers exceed the retained-result memory limit.".into());
            for row in set.by_ref() { if let Err(error) = row { report.error = Some(error.to_string()); } }
            continue;
        }
        remaining -= header_bytes;
        let set_started = Instant::now();
        for row in set.by_ref() {
            let row = match row {
                Ok(row) => row,
                Err(error) => { report.error = Some(error.to_string()); break; }
            };
            if grid.truncated { continue; }
            let row_bytes = (0..row.len()).fold(std::mem::size_of::<Vec<Value>>(), |sum, i| {
                sum.saturating_add(std::mem::size_of::<Value>()).saturating_add(match row.as_ref(i) {
                    Some(Wire::Bytes(bytes)) => bytes.len(),
                    // Numeric/temporal wire values may become owned display strings.
                    _ => 32,
                })
            });
            if grid.rows.len() >= row_limit || row_bytes > remaining {
                grid.truncated = true;
                continue;
            }
            let values = columns.iter().zip(row.unwrap()).map(Value::try_from)
                .collect::<Result<Vec<_>, _>>();
            match values {
                Ok(values) => { remaining -= row_bytes; grid.rows.push(values); }
                Err(error) => { report.error = Some(error); break; }
            }
        }
        grid.elapsed = set_started.elapsed();
        report.grids.push(Arc::new(grid));
    }
    report.elapsed = started.elapsed();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::mysql::{Conn, OptsBuilder, prelude::Queryable};

    #[test]
    #[ignore = "requires disposable MySQL/MariaDB fixture"]
    fn live_bounded_results_and_connection_reuse() {
        let port: u16 = std::env::var("RUSTABLE_MYSQL_TEST_PORT").unwrap().parse().unwrap();
        assert!([43316, 43317].contains(&port));
        let mut conn = Conn::new(OptsBuilder::default().ip_or_hostname(Some("127.0.0.1"))
            .tcp_port(port).user(Some("rustable")).pass(Some("rustable-test-only"))
            .db_name(Some("rustable_test")).prefer_socket(false)).unwrap();
        let report = ScriptResult::from(conn.query_iter("SELECT 1 AS empty_header WHERE FALSE; SELECT 9007199254740993 AS exact_value; SELECT NULL AS nullable").unwrap());
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.grids.len(), 3);
        assert_eq!(report.grids[0].columns, ["empty_header"]);
        assert!(report.grids[0].rows.is_empty());
        assert_eq!(report.grids[1].rows, [vec![Value::Integer(9007199254740993)]]);
        assert_eq!(report.grids[2].rows, [vec![Value::Null]]);
        let many = "WITH RECURSIVE n(x) AS (SELECT 0 UNION ALL SELECT x+1 FROM n WHERE x<99) SELECT a.x*100+b.x AS value FROM n a CROSS JOIN n b";
        let report = ScriptResult::from(conn.query_iter(many).unwrap());
        assert!(report.error.is_none());
        assert_eq!(report.grids[0].rows.len(), QUERY_LIMIT);
        assert!(report.grids[0].truncated);
        let report = collect(conn.query_iter("SELECT REPEAT('x', 4096) AS big; SELECT 2 AS following").unwrap(), 1024, 5000);
        assert!(report.grids[0].truncated);
        assert!(report.grids[0].rows.is_empty());
        assert_eq!(report.grids[1].rows, [vec![Value::Integer(2)]]);
        let report = ScriptResult::from(conn.query_iter("SELECT 1 AS retained; SELECT * FROM missing_rustable_result_table_93478").unwrap());
        assert_eq!(report.grids[0].rows, [vec![Value::Integer(1)]]);
        assert!(report.error.is_some());
        assert_eq!(conn.query_first::<u64, _>("SELECT 42").unwrap(), Some(42));
        let script = "SELECT 1;".repeat(130);
        let report = ScriptResult::from(conn.query_iter(script).unwrap());
        assert_eq!(report.grids.len(), 128);
        assert!(report.error.unwrap().contains("128"));
        assert_eq!(conn.query_first::<u64, _>("SELECT 43").unwrap(), Some(43));
    }
}
