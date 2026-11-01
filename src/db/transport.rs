//! Bounded binary messages preserve database values across browser workers.
use bincode::Options;
pub(super) fn codec() -> impl Options {
    bincode::DefaultOptions::new().with_fixint_encoding().with_limit(32 * 1024 * 1024)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Grid, Payload, ScriptResult, TransactionState, Value};
    use std::sync::Arc;

    #[test]
    fn worker_reply_preserves_typed_cells_and_transaction_state() {
        let payload = Payload::Query {
            report: ScriptResult {
                grids: vec![Arc::new(Grid {
                    columns: vec!["value".into()],
                    rows: vec![vec![Value::Integer(i64::MAX)], vec![Value::Blob(vec![0, 255])], vec![Value::Null], vec![Value::Text("日本\0text".into())], vec![Value::Real(f64::INFINITY)]],
                    ..Default::default()
                })],
                ..Default::default()
            },
            objects: None,
            schemas: Default::default(),
        };
        let scalar = Value::Integer(i64::MAX);
        assert_eq!(codec().deserialize::<Value>(&codec().serialize(&scalar).unwrap()).unwrap(), scalar);
        let Payload::Query { report, .. } = &payload else { unreachable!() };
        let _: Grid = codec().deserialize(&codec().serialize(report.grids[0].as_ref()).unwrap()).expect("grid roundtrip");
        let _: Payload = codec().deserialize(&codec().serialize(&payload).unwrap()).expect("payload roundtrip");
        let bytes = codec().serialize(&(Ok::<_, String>(payload), TransactionState::Active)).unwrap();
        let (reply, state): (Result<Payload, String>, TransactionState) = codec().deserialize(&bytes).unwrap();
        let Payload::Query { report, .. } = reply.unwrap() else { panic!("expected query"); };
        assert_eq!(state, TransactionState::Active);
        assert_eq!(report.grids[0].rows[0], [Value::Integer(i64::MAX)]);
        assert_eq!(report.grids[0].rows[1], [Value::Blob(vec![0, 255])]);
        assert_eq!(report.grids[0].rows[2], [Value::Null]);
        assert_eq!(report.grids[0].rows[3], [Value::Text("日本\0text".into())]);
        assert_eq!(report.grids[0].rows[4], [Value::Real(f64::INFINITY)]);
        assert!(codec().deserialize::<(Result<Payload, String>, TransactionState)>(&bytes[..bytes.len()-1]).is_err());
    }
    #[test]
    fn oversized_declared_buffer_is_rejected() {
        assert!(codec().deserialize::<Vec<u8>>(&u64::MAX.to_le_bytes()).is_err());
    }
}
