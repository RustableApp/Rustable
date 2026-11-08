use super::*;
use std::io::Write;
use std::net::TcpListener;
use std::time::{Duration, Instant};

#[test]
fn scan_and_preview_protocol_has_one_then_two_exchanges() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut parser = redis::Parser::new();
        let read = |parser: &mut redis::Parser, stream: &mut std::net::TcpStream| -> Vec<Vec<u8>> {
            redis::from_redis_value(parser.parse_value(stream).unwrap()).unwrap()
        };
        loop {
            let command = read(&mut parser, &mut stream);
            match command[0].as_slice() {
                b"CLIENT" | b"AUTH" => stream.write_all(b"+OK\r\n").unwrap(),
                b"PING" => stream.write_all(b"+PONG\r\n").unwrap(),
                b"SELECT" => {
                    stream.write_all(b"+OK\r\n").unwrap();
                    break;
                }
                other => panic!("unexpected setup command: {other:?}"),
            }
        }
        // Both first and repeated use: no health checks, describes or cleanup.
        for _ in 0..2 {
            let scan = read(&mut parser, &mut stream);
            assert_eq!(
                scan,
                [
                    b"SCAN".to_vec(),
                    b"0".to_vec(),
                    b"MATCH".to_vec(),
                    b"app:*".to_vec(),
                    b"COUNT".to_vec(),
                    b"200".to_vec(),
                    b"TYPE".to_vec(),
                    b"string".to_vec()
                ]
            );
            stream
                .write_all(b"*2\r\n$1\r\n0\r\n*2\r\n$5\r\napp:a\r\n$5\r\napp:b\r\n")
                .unwrap();
            // Withhold all responses until the entire batch arrives. Serial
            // per-key requests would time out instead of passing this test.
            for name in [b"TYPE".as_slice(), b"PTTL", b"TYPE", b"PTTL"] {
                assert_eq!(read(&mut parser, &mut stream)[0], name);
            }
            stream
                .write_all(b"+string\r\n:-1\r\n+string\r\n:5000\r\n")
                .unwrap();
            for _ in 0..2 {
                assert_eq!(read(&mut parser, &mut stream)[0], b"GETRANGE");
            }
            stream.write_all(b"$1\r\na\r\n$1\r\nb\r\n").unwrap();
        }
        assert_eq!(read(&mut parser, &mut stream)[0], b"SCAN");
        stream.write_all(b"-ERR syntax error\r\n").unwrap();
        assert_eq!(read(&mut parser, &mut stream)[0], b"PING");
        stream.write_all(b"+PONG\r\n").unwrap();
    });
    let mut session = Session::connect(
        &Settings {
            host: "127.0.0.1".into(),
            port,
            tls: false,
            ..Default::default()
        },
        "",
    )
    .unwrap();
    for _ in 0..2 {
        let page = session.scan_filtered(0, b"app:*", 200, "string").unwrap();
        assert_eq!(page.keys.len(), 2);
        let rows = session.key_rows(&page.keys).unwrap();
        assert_eq!(rows[0].value, redis::Value::BulkString(b"a".to_vec()));
        assert_eq!(rows[1].ttl_ms, 5000);
    }
    assert!(session.scan_filtered(0, b"*", 200, "string").is_err());
    assert!(session.is_connected());
    session.query(&redis::cmd("PING")).unwrap();
    drop(session);
    peer.join().unwrap();
}

