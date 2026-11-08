//! Bounded Search catalog metadata; one pipeline per requested page.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Metadata {
    pub prefixes: String,
    pub types: String,
    pub documents: Option<u64>,
    pub fields: usize,
}
pub(crate) const PAGE_SIZE: usize = 50;
pub(crate) type Rows = Vec<(Vec<u8>, Result<Metadata, String>)>;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn fetch(connection: &mut redis::Connection, names: &[Vec<u8>]) -> Result<Rows, String> {
    if names.is_empty() || names.len() > PAGE_SIZE || names.iter().any(|name| name.len() > 4096) {
        return Err("Request 1–50 index names, each at most 4 KiB.".into());
    }
    let mut pipeline = redis::pipe();
    for name in names { pipeline.cmd("FT.INFO").arg(name); }
    let replies = connection.req_packed_commands_bounded(&pipeline.get_packed_pipeline(), &vec![1024 * 1024; names.len()])
        .map_err(|error| format!("Index metadata connection closed: {error}. Refresh after reconnecting."))?;
    Ok(names.iter().cloned().zip(replies.into_iter().map(|reply| {
        reply.extract_error().map_err(|error| error.to_string().chars().take(1024).collect()).and_then(decode)
    })).collect())
}

#[cfg(not(target_arch = "wasm32"))]
fn decode(value: redis::Value) -> Result<Metadata, String> {
    use redis::Value;
    fn member<'a>(values: &'a [Value], name: &[u8]) -> Option<&'a Value> {
        values.as_chunks::<2>().0.iter().find(|pair| matches!(&pair[0], Value::BulkString(key) if key == name)
            || matches!(&pair[0], Value::SimpleString(key) if key.as_bytes() == name)).map(|pair| &pair[1])
    }
    let Value::Array(values) = value else { return Err("Unexpected index metadata.".into()); };
    if values.len() % 2 != 0 { return Err("Incomplete index metadata.".into()); }
    let documents = member(&values, b"num_docs").and_then(|value| match value {
        Value::Int(n) => u64::try_from(*n).ok(),
        Value::BulkString(bytes) => std::str::from_utf8(bytes).ok()?.parse().ok(),
        Value::SimpleString(text) => text.parse().ok(),
        _ => None,
    });
    let prefixes = match member(&values, b"index_definition") {
        Some(Value::Array(definition)) => match member(definition, b"prefixes") {
            Some(Value::Array(prefixes)) => prefixes.iter().take(16).map(crate::redis_value::text)
                .map(|text| text.chars().take(128).collect::<String>()).collect::<Vec<_>>().join(", "),
            _ => String::new(),
        },
        _ => String::new(),
    };
    let Some(Value::Array(attributes)) = member(&values, b"attributes") else { return Err("Missing index fields.".into()); };
    if attributes.len() > 1000 { return Err("Index exceeds 1,000 fields.".into()); }
    let mut types = std::collections::BTreeSet::new();
    for attribute in attributes {
        if let Value::Array(parts) = attribute {
            // SORTABLE/NOINDEX are standalone flags, so field records aren't
            // always an even key/value list.
            if let Some(pair) = parts.windows(2).find(|pair| crate::redis_value::text(&pair[0]) == "type") {
                types.insert(crate::redis_value::text(&pair[1]).chars().take(32).collect::<String>());
            }
        }
    }
    Ok(Metadata { prefixes, types: types.into_iter().collect::<Vec<_>>().join(", "), documents, fields: attributes.len() })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    fn text(s: &str) -> redis::Value { redis::Value::BulkString(s.as_bytes().to_vec()) }
    #[test]
    #[ignore = "disposable Redis Search fixture via scripts/test-redis-search-catalog.sh"]
    fn live_catalog_metadata_and_missing_index() {
        assert_eq!(std::env::var("RUSTABLE_REDIS_SEARCH_CATALOG_QA").as_deref(), Ok("1"));
        let settings = crate::redis_settings::Settings {
            host: "127.0.0.1".into(), port: std::env::var("RUSTABLE_REDIS_SEARCH_CATALOG_PORT").unwrap().parse().unwrap(),
            tls: false, ..Default::default()
        };
        let mut connection = crate::redis_session::Session::raw_node(&settings, "rustable-search-test-only").unwrap();
        redis::cmd("FT.CREATE").arg("idx:catalog-proof").arg("ON").arg("HASH").arg("PREFIX").arg(1).arg("catalog:")
            .arg("SCHEMA").arg("name").arg("TEXT").arg("SORTABLE").arg("rank").arg("NUMERIC")
            .query::<()>(&mut connection).unwrap();
        redis::cmd("HSET").arg("catalog:one").arg("name").arg("Avery").arg("rank").arg(1).query::<i64>(&mut connection).unwrap();
        let mut session = crate::redis_session::Session::connect(&settings, "rustable-search-test-only").unwrap();
        let crate::redis_search::Reply::Metadata(rows) = session.search(crate::redis_search::Request::Metadata(vec![b"idx:catalog-proof".to_vec(), b"idx:missing".to_vec()])).unwrap() else { panic!() };
        let info = rows[0].1.as_ref().unwrap();
        assert_eq!(info.prefixes, "catalog:");
        assert_eq!(info.types, "NUMERIC, TEXT");
        assert_eq!(info.fields, 2);
        assert!(info.documents.is_some());
        assert!(rows[1].1.is_err());
        assert_eq!(session.execute("PING", true).unwrap(), vec![redis::Value::SimpleString("PONG".into())]);
        use crate::redis_search_manage::{Create, Field, Kind, Mutation};
        for json in [false, true] {
            let name = if json { "idx:managed-json" } else { "idx:managed-hash" };
            let create = Mutation::Create(Create { name: name.into(), json, prefix: "catalog:".into(), fields: vec![Field {
                path: if json { "$.name" } else { "name" }.into(), alias: "name".into(), kind: Kind::Text, sortable: true,
            }] });
            assert!(session.search_mutate(create.clone(), true).is_err());
            assert!(session.search_mutate(create.clone(), false).is_ok());
            assert!(session.search_mutate(create, false).is_err(), "duplicate index must fail without replacing it");
            let drop = Mutation::Drop(name.as_bytes().to_vec());
            assert!(session.search_mutate(drop.clone(), true).is_err());
            assert!(session.search_mutate(drop, false).is_ok());
        }
        assert_eq!(redis::cmd("EXISTS").arg("catalog:one").query::<i64>(&mut connection).unwrap(), 1, "dropping indexes must preserve documents");
    }

    #[test]
    fn metadata_pipeline_sends_whole_page_and_preserves_per_index_errors() {
        use std::{io::Write, net::TcpListener, time::Duration};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut parser = redis::Parser::new();
            let read = |socket: &mut std::net::TcpStream, parser: &mut redis::Parser| -> Vec<Vec<u8>> {
                redis::from_redis_value(parser.parse_value(socket).unwrap()).unwrap()
            };
            loop {
                let command = read(&mut socket, &mut parser);
                if command[0] == b"FT.INFO" { assert_eq!(command[1], b"present"); break; }
                assert_eq!(command[0], b"CLIENT");
                socket.write_all(b"+OK\r\n").unwrap();
            }
            // Every command in the full 50-index page must arrive before any reply.
            assert_eq!(read(&mut socket, &mut parser), vec![b"FT.INFO".to_vec(), b"gone".to_vec()]);
            for _ in 2..PAGE_SIZE { assert_eq!(read(&mut socket, &mut parser), vec![b"FT.INFO".to_vec(), b"present".to_vec()]); }
            std::thread::sleep(Duration::from_millis(80));
            socket.write_all(b"*4\r\n$8\r\nnum_docs\r\n:3\r\n$10\r\nattributes\r\n*0\r\n-ERR Unknown index name\r\n").unwrap();
            for _ in 2..PAGE_SIZE { socket.write_all(b"*4\r\n$8\r\nnum_docs\r\n:3\r\n$10\r\nattributes\r\n*0\r\n").unwrap(); }
            assert_eq!(read(&mut socket, &mut parser)[0], b"PING");
            socket.write_all(b"+PONG\r\n").unwrap();
        });
        let settings = crate::redis_settings::Settings { host: "127.0.0.1".into(), port, tls: false, ..Default::default() };
        let mut connection = crate::redis_session::Session::raw_node(&settings, "").unwrap();
        let started = std::time::Instant::now();
        let mut names = vec![b"present".to_vec(); PAGE_SIZE]; names[1] = b"gone".to_vec();
        let rows = fetch(&mut connection, &names).unwrap();
        assert_eq!(rows.len(), PAGE_SIZE);
        assert!(rows[2..].iter().all(|row| row.1.is_ok()));
        assert!(connection.req_packed_commands_bounded(b"invalid-not-sent", &[16 * 1024 * 1024; 5]).is_err());
        let elapsed = started.elapsed();
        eprintln!("catalog metadata page with 80 ms simulated response delay: {elapsed:?}");
        assert!(elapsed >= Duration::from_millis(80) && elapsed < Duration::from_millis(250));
        assert_eq!(rows[0].1.as_ref().unwrap().documents, Some(3));
        assert!(rows[1].1.as_ref().unwrap_err().contains("Unknown index"));
        assert!(fetch(&mut connection, &vec![b"too-many".to_vec(); PAGE_SIZE + 1]).is_err());
        assert_eq!(redis::cmd("PING").query::<String>(&mut connection).unwrap(), "PONG");
        peer.join().unwrap();
    }

    #[test]
    fn metadata_accepts_numeric_counts_and_flagged_fields() {
        use redis::Value::{Array, Int};
        let value = Array(vec![text("num_docs"), text("42"), text("index_definition"), Array(vec![text("prefixes"), Array(vec![text("app:users:")])]), text("attributes"), Array(vec![Array(vec![text("identifier"),text("name"),text("attribute"),text("name"),text("type"),text("TEXT"),text("SORTABLE")]), Array(vec![text("type"),text("TAG")])])]);
        let metadata = decode(value).unwrap();
        assert_eq!(metadata.documents, Some(42));
        assert_eq!(metadata.fields, 2);
        assert_eq!(metadata.types, "TAG, TEXT");
        assert_eq!(metadata.prefixes, "app:users:");
        assert!(decode(Array(vec![text("attributes")])).is_err());
        assert!(decode(Array(vec![text("num_docs"), Int(1)])).is_err());
    }
}
