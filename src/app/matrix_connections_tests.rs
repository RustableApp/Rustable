use super::*;
use serde_json::{Value as Json, json};

fn settle(app: &mut Rustable) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while app.pending.is_some() {
        assert!(
            std::time::Instant::now() < deadline,
            "matrix worker timed out"
        );
        app.poll();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
fn call(app: &mut Rustable, name: &str, mut arguments: Json) -> Result<Json, String> {
    let (reply, receive) = mpsc::sync_channel(1);
    arguments["workspace_id"] = json!(app.matrix_id().unwrap());
    app.handle_matrix(crate::matrix::Request {
        name: name.into(),
        arguments,
        expires: std::time::Instant::now() + std::time::Duration::from_secs(5),
        reply,
    });
    receive.try_recv().unwrap()
}
fn run(app: &mut Rustable, sql: &str) -> Json {
    let id =
        call(app, "open_query", json!({"title":"Fixture","sql":sql})).unwrap()["tab_id"].clone();
    let result = call(app, "run_query", json!({"tab_id":id,"expected_sql":sql})).unwrap();
    assert_eq!(result["started"], true, "{result}");
    settle(app);
    call(app, "inspect_query", json!({"tab_id":id})).unwrap()
}
fn connect(command: Command) -> Rustable {
    let ctx = egui::Context::default();
    let mut app = Rustable::with_initial_connection(&ctx, Preferences::default(), command, None);
    app.offer_matrix();
    settle(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(
        app.matrix_summary().is_none(),
        "connections must start unexposed"
    );
    app.connections = false;
    app.safe = true;
    app.grant_matrix_for_test();
    app
}

#[test]
fn matrix_real_sqlite_file_reuses_ui_session_and_revokes_on_reconnect() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("real.sqlite");
    let admin = rusqlite::Connection::open(&path).unwrap();
    admin.execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY, value TEXT); INSERT INTO items VALUES(1,'original')").unwrap();
    let mut app = connect(Command::Open(Some(path.clone())));
    let report = run(&mut app, "SELECT value FROM items");
    assert!(report["error"].is_null(), "{report}");
    assert_eq!(report["preview"][0][0], "original");
    assert!(!run(&mut app, "UPDATE items SET value='blocked'")["error"].is_null());
    // User opts out of Safe mode. Ordinary writes use the same live session.
    app.safe = false;
    assert!(run(&mut app, "INSERT INTO items VALUES(2,'saved')")["error"].is_null());
    assert_eq!(
        admin
            .query_row("SELECT count(*) FROM items", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    app.protection.mode = query_protection::Mode::AlertWrites;
    // Protected SQL opens the normal confirmation and does not run.
    let sql = "DELETE FROM items";
    let tab = call(&mut app, "open_query", json!({"title":"Delete","sql":sql})).unwrap()["tab_id"]
        .clone();
    let reply = call(
        &mut app,
        "run_query",
        json!({"tab_id":tab,"expected_sql":sql}),
    )
    .unwrap();
    assert_eq!(reply["awaiting_confirmation"], true);
    assert!(app.pending.is_none());
    assert_eq!(
        admin
            .query_row("SELECT count(*) FROM items", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    app.pause_matrix(false);
    assert!(!app.protection.modal_open());
    app.grant_matrix_for_test();
    let id = app.matrix_id().unwrap();
    app.dispatch(Command::Open(Some(path)), Pending::Connect(None));
    assert_ne!(app.matrix_id(), Some(id));
    assert!(app.matrix_summary().is_none());
    settle(&mut app);
    assert!(app.matrix_summary().is_none());
}

#[test]
#[ignore = "requires opted-in disposable PostgreSQL on 54329"]
fn matrix_postgres_live_connection() {
    assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
    let mut admin = ::postgres::Client::connect(
        "host=127.0.0.1 port=54329 user=postgres password=rustable_qa_only dbname=rustable_qa",
        ::postgres::NoTls,
    )
    .unwrap();
    admin.batch_execute("DROP SCHEMA IF EXISTS matrix_qa CASCADE; CREATE SCHEMA matrix_qa;
        CREATE TYPE matrix_qa.mood AS ENUM('calm','busy');
        CREATE TABLE matrix_qa.items(id int PRIMARY KEY, mood matrix_qa.mood);
        INSERT INTO matrix_qa.items VALUES(1,'calm');
        CREATE FUNCTION matrix_qa.mutate() RETURNS int LANGUAGE plpgsql AS $$ BEGIN DELETE FROM matrix_qa.items; RETURN 1; END $$;
        CREATE VIEW matrix_qa.danger AS SELECT matrix_qa.mutate() AS n;").unwrap();
    let config = crate::connection::PostgresConfig {
        host: "127.0.0.1".into(),
        port: 54329,
        database: "rustable_qa".into(),
        user: "postgres".into(),
        password: "rustable_qa_only".into(),
        tls: crate::connection::PostgresTlsMode::Disable,
        ssh: None,
    };
    let mut app = connect(Command::OpenPostgres(config));
    let version = run(&mut app, "SELECT version()");
    eprintln!("Matrix PostgreSQL fixture: {}", version["preview"][0][0]);
    for sql in [
        "SELECT * FROM matrix_qa.items",
        "SELECT * FROM matrix_qa.items WHERE false",
    ] {
        let result = run(&mut app, sql);
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(result["columns"], json!(["id", "mood"]));
    }
    for sql in [
        "DELETE FROM matrix_qa.items",
        "SELECT * FROM matrix_qa.danger",
        "SELECT * FROM missing_matrix_table",
    ] {
        assert!(!run(&mut app, sql)["error"].is_null(), "{sql}");
    }
    let sql = "SELECT pg_sleep(30)";
    let tab = call(&mut app, "open_query", json!({"title":"Cancel","sql":sql})).unwrap()["tab_id"]
        .clone();
    call(
        &mut app,
        "run_query",
        json!({"tab_id":tab,"expected_sql":sql}),
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    call(&mut app, "cancel_query", json!({})).unwrap();
    settle(&mut app);
    assert!(!call(&mut app, "inspect_query", json!({"tab_id":tab})).unwrap()["error"].is_null());
    assert!(run(&mut app, "SELECT 42")["error"].is_null());
    let id = app.matrix_id().unwrap();
    app.dispatch(Command::SelectSchema("matrix_qa".into()), Pending::Export);
    assert_ne!(app.matrix_id(), Some(id));
    settle(&mut app);
    assert!(app.matrix_summary().is_none());
    let count: i64 = admin
        .query_one("SELECT count(*) FROM matrix_qa.items", &[])
        .unwrap()
        .get(0);
    assert_eq!(count, 1);
    admin
        .batch_execute("DROP SCHEMA matrix_qa CASCADE")
        .unwrap();
}

#[test]
#[ignore = "requires disposable MySQL/MariaDB on 43316 or 43317"]
fn matrix_mysql_live_connection() {
    let port: u16 = std::env::var("RUSTABLE_MYSQL_TEST_PORT")
        .unwrap()
        .parse()
        .unwrap();
    assert!([43316, 43317].contains(&port));
    let config = crate::connection::MysqlConfig {
        host: "127.0.0.1".into(),
        port,
        database: "rustable_test".into(),
        user: "rustable".into(),
        password: "rustable-test-only".into(),
        tls: false,
        ..Default::default()
    };
    let mut app = connect(Command::OpenMysql(config));
    eprintln!(
        "Matrix MySQL fixture: {}",
        run(&mut app, "SELECT version()")["preview"][0][0]
    );
    assert!(run(&mut app, "SELECT CAST(1 AS UNSIGNED) AS id WHERE false")["error"].is_null());
    assert!(!run(&mut app, "CREATE TABLE matrix_should_not_exist(id int)")["error"].is_null());
    assert!(!run(&mut app, "SELECT * FROM matrix_missing_table")["error"].is_null());
    let sql = "SELECT SLEEP(30)";
    let tab = call(&mut app, "open_query", json!({"title":"Cancel","sql":sql})).unwrap()["tab_id"]
        .clone();
    call(
        &mut app,
        "run_query",
        json!({"tab_id":tab,"expected_sql":sql}),
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    call(&mut app, "cancel_query", json!({})).unwrap();
    settle(&mut app);
    assert!(run(&mut app, "SELECT 42")["error"].is_null());
}