#[test]
#[ignore = "requires disposable Redis on localhost:16379"]
fn live_filtered_scan_and_delayed_browser_latency() {
    use std::io::Read;
    use std::net::TcpStream;
    let config = Settings {
        host: "127.0.0.1".into(),
        port: 16379,
        tls: false,
        ..Default::default()
    };
    let mut admin = Session::connect(&config, "rustable-redis-test-only").unwrap();
    let prefix = format!("rustable-browser-{}:", std::process::id());
    for i in 0..40 {
        admin
            .query(redis::cmd("SET").arg(format!("{prefix}{i}")).arg("preview"))
            .unwrap();
    }
    let hash = format!("{prefix}hash");
    admin
        .query(redis::cmd("HSET").arg(&hash).arg("field").arg("value"))
        .unwrap();
    let mut cursor = 0;
    let mut found = Vec::new();
    loop {
        let page = admin
            .scan_filtered(cursor, format!("{prefix}*").as_bytes(), 1, "hash")
            .unwrap();
        found.extend(page.keys);
        cursor = page.cursor;
        if cursor == 0 {
            break;
        }
    }
    found.sort();
    found.dedup();
    assert_eq!(found, vec![hash.as_bytes().to_vec()]);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let proxy = std::thread::spawn(move || {
        let (downstream, _) = listener.accept().unwrap();
        let upstream = TcpStream::connect("127.0.0.1:16379").unwrap();
        upstream.set_nodelay(true).unwrap();
        downstream.set_nodelay(true).unwrap();
        fn forward(mut source: TcpStream, mut target: TcpStream) {
            let shutdown = source.try_clone().unwrap();
            let (send, receive) = std::sync::mpsc::sync_channel(64);
            let reader = std::thread::spawn(move || {
                let mut bytes = [0u8; 65536];
                while let Ok(length) = source.read(&mut bytes) {
                    if length == 0 {
                        break;
                    }
                    let deadline = Instant::now() + Duration::from_millis(15);
                    if send.send((deadline, bytes[..length].to_vec())).is_err() {
                        break;
                    }
                }
            });
            while let Ok((deadline, bytes)) = receive.recv() {
                std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                if target.write_all(&bytes).is_err() {
                    break;
                }
            }
            drop(receive);
            let _ = shutdown.shutdown(std::net::Shutdown::Both);
            let _ = target.shutdown(std::net::Shutdown::Both);
            reader.join().unwrap();
        }
        let reverse_source = upstream.try_clone().unwrap();
        let reverse_target = downstream.try_clone().unwrap();
        let reverse = std::thread::spawn(move || forward(reverse_source, reverse_target));
        forward(downstream, upstream);
        reverse.join().unwrap();
    });
    let mut session =
        Session::connect(&Settings { port, ..config }, "rustable-redis-test-only").unwrap();
    let mut first = Vec::new();
    let mut complete = Vec::new();
    for _ in 0..12 {
        let start = Instant::now();
        let page = session
            .scan_filtered(0, format!("{prefix}*").as_bytes(), 1000, "string")
            .unwrap();
        assert_eq!(page.cursor, 0);
        assert_eq!(page.keys.len(), 40);
        let names_ms = start.elapsed().as_secs_f64() * 1000.0;
        let rows = session.key_rows(&page.keys).unwrap();
        assert_eq!(rows.len(), 40);
        assert!(
            rows.iter()
                .all(|r| r.value == redis::Value::BulkString(b"preview".to_vec()))
        );
        first.push(names_ms);
        complete.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    eprintln!(
        "Redis 30ms added RTT: first-use names={:.2}ms complete={:.2}ms",
        first[0], complete[0]
    );
    first.sort_by(f64::total_cmp);
    complete.sort_by(f64::total_cmp);
    eprintln!(
        "Redis 30ms added RTT, 12 samples: progressive names p50={:.2} p95={:.2}ms; old publication point/all previews p50={:.2} p95={:.2}ms",
        first[6], first[11], complete[6], complete[11]
    );
    assert!(first[6] < complete[6] * 0.65);
    let key = format!("{prefix}0").into_bytes();
    let mut legacy = Vec::new();
    let mut current = Vec::new();
    let mut paging = Vec::new();
    for iteration in 0..12 {
        for old in if iteration % 2 == 0 {
            [true, false]
        } else {
            [false, true]
        } {
            let start = Instant::now();
            if old {
                let kind: String = session.typed(redis::cmd("TYPE").arg(&key)).unwrap();
                let ttl: i64 = session.typed(redis::cmd("PTTL").arg(&key)).unwrap();
                let len: u64 = session.typed(redis::cmd("STRLEN").arg(&key)).unwrap();
                let value = session
                    .query(redis::cmd("GETRANGE").arg(&key).arg(0).arg(65535))
                    .unwrap();
                assert_eq!((kind.as_str(), ttl, len), ("string", -1, 7));
                assert_eq!(value, redis::Value::BulkString(b"preview".to_vec()));
                legacy.push(start.elapsed().as_secs_f64() * 1000.0);
            } else {
                let view = session.inspect_view(&key).unwrap();
                assert_eq!(
                    view.snapshot.value,
                    redis::Value::BulkString(b"preview".to_vec())
                );
                current.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        let start = Instant::now();
        let view = session
            .value_page(&crate::redis_value::PageRequest {
                key: key.clone(),
                kind: "string".into(),
                cursor: crate::redis_value::Cursor::Offset(0),
                pattern: b"*".to_vec(),
            })
            .unwrap();
        assert_eq!(
            view.snapshot.value,
            redis::Value::BulkString(b"preview".to_vec())
        );
        paging.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    eprintln!(
        "Inspector first-use legacy={:.2}ms current={:.2}ms page={:.2}ms",
        legacy[0], current[0], paging[0]
    );
    legacy.sort_by(f64::total_cmp);
    current.sort_by(f64::total_cmp);
    paging.sort_by(f64::total_cmp);
    eprintln!(
        "Inspector 30ms added RTT, 12 alternating pairs: legacy p50={:.2} p95={:.2}ms; current p50={:.2} p95={:.2}ms; subsequent page p50={:.2} p95={:.2}ms",
        legacy[6], legacy[11], current[6], current[11], paging[6], paging[11]
    );
    assert!(current[6] < legacy[6] * 0.75);
    assert!(paging[6] < current[6] * 0.75);
    drop(session);
    proxy.join().unwrap();
    for i in 0..40 {
        admin
            .delete(format!("{prefix}{i}").as_bytes(), false)
            .unwrap();
    }
    admin.delete(hash.as_bytes(), false).unwrap();
}

#[test]
fn inspector_uses_two_exchanges_then_one_per_page() {
    use crate::redis_value::{Cursor, PageRequest};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut parser = redis::Parser::new();
        fn command(parser: &mut redis::Parser, socket: &mut std::net::TcpStream) -> Vec<Vec<u8>> {
            redis::from_redis_value(parser.parse_value(socket).unwrap()).unwrap()
        }
        loop {
            let cmd = command(&mut parser, &mut socket);
            socket
                .write_all(if cmd[0] == b"PING" {
                    b"+PONG\r\n"
                } else {
                    b"+OK\r\n"
                })
                .unwrap();
            if cmd[0] == b"SELECT" {
                break;
            }
        }
        for _ in 0..2 {
            for name in [b"TYPE".as_slice(), b"PTTL"] {
                assert_eq!(command(&mut parser, &mut socket)[0], name);
            }
            socket.write_all(b"+list\r\n:-1\r\n").unwrap();
            for name in [b"LLEN".as_slice(), b"LRANGE"] {
                assert_eq!(command(&mut parser, &mut socket)[0], name);
            }
            let mut reply = b":201\r\n*200\r\n".to_vec();
            for _ in 0..200 {
                reply.extend_from_slice(b"$1\r\nx\r\n");
            }
            socket.write_all(&reply).unwrap();
            for name in [b"TYPE".as_slice(), b"PTTL", b"LLEN", b"LRANGE"] {
                let cmd = command(&mut parser, &mut socket);
                assert_eq!(cmd[0], name);
                if name == b"LRANGE" {
                    assert_eq!(cmd[2], b"200");
                }
            }
            socket
                .write_all(b"+list\r\n:-1\r\n:201\r\n*1\r\n$1\r\ny\r\n")
                .unwrap();
        }
        for (cursor, response) in [
            (
                b"0".as_slice(),
                b"+hash\r\n:-1\r\n:1\r\n*2\r\n$2\r\n42\r\n*0\r\n".as_slice(),
            ),
            (
                b"42",
                b"+hash\r\n:-1\r\n:1\r\n*2\r\n$1\r\n0\r\n*2\r\n$3\r\nfoo\r\n$3\r\nbar\r\n",
            ),
        ] {
            for name in [b"TYPE".as_slice(), b"PTTL", b"HLEN", b"HSCAN"] {
                let cmd = command(&mut parser, &mut socket);
                assert_eq!(cmd[0], name);
                if name == b"HSCAN" {
                    assert_eq!(cmd[2], cursor);
                    assert_eq!(cmd[3], b"MATCH");
                    assert_eq!(cmd[4], b"foo*");
                }
            }
            socket.write_all(response).unwrap();
        }
        for name in [b"TYPE".as_slice(), b"PTTL", b"LLEN", b"LRANGE"] {
            assert_eq!(command(&mut parser, &mut socket)[0], name);
        }
        socket
            .write_all(b"+string\r\n:-1\r\n-WRONGTYPE changed\r\n-WRONGTYPE changed\r\n")
            .unwrap();
        assert_eq!(command(&mut parser, &mut socket)[0], b"PING");
        socket.write_all(b"+PONG\r\n").unwrap();
    });
    let mut session = Session::connect(
        &Settings {
            host: "127.0.0.1".into(),
            port,
            tls: false,
            ..Default::default()
        },
        "",
    )
    .unwrap();
    for _ in 0..2 {
        let first = session.inspect_view(b"list").unwrap();
        assert_eq!(first.page.display.rows.len(), 200);
        assert_eq!(first.page.next, Some(Cursor::Offset(200)));
        let page = session
            .value_page(&PageRequest {
                key: b"list".to_vec(),
                kind: "list".into(),
                cursor: first.page.next.unwrap(),
                pattern: b"*".to_vec(),
            })
            .unwrap();
        assert_eq!(page.page.display.rows[0].cells, vec!["200", "y"]);
        assert!(page.page.next.is_none());
    }
    let mut request = PageRequest {
        key: b"hash".to_vec(),
        kind: "hash".into(),
        cursor: Cursor::Scan(0),
        pattern: b"foo*".to_vec(),
    };
    let empty = session.value_page(&request).unwrap();
    assert!(empty.page.display.rows.is_empty());
    request.cursor = empty.page.next.unwrap();
    let page = session.value_page(&request).unwrap();
    assert_eq!(page.page.display.rows[0].cells, vec!["foo", "bar"]);
    assert!(
        session
            .value_page(&PageRequest {
                key: b"changed".to_vec(),
                kind: "list".into(),
                cursor: Cursor::Offset(0),
                pattern: b"*".to_vec()
            })
            .is_err()
    );
    session.query(&redis::cmd("PING")).unwrap();
    drop(session);
    peer.join().unwrap();
}

#[test]
#[ignore = "requires disposable Redis on localhost:16379"]
fn live_collection_pages_search_stream_ids_and_binary_strings() {
    use crate::redis_value::{Cursor, PageRequest};
    let mut session = Session::connect(
        &Settings {
            host: "127.0.0.1".into(),
            port: 16379,
            tls: false,
            ..Default::default()
        },
        "rustable-redis-test-only",
    )
    .unwrap();
    let prefix = format!("rustable-inspector-{}:", std::process::id());
    let key = |name: &str| format!("{prefix}{name}").into_bytes();
    let mut seed = redis::pipe();
    for i in 0..701 {
        seed.cmd("RPUSH").arg(key("list")).arg(format!("value:{i}"));
        seed.cmd("HSET")
            .arg(key("hash"))
            .arg(format!("field:{i}"))
            .arg(format!("value:{i}"));
        seed.cmd("SADD").arg(key("set")).arg(format!("member:{i}"));
        seed.cmd("ZADD")
            .arg(key("zset"))
            .arg(i)
            .arg(format!("member:{i}"));
        if i < 201 {
            seed.cmd("XADD")
                .arg(key("stream"))
                .arg(format!("{}-0", i + 1))
                .arg("name")
                .arg(format!("event:{i}"));
        }
    }
    session.pipeline(&seed).unwrap();
    for kind in ["list", "hash", "set", "zset", "stream"] {
        let mut view = session.inspect_view(&key(kind)).unwrap();
        let mut seen = std::collections::HashSet::new();
        let mut pages = 0;
        loop {
            assert!(!view.snapshot.truncated);
            for row in &view.page.display.rows {
                seen.insert(row.cells[0].clone());
            }
            pages += 1;
            assert!(pages < 100);
            let Some(cursor) = view.page.next else {
                break;
            };
            view = session
                .value_page(&PageRequest {
                    key: key(kind),
                    kind: kind.into(),
                    cursor,
                    pattern: b"*".to_vec(),
                })
                .unwrap();
        }
        assert_eq!(
            seen.len(),
            if kind == "stream" { 201 } else { 701 },
            "{kind}"
        );
        assert!(pages > 1);
    }
    for kind in ["hash", "set", "zset"] {
        let pattern = if kind == "hash" {
            b"field:1*".to_vec()
        } else {
            b"member:1*".to_vec()
        };
        let mut cursor = Cursor::Scan(0);
        let mut seen = std::collections::HashSet::new();
        loop {
            let view = session
                .value_page(&PageRequest {
                    key: key(kind),
                    kind: kind.into(),
                    cursor,
                    pattern: pattern.clone(),
                })
                .unwrap();
            for row in &view.page.display.rows {
                assert!(row.cells[0].split(':').nth(1).unwrap().starts_with('1'));
                seen.insert(row.cells[0].clone());
            }
            let Some(next) = view.page.next else {
                break;
            };
            cursor = next;
        }
        assert_eq!(seen.len(), 111);
    }
    let binary = vec![255u8; 70000];
    session
        .query(redis::cmd("SET").arg(key("string")).arg(&binary))
        .unwrap();
    let first = session.inspect_view(&key("string")).unwrap();
    assert!(first.snapshot.truncated);
    assert!(first.page.display.strings.escaped);
    let last = session
        .value_page(&PageRequest {
            key: key("string"),
            kind: "string".into(),
            cursor: first.page.next.unwrap(),
            pattern: b"*".to_vec(),
        })
        .unwrap();
    assert_eq!(
        last.snapshot.value,
        redis::Value::BulkString(vec![255; 4464])
    );
    assert!(last.page.next.is_none());
    session
        .query(redis::cmd("SET").arg(key("string")).arg(""))
        .unwrap();
    let empty = session.inspect_view(&key("string")).unwrap();
    assert!(!empty.snapshot.truncated);
    assert!(empty.page.next.is_none());
    let request = PageRequest {
        key: key("list"),
        kind: "list".into(),
        cursor: Cursor::Offset(200),
        pattern: b"*".to_vec(),
    };
    session.delete(&key("list"), false).unwrap();
    session
        .query(redis::cmd("SET").arg(key("list")).arg("changed"))
        .unwrap();
    assert!(session.value_page(&request).is_err());
    session.query(&redis::cmd("PING")).unwrap();
    for kind in ["list", "hash", "set", "zset", "stream", "string"] {
        session.delete(&key(kind), false).unwrap();
    }
    assert_eq!(
        session.inspect_view(&key("gone")).unwrap().snapshot.kind,
        "none"
    );
}

#[test]
#[ignore = "requires disposable Redis on localhost:16379"]
fn live_guarded_mutations_bulk_expiry_and_diagnostics() {
    use crate::redis_mutation::{Action, Mutation};
    let mut session = Session::connect(
        &Settings {
            host: "127.0.0.1".into(),
            port: 16379,
            tls: false,
            ..Default::default()
        },
        "rustable-redis-test-only",
    )
    .unwrap();
    let prefix = format!("rustable-mutation-{}:", std::process::id());
    let mut all = Vec::new();
    let mut request = Mutation {
        action: Action::Create,
        keys: vec![],
        kind: String::new(),
        field: b"field".to_vec(),
        value: vec![0, 255, b'v'],
        expected: vec![],
        length: 1,
        ttl_ms: Some(60000),
    };
    for kind in ["string", "hash", "list", "set", "zset", "stream"] {
        let key = format!("{prefix}{kind}").into_bytes();
        all.push(key.clone());
        request.keys = vec![key.clone()];
        request.kind = kind.into();
        request.action = Action::Create;
        request.field = b"field".to_vec();
        request.value = if kind == "zset" {
            b"1.25".to_vec()
        } else {
            vec![0, 255, b'v']
        };
        assert!(session.mutate(&request, true).is_err());
        assert_eq!(session.mutate(&request, false).unwrap(), 1);
        assert!(
            session.mutate(&request, false).is_err(),
            "never overwrite creation"
        );
        let ttl: i64 = session.typed(redis::cmd("PTTL").arg(&key)).unwrap();
        assert!(ttl > 0 && ttl <= 60000);
        if kind == "string" {
            continue;
        }
        if kind == "stream" {
            let view = session.inspect_view(&key).unwrap();
            let redis::Value::Array(rows) = view.snapshot.value else {
                panic!()
            };
            let redis::Value::Array(parts) = &rows[0] else {
                panic!()
            };
            let redis::Value::BulkString(id) = &parts[0] else {
                panic!()
            };
            request.field = id.clone();
            request.action = Action::Remove;
            assert_eq!(session.mutate(&request, false).unwrap(), 1);
            continue;
        }
        request.expected = request.value.clone();
        request.action = Action::Update;
        request.field = match kind {
            "list" => b"0".to_vec(),
            "set" => request.value.clone(),
            _ => b"field".to_vec(),
        };
        request.value = if kind == "zset" {
            b"2.5".to_vec()
        } else {
            b"replacement".to_vec()
        };
        assert_eq!(session.mutate(&request, false).unwrap(), 1);
        assert!(
            session.mutate(&request, false).is_err(),
            "stale edits rejected: {kind}"
        );
        let ttl: i64 = session.typed(redis::cmd("PTTL").arg(&key)).unwrap();
        assert!(ttl > 0, "preserve expiry");
        if kind == "set" {
            request.field = request.value.clone();
        }
        request.expected = request.value.clone();
        request.action = Action::Remove;
        assert_eq!(session.mutate(&request, false).unwrap(), 1);
        assert_eq!(
            session
                .typed::<i64>(redis::cmd("EXISTS").arg(&key))
                .unwrap(),
            0
        );
    }
    let list = format!("{prefix}list").into_bytes();
    session
        .query(
            redis::cmd("RPUSH")
                .arg(&list)
                .arg(&["a", "same", "same", "z"]),
        )
        .unwrap();
    request = Mutation {
        action: Action::Remove,
        keys: vec![list.clone()],
        kind: "list".into(),
        field: b"2".to_vec(),
        value: vec![],
        expected: b"same".to_vec(),
        length: 4,
        ttl_ms: None,
    };
    session.mutate(&request, false).unwrap();
    assert_eq!(
        session
            .typed::<Vec<String>>(redis::cmd("LRANGE").arg(&list).arg(0).arg(-1))
            .unwrap(),
        vec!["a", "same", "z"]
    );
    assert!(session.mutate(&request, false).is_err());
    request.action = Action::Rename;
    request.field = format!("{prefix}string").into_bytes();
    assert!(session.mutate(&request, false).is_err());
    let renamed = format!("{prefix}renamed").into_bytes();
    all.push(renamed.clone());
    request.field = renamed.clone();
    session
        .query(redis::cmd("PEXPIRE").arg(&list).arg(60000))
        .unwrap();
    assert_eq!(session.mutate(&request, false).unwrap(), 1);
    let renamed_ttl: i64 = session.typed(redis::cmd("PTTL").arg(&renamed)).unwrap();
    assert!(renamed_ttl > 0 && renamed_ttl <= 60000);
    request.action = Action::ExpireKeys;
    request.keys = vec![renamed.clone(), format!("{prefix}string").into_bytes()];
    request.ttl_ms = Some(120000);
    assert_eq!(session.mutate(&request, false).unwrap(), 2);
    request.ttl_ms = None;
    assert_eq!(session.mutate(&request, false).unwrap(), 2);
    let server = session
        .diagnostics(&crate::redis_diagnostics::Request::Server)
        .unwrap();
    assert!(!server.summary.is_empty());
    let report = session
        .diagnostics(&crate::redis_diagnostics::Request::Keys(
            request.keys.clone(),
        ))
        .unwrap();
    assert_eq!(report.rows.len(), 2);
    request.action = Action::DeleteKeys;
    request.keys = all;
    // XDEL preserves an empty stream key.
    assert_eq!(session.mutate(&request, false).unwrap(), 3);
}

#[test]
fn mutations_use_one_exchange_without_preflight_or_replay() {
    use crate::redis_mutation::{Action, Mutation};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut parser = redis::Parser::new();
        let read = |p: &mut redis::Parser, s: &mut std::net::TcpStream| -> Vec<Vec<u8>> {
            redis::from_redis_value(p.parse_value(s).unwrap()).unwrap()
        };
        loop {
            let cmd = read(&mut parser, &mut stream);
            let selected = cmd[0] == b"SELECT";
            stream
                .write_all(if cmd[0] == b"PING" {
                    b"+PONG\r\n"
                } else {
                    b"+OK\r\n"
                })
                .unwrap();
            if selected {
                break;
            }
        }
        for _ in 0..2 {
            let cmd = read(&mut parser, &mut stream);
            assert_eq!(cmd[0], b"EVAL");
            assert_eq!(cmd[3], b"key");
            stream.write_all(b":1\r\n").unwrap();
        }
        let cmd = read(&mut parser, &mut stream);
        assert_eq!(cmd[0], b"EVAL");
        // Close before replying: the client must report the unknown outcome,
        // discard this connection and never reconnect or replay the mutation.
    });
    let mut session = Session::connect(
        &Settings {
            host: "127.0.0.1".into(),
            port,
            tls: false,
            ..Default::default()
        },
        "",
    )
    .unwrap();
    let request = Mutation {
        action: Action::Create,
        keys: vec![b"key".to_vec()],
        kind: "string".into(),
        field: vec![],
        value: b"value".to_vec(),
        expected: vec![],
        length: 0,
        ttl_ms: None,
    };
    assert!(session.mutate(&request, true).is_err());
    assert_eq!(session.mutate(&request, false).unwrap(), 1);
    assert_eq!(session.mutate(&request, false).unwrap(), 1);
    assert!(
        session
            .mutate(&request, false)
            .unwrap_err()
            .contains("may have completed")
    );
    assert!(!session.is_connected());
    peer.join().unwrap();
}

#[test]
#[ignore = "requires disposable RedisJSON fixture via scripts/test-redis.sh"]
fn live_json_documents_preserve_ttl_and_reject_stale_edits() {
    use crate::redis_mutation::{Action, Mutation};
    let port = std::env::var("RUSTABLE_REDIS_JSON_PORT")
        .expect("Run scripts/test-redis.sh for the disposable JSON fixture")
        .parse()
        .unwrap();
    let mut session = Session::connect(
        &Settings {
            host: "127.0.0.1".into(),
            port,
            tls: false,
            ..Default::default()
        },
        "rustable-redis-test-only",
    )
    .unwrap();
    let key = format!("rustable-json-{}", std::process::id()).into_bytes();
    let mut request = Mutation {
        action: Action::Create,
        keys: vec![key.clone()],
        kind: "ReJSON-RL".into(),
        field: vec![],
        value: br#"{"name":"Avery","enabled":true,"n":1234567890123456789}"#.to_vec(),
        expected: vec![],
        length: 0,
        ttl_ms: Some(60000),
    };
    session.mutate(&request, false).unwrap();
    let view = session.inspect_view(&key).unwrap();
    assert_eq!(view.snapshot.kind, "ReJSON-RL");
    assert!(view.page.display.strings.json_error.is_none());
    assert!(!view.snapshot.truncated);
    let redis::Value::BulkString(original) = view.snapshot.value else {
        panic!()
    };
    request.action = Action::Update;
    request.expected = original;
    request.value = br#"{"name":"Sam","enabled":false}"#.to_vec();
    session.mutate(&request, false).unwrap();
    assert!(session.mutate(&request, false).is_err());
    let ttl: i64 = session.typed(redis::cmd("PTTL").arg(&key)).unwrap();
    assert!(ttl > 0 && ttl <= 60000);
    request.action = Action::DeleteKeys;
    session.mutate(&request, false).unwrap();
}

#[test]
#[ignore = "requires disposable Redis on localhost:16379"]
fn live_mutation_acl_failures_do_not_leave_partial_edits() {
    use crate::redis_mutation::{Action, Mutation};
    let config = Settings {
        host: "127.0.0.1".into(),
        port: 16379,
        tls: false,
        ..Default::default()
    };
    let mut admin = Session::connect(&config, "rustable-redis-test-only").unwrap();
    let name = format!("mutation-acl-{}", std::process::id());
    admin
        .query(
            redis::cmd("ACL")
                .arg("SETUSER")
                .arg(&name)
                .arg("on")
                .arg(">fixture-password")
                .arg("~*")
                .arg("+@all")
                .arg("-pexpire")
                .arg("-srem"),
        )
        .unwrap();
    let mut restricted = Session::connect(
        &Settings {
            username: name.clone(),
            ..config
        },
        "fixture-password",
    )
    .unwrap();
    let key = format!("rustable-{name}").into_bytes();
    let mut request = Mutation {
        action: Action::Create,
        keys: vec![key.clone()],
        kind: "string".into(),
        field: vec![],
        value: b"value".to_vec(),
        expected: vec![],
        length: 0,
        ttl_ms: Some(60000),
    };
    assert!(restricted.mutate(&request, false).is_err());
    assert_eq!(
        admin.typed::<i64>(redis::cmd("EXISTS").arg(&key)).unwrap(),
        0
    );
    admin
        .query(redis::cmd("SADD").arg(&key).arg("original"))
        .unwrap();
    request.kind = "set".into();
    request.action = Action::Update;
    request.field = b"original".to_vec();
    request.value = b"replacement".to_vec();
    assert!(restricted.mutate(&request, false).is_err());
    assert_eq!(
        admin
            .typed::<Vec<String>>(redis::cmd("SMEMBERS").arg(&key))
            .unwrap(),
        vec!["original"]
    );
    admin.query(redis::cmd("DEL").arg(&key)).unwrap();
    admin
        .query(redis::cmd("ACL").arg("DELUSER").arg(&name))
        .unwrap();
}

#[test]
#[ignore = "requires disposable Redis 8 Search fixture via scripts/test-redis.sh"]
fn live_search_indexes_schema_paging_and_errors() {
    use crate::redis_search::{Reply, Request};
    let port = std::env::var("RUSTABLE_REDIS_JSON_PORT")
        .expect("Run scripts/test-redis.sh")
        .parse()
        .unwrap();
    let mut session = Session::connect(
        &Settings {
            host: "127.0.0.1".into(),
            port,
            tls: false,
            ..Default::default()
        },
        "rustable-redis-test-only",
    )
    .unwrap();
    let index = format!("rustable-search-{}", std::process::id());
    let prefix = format!("{index}:");
    session
        .query(
            redis::cmd("FT.CREATE")
                .arg(&index)
                .arg("ON")
                .arg("HASH")
                .arg("PREFIX")
                .arg(1)
                .arg(&prefix)
                .arg("SCHEMA")
                .arg("name")
                .arg("TEXT")
                .arg("rank")
                .arg("NUMERIC")
                .arg("SORTABLE"),
        )
        .unwrap();
    for i in 0..55 {
        session
            .query(
                redis::cmd("HSET")
                    .arg(format!("{prefix}{i}"))
                    .arg("name")
                    .arg("Avery")
                    .arg("rank")
                    .arg(i),
            )
            .unwrap();
    }
    let Reply::Indexes(indexes) = session.search(Request::Indexes).unwrap() else {
        panic!()
    };
    assert!(indexes.contains(&index.as_bytes().to_vec()));
    let Reply::Schema(schema) = session
        .search(Request::Schema(index.as_bytes().to_vec()))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(schema[0].name, b"name");
    assert_eq!(schema[0].kind, "TEXT");
    for (offset, expected) in [(0, 50), (50, 5)] {
        let Reply::Results { total, keys, .. } = session
            .search(Request::Query {
                index: index.as_bytes().to_vec(),
                query: "@name:avery".into(),
                offset,
                options: Default::default(),
            })
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(total, 55);
        assert_eq!(keys.len(), expected);
        assert!(keys.iter().all(|key| key.starts_with(prefix.as_bytes())));
    }
    let Reply::Results { keys, .. } = session
        .search(Request::Query {
            index: index.as_bytes().to_vec(),
            query: "nothingmatches".into(),
            offset: 0,
            options: Default::default(),
        })
        .unwrap()
    else {
        panic!()
    };
    assert!(keys.is_empty());
    assert!(
        session
            .search(Request::Query {
                index: index.as_bytes().to_vec(),
                query: "@name:[".into(),
                offset: 0,
                options: Default::default(),
            })
            .is_err()
    );
    for descending in [false, true] {
        let Reply::Results { keys, values, .. } = session
            .search(Request::Query {
                index: index.as_bytes().to_vec(),
                query: "*".into(),
                offset: 0,
                options: crate::redis_search::Options {
                    fields: vec![b"rank".to_vec(), b"name".to_vec()],
                    sort: Some(b"rank".to_vec()),
                    descending,
                },
            })
            .unwrap()
        else {
            panic!()
        };
        let first = if descending { 54 } else { 0 };
        assert_eq!(keys[0], format!("{prefix}{first}").as_bytes());
        assert_eq!(values[0], vec![first.to_string(), "Avery".into()]);
    }
    assert!(session.is_connected());
    session
        .query(redis::cmd("FT.DROPINDEX").arg(index).arg("DD"))
        .unwrap();
}

#[test]
fn search_uses_one_exchange_on_first_and_repeated_queries() {
    use crate::redis_search::{Reply, Request};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut parser = redis::Parser::new();
        loop {
            let args: Vec<Vec<u8>> =
                redis::from_redis_value(parser.parse_value(&mut stream).unwrap()).unwrap();
            let reply = if args[0] == b"PING" {
                b"+PONG\r\n".as_slice()
            } else {
                b"+OK\r\n"
            };
            stream.write_all(reply).unwrap();
            if args[0] == b"SELECT" {
                break;
            }
        }
        for i in 0..4 {
            let args: Vec<Vec<u8>> =
                redis::from_redis_value(parser.parse_value(&mut stream).unwrap()).unwrap();
            assert_eq!(args[0], b"FT.SEARCH");
            assert_eq!(args[1], b"idx");
            assert_eq!(
                args[3],
                if i < 2 {
                    b"NOCONTENT".as_slice()
                } else {
                    b"RETURN"
                }
            );
            if i >= 2 {
                assert!(
                    args.windows(3)
                        .any(|a| a == [b"SORTBY".to_vec(), b"name".to_vec(), b"ASC".to_vec()])
                );
            }
            stream
                .write_all(if i < 2 {
                    b"*2\r\n:1\r\n$3\r\na\0b\r\n".as_slice()
                } else {
                    b"*3\r\n:1\r\n$3\r\na\0b\r\n*2\r\n$4\r\nname\r\n$5\r\nAvery\r\n"
                })
                .unwrap();
        }
    });
    let mut session = Session::connect(
        &Settings {
            host: "127.0.0.1".into(),
            port,
            tls: false,
            ..Default::default()
        },
        "",
    )
    .unwrap();
    for i in 0..4 {
        let Reply::Results { keys, .. } = session
            .search(Request::Query {
                index: b"idx".to_vec(),
                query: "*".into(),
                offset: 0,
                options: if i < 2 {
                    Default::default()
                } else {
                    crate::redis_search::Options {
                        fields: vec![b"name".to_vec()],
                        sort: Some(b"name".to_vec()),
                        descending: false,
                    }
                },
            })
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(keys, vec![b"a\0b".to_vec()]);
    }
    peer.join().unwrap();
}

#[test]
#[ignore = "requires disposable Redis 8 Search fixture via scripts/test-redis.sh"]
fn live_aggregations_and_query_profiles() {
    use crate::redis_analytics::{Query, Reducer};
    use crate::redis_search::{Options, Reply, Request};
    let port = std::env::var("RUSTABLE_REDIS_JSON_PORT")
        .expect("Run scripts/test-redis.sh")
        .parse()
        .unwrap();
    let mut session = Session::connect(
        &Settings {
            host: "127.0.0.1".into(),
            port,
            tls: false,
            ..Default::default()
        },
        "rustable-redis-test-only",
    )
    .unwrap();
    let index = format!("rustable-analytics-{}", std::process::id());
    let prefix = format!("{index}:");
    session
        .query(
            redis::cmd("FT.CREATE")
                .arg(&index)
                .arg("ON")
                .arg("HASH")
                .arg("PREFIX")
                .arg(1)
                .arg(&prefix)
                .arg("SCHEMA")
                .arg("plan")
                .arg("TAG")
                .arg("rank")
                .arg("NUMERIC")
                .arg("SORTABLE"),
        )
        .unwrap();
    for i in 0..55 {
        session
            .query(
                redis::cmd("HSET")
                    .arg(format!("{prefix}{i}"))
                    .arg("plan")
                    .arg("Pro")
                    .arg("rank")
                    .arg(i),
            )
            .unwrap();
    }
    let base = Query {
        index: index.as_bytes().to_vec(),
        query: "*".into(),
        ..Default::default()
    };
    for (reducer, expected) in [
        (Reducer::Sum, "1485"),
        (Reducer::Average, "27"),
        (Reducer::Minimum, "0"),
        (Reducer::Maximum, "54"),
    ] {
        let Reply::Aggregate(report) = session
            .search(Request::Aggregate(Query {
                reducer,
                metric: Some(b"rank".to_vec()),
                ..base.clone()
            }))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            report.rows,
            vec![vec!["55".to_owned(), expected.to_owned()]]
        );
    }
    let q = Query {
        groups: vec![b"plan".to_vec()],
        load: vec![b"plan".to_vec()],
        ..base.clone()
    };
    let Reply::Aggregate(report) = session.search(Request::Aggregate(q.clone())).unwrap() else {
        panic!()
    };
    assert_eq!(report.rows, vec![vec!["Pro".to_owned(), "55".to_owned()]]);
    for (offset, rows, more) in [(0, 50, true), (50, 5, false)] {
        let Reply::Aggregate(report) = session
            .search(Request::Aggregate(Query {
                groups: vec![b"rank".to_vec()],
                offset,
                ..base.clone()
            }))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(report.rows.len(), rows);
        assert_eq!(report.more, more);
    }
    for request in [
        Request::Query {
            index: index.as_bytes().to_vec(),
            query: "*".into(),
            offset: 0,
            options: Options::default(),
        },
        Request::Aggregate(q),
    ] {
        let Reply::Profile { reply, profile } =
            session.search(Request::Profile(Box::new(request))).unwrap()
        else {
            panic!()
        };
        assert!(profile.client_ms > 0.0);
        assert!(!profile.rows.is_empty());
        assert!(
            profile
                .rows
                .iter()
                .any(|r| r[0].contains("Total profile time"))
        );
        assert!(matches!(
            *reply,
            Reply::Results { .. } | Reply::Aggregate(_)
        ));
    }
    session
        .query(redis::cmd("FT.DROPINDEX").arg(index).arg("DD"))
        .unwrap();
}

#[test]
fn analytics_and_profiles_have_no_preflight_or_cleanup_exchange() {
    use crate::redis_analytics::Query;
    use crate::redis_search::{Options, Reply, Request};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut parser = redis::Parser::new();
        loop {
            let args: Vec<Vec<u8>> =
                redis::from_redis_value(parser.parse_value(&mut stream).unwrap()).unwrap();
            stream
                .write_all(if args[0] == b"PING" {
                    b"+PONG\r\n".as_slice()
                } else {
                    b"+OK\r\n"
                })
                .unwrap();
            if args[0] == b"SELECT" {
                break;
            }
        }
        for _ in 0..2 {
            let args: Vec<Vec<u8>> =
                redis::from_redis_value(parser.parse_value(&mut stream).unwrap()).unwrap();
            assert_eq!(args[0], b"FT.AGGREGATE");
            assert!(
                args.windows(3)
                    .any(|p| p == [b"LIMIT".to_vec(), b"0".to_vec(), b"51".to_vec()])
            );
            stream.write_all(b"*1\r\n:0\r\n").unwrap();
            for kind in [b"SEARCH".as_slice(), b"AGGREGATE"] {
                let args: Vec<Vec<u8>> =
                    redis::from_redis_value(parser.parse_value(&mut stream).unwrap()).unwrap();
                assert_eq!(args[0], b"FT.PROFILE");
                assert_eq!(args[2], kind);
                assert_eq!(args[3], b"LIMITED");
                stream
                    .write_all(
                        b"*2\r\n*1\r\n:0\r\n*2\r\n$18\r\nTotal profile time\r\n$3\r\n0.1\r\n",
                    )
                    .unwrap();
            }
        }
    });
    let mut session = Session::connect(
        &Settings {
            host: "127.0.0.1".into(),
            port,
            tls: false,
            ..Default::default()
        },
        "",
    )
    .unwrap();
    let aggregate = Request::Aggregate(Query {
        index: b"idx".to_vec(),
        query: "*".into(),
        ..Default::default()
    });
    let search = Request::Query {
        index: b"idx".to_vec(),
        query: "*".into(),
        offset: 0,
        options: Options::default(),
    };
    for _ in 0..2 {
        assert!(matches!(
            session.search(aggregate.clone()).unwrap(),
            Reply::Aggregate(_)
        ));
        for q in [search.clone(), aggregate.clone()] {
            assert!(matches!(
                session.search(Request::Profile(Box::new(q))).unwrap(),
                Reply::Profile { .. }
            ));
        }
    }
    peer.join().unwrap();
}

#[test]
#[ignore = "disposable ephemeral Redis JSON fixture scripts/test-redis-json-edits.sh"]
fn live_json_path_edits_preserve_conflicts_precision_and_expiry() {
    use crate::redis_mutation::{Action, Mutation};
    assert_eq!(std::env::var("RUSTABLE_REDIS_JSON_EDIT_QA").as_deref(),Ok("1"));
    let mut session=Session::connect(&Settings {
        host:"127.0.0.1".into(), port:std::env::var("RUSTABLE_REDIS_JSON_EDIT_PORT").unwrap().parse().unwrap(),tls:false,..Default::default()
    },"rustable-json-test-only").unwrap();
    let key=b"json:path:test";
    session.query(redis::cmd("JSON.SET").arg(key).arg("$").arg(r#"{"name":"Avery","items":[1,2,3],"n":1234567890123456789}"#)).unwrap();
    session.query(redis::cmd("PEXPIRE").arg(key).arg(60000)).unwrap();
    let mut mutation=Mutation {action:Action::Update,keys:vec![key.to_vec()],kind:"ReJSON-RL".into(),field:b"$.name".to_vec(),value:br#""Sam""#.to_vec(),expected:session.typed(redis::cmd("JSON.GET").arg(key)).unwrap(),length:0,ttl_ms:None};
    assert!(session.mutate(&mutation,true).is_err());
    session.mutate(&mutation,false).unwrap();
    let error=session.mutate(&mutation,false).unwrap_err();
    assert!(error.contains("changed. Refresh before editing."),"{error}");
    mutation.expected=session.typed(redis::cmd("JSON.GET").arg(key)).unwrap();
    assert!(std::str::from_utf8(&mutation.expected).unwrap().contains("1234567890123456789"));
    mutation.field=b"$[\"new.property\"]".to_vec();
    mutation.value=b"true".to_vec();
    session.mutate(&mutation,false).unwrap();
    mutation.expected=session.typed(redis::cmd("JSON.GET").arg(key)).unwrap();
    mutation.field=b"$.missing.child".to_vec();
    assert!(session.mutate(&mutation,false).is_err());
    let unchanged:Vec<u8>=session.typed(redis::cmd("JSON.GET").arg(key)).unwrap();
    assert_eq!(unchanged,mutation.expected);
    mutation.action=Action::Remove;
    mutation.field=b"$.items[1]".to_vec();
    session.mutate(&mutation,false).unwrap();
    let values:String=session.typed(redis::cmd("JSON.GET").arg(key).arg("$.items")).unwrap();
    assert_eq!(values,"[[1,3]]");
    mutation.expected=session.typed(redis::cmd("JSON.GET").arg(key)).unwrap();
    mutation.field=b"$.absent".to_vec();
    assert!(session.mutate(&mutation,false).unwrap_err().contains("does not exist"));
    for path in ["$", "$..name", "$.items[*]"] {
        mutation.field=path.as_bytes().to_vec();
        assert!(session.mutate(&mutation,false).is_err());
    }
    let unchanged:Vec<u8>=session.typed(redis::cmd("JSON.GET").arg(key)).unwrap();
    assert_eq!(unchanged,mutation.expected);
    let ttl:i64=session.typed(redis::cmd("PTTL").arg(key)).unwrap();
    assert!(ttl>0 && ttl<=60000);
    session.query(redis::cmd("DEL").arg(key)).unwrap();
}
