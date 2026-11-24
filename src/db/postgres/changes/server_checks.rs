//! Execute guards on the server, in their original order, without client waits.
//! Commit remains a separate, cancellable client decision. No helper is installed.
use super::*;

// Dollar quoting is independent of standard_conforming_strings. Pick a delimiter
// absent from the entire value (including SQL bodies containing other literals).
fn literal(value: &str) -> Result<String, String> {
    if value.contains('\0') || value.len() > BYTE_LIMIT {
        return Err("Invalid or oversized PostgreSQL editing value".into());
    }
    let mut suffix = String::new();
    loop {
        let delimiter = format!("$rustable{suffix}$");
        if !value.contains(&delimiter) {
            if value.len() + 2 * delimiter.len() > BYTE_LIMIT {
                return Err("Editing batch exceeds the memory limit".into());
            }
            return Ok(format!("{delimiter}{value}{delimiter}"));
        }
        // Exponential growth bounds scans even for adversarial delimiter text.
        // A delimiter longer than the input necessarily cannot occur in it.
        suffix = "_".repeat((suffix.len() * 2).max(1));
    }
}

fn execute(sql: &str, parameters: &[Value], into: &str) -> Result<String, String> {
    let mut result = format!("EXECUTE {}{into}", literal(sql)?);
    if !parameters.is_empty() {
        result.push_str(" USING ");
        for (index, value) in parameters.iter().enumerate() {
            if index != 0 { result.push(','); }
            match value {
                Value::Null => result.push_str("NULL::pg_catalog.text"),
                Value::Text(value) => result.push_str(&format!("{}::pg_catalog.text", literal(value)?)),
                _ => return Err("Invalid PostgreSQL reviewed parameter encoding; regenerate the plan".into()),
            }
            if result.len() > BYTE_LIMIT { return Err("Editing batch exceeds the memory limit".into()); }
        }
    }
    result.push(';');
    Ok(result)
}

fn batch(plan: &ChangePlan) -> Result<String, String> {
    if plan.rebuild || plan.schema_version.is_some() {
        return Err("Invalid PostgreSQL change plan".into());
    }
    let schema_plan = plan.statements.iter().all(|s| s.expected_rows.is_none());
    let mut body = String::from("DECLARE payload pg_catalog.jsonb; fingerprint pg_catalog.jsonb; affected pg_catalog.int8; BEGIN ");
    let mut schemas = plan.schemas.iter().collect::<Vec<_>>();
    schemas.sort_by(|a,b| a.0.cmp(&b.0));
    for (table, expected) in schemas {
        body.push_str(&execute(&format!("LOCK TABLE {table} IN {} MODE", if schema_plan { "ACCESS EXCLUSIVE" } else { "ROW EXCLUSIVE" }), &[], "")?);
        // Dynamic execution gets a fresh snapshot AFTER the lock, including when
        // another session committed DDL while we waited for that lock.
        body.push_str(&execute(&metadata_sql(BYTE_LIMIT), &[Value::Text(table.clone())], " INTO STRICT payload")?);
        body.push_str("IF payload IS NULL THEN RAISE EXCEPTION 'Editing metadata exceeds the memory limit; nothing was saved'; END IF;");
        // Reconstruct the same tuple as decode_metadata. Types contain original
        // server-generated signature strings; never normalize their contents.
        body.push_str(r#"SELECT pg_catalog.jsonb_build_array(payload->'oid',
            COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_array(c->0,c->1,c->2,c->3,c->4,c->5,c->6,
                '"' || pg_catalog.replace(c->>7,'"','""') || '"."' || pg_catalog.replace(c->>8,'"','""') || '"',c->10) ORDER BY ordinal)
                FROM pg_catalog.jsonb_array_elements(payload->'columns') WITH ORDINALITY AS x(c,ordinal)), '[]'::pg_catalog.jsonb),
            payload->'constraints',
            COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_array(t,payload->'trigger_functions'->(ordinal::int-1)) ORDER BY ordinal)
                FROM pg_catalog.jsonb_array_elements(payload->'trigger_definitions') WITH ORDINALITY AS x(t,ordinal)), '[]'::pg_catalog.jsonb),
            payload->'types',payload->'indexes',payload->'cascade_triggers') INTO fingerprint;
            "#);
        body.push_str(&format!("IF fingerprint IS DISTINCT FROM {}::pg_catalog.jsonb THEN RAISE EXCEPTION 'Table structure changed since review. Nothing was saved; refresh and review again.'; END IF;", literal(expected)?));
        if body.len() > BYTE_LIMIT { return Err("Editing batch exceeds the memory limit".into()); }
    }
    for (index, statement) in plan.statements.iter().enumerate() {
        for check in plan.preconditions.iter().filter(|c| c.before_statement == index) {
            body.push_str(&execute(&check.sql, &check.parameters, " INTO STRICT affected")?);
            let message = if schema_plan { "A type definition changed since review. Nothing was saved; review the structure changes again." } else { "A row changed since it was loaded. Nothing was saved; pending changes are retained." };
            body.push_str(&format!("IF affected IS DISTINCT FROM 1 THEN RAISE EXCEPTION '{}'; END IF;", message));
            if body.len() > BYTE_LIMIT { return Err("Editing batch exceeds the memory limit".into()); }
        }
        body.push_str(&execute(&statement.sql, &statement.parameters, "")?);
        if let Some(expected) = statement.expected_rows {
            body.push_str(&format!("GET DIAGNOSTICS affected = ROW_COUNT; IF affected IS DISTINCT FROM {expected} THEN RAISE EXCEPTION 'A write did not affect the expected number of rows. Nothing was saved; pending changes are retained.'; END IF;"));
        }
        if body.len() > BYTE_LIMIT { return Err("Editing batch exceeds the memory limit".into()); }
    }
    body.push_str("END");
    Ok(format!("DO {} LANGUAGE plpgsql", literal(&body)?))
}

pub(super) fn apply(driver: &mut PostgresDriver, plan: &ChangePlan) -> Result<(), String> {
    driver.require_idle_transaction()?;
    driver.check_tunnel()?;
    let sql = batch(plan)?;
    if driver.cancel.load(Ordering::Relaxed) { return Err("Cancelled. Nothing was saved.".into()); }
    let _watch = driver.watch();
    let tx = driver.client.build_transaction().read_only(false).start_with_batch(&sql).map_err(error)?;
    if driver.cancel.load(Ordering::Relaxed) { return Err("Cancelled. Nothing was saved.".into()); }
    tx.commit().map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::fixture;

    fn edit(table: &str, id: i64, original: &str, value: &str) -> Change {
        Change::Cell { table: table.into(), column: "n".into(), original_columns: None,
            original: vec![Value::Integer(id), Value::Text(original.into())], value: Value::Text(value.into()) }
    }

    #[test]
    fn delimiters_and_invalid_values() {
        let text = "'$rustable$\\$rustable_$ $rustable__$☃";
        assert_eq!(literal(text).unwrap(), format!("$rustable____${text}$rustable____$"));
        assert!(literal("a\0b").is_err());
        assert!(execute("SELECT $1", &[Value::Integer(1)], "").is_err());
    }

    #[test]
    #[ignore = "requires disposable PostgreSQL QA"]
    fn bound_values_and_compatibility_fallback() {
        let mut driver = fixture();
        driver.client.batch_execute("CREATE TEMP TABLE server_values(id int PRIMARY KEY,n text); INSERT INTO server_values VALUES(1,'old'); SET standard_conforming_strings=off").unwrap();
        let value = "quotes ' \"; \\ paths; $rustable_0$ $rustable_1$; Unicode ☃; --";
        let reviewed = plan(&mut driver, &[edit("server_values",1,"old",value)]).unwrap();
        assert!(reviewed.postgres_server_checks);
        apply(&mut driver, &reviewed).unwrap();
        assert_eq!(driver.client.query_one("SELECT n FROM server_values", &[]).unwrap().get::<_,String>(0),value);
        let mut fallback = plan(&mut driver, &[edit("server_values",1,value,"fallback")]).unwrap();
        fallback.postgres_server_checks = false;
        super::super::apply(&mut driver, &fallback).unwrap();
        assert_eq!(driver.client.query_one("SELECT n FROM server_values", &[]).unwrap().get::<_,String>(0),"fallback");
    }

    #[test]
    #[ignore = "requires disposable PostgreSQL QA"]
    fn failed_guards_skip_writes_not_just_rollback() {
        let mut driver = fixture();
        driver.client.batch_execute("CREATE TEMP SEQUENCE guard_probe; CREATE TEMP TABLE server_guards(id int PRIMARY KEY,n text); INSERT INTO server_guards VALUES(1,'old'),(2,'old'); CREATE FUNCTION pg_temp.guard_probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM nextval('guard_probe'); RETURN NEW; END $$; CREATE TRIGGER probe BEFORE UPDATE ON server_guards FOR EACH ROW EXECUTE FUNCTION pg_temp.guard_probe()").unwrap();
        let mut reviewed = plan(&mut driver, &[edit("server_guards",1,"stale","new")]).unwrap();
        assert!(apply(&mut driver,&reviewed).unwrap_err().contains("row changed"));
        assert!(!driver.client.query_one("SELECT is_called FROM guard_probe", &[]).unwrap().get::<_,bool>(0));
        reviewed = plan(&mut driver, &[edit("server_guards",1,"old","new")]).unwrap();
        driver.client.batch_execute("ALTER TABLE server_guards ADD COLUMN extra int").unwrap();
        assert!(apply(&mut driver,&reviewed).unwrap_err().contains("structure changed"));
        assert!(!driver.client.query_one("SELECT is_called FROM guard_probe", &[]).unwrap().get::<_,bool>(0));
        driver.client.batch_execute("ALTER TABLE server_guards DROP COLUMN extra; CREATE OR REPLACE FUNCTION pg_temp.guard_probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM nextval('guard_probe'); RETURN NULL; END $$").unwrap();
        reviewed = plan(&mut driver, &[edit("server_guards",1,"old","new"),edit("server_guards",2,"old","new")]).unwrap();
        assert!(apply(&mut driver,&reviewed).unwrap_err().contains("expected number"));
        assert_eq!(driver.client.query_one("SELECT last_value FROM guard_probe", &[]).unwrap().get::<_,i64>(0),1,"failed affected-row check must skip the second write");
        assert_eq!(driver.client.query_one("SELECT count(*) FROM server_guards WHERE n='old'", &[]).unwrap().get::<_,i64>(0),2);
        assert!(driver.require_idle_transaction().is_ok());
    }

    #[test]
    #[ignore = "requires disposable PostgreSQL QA"]
    fn unavailable_language_uses_existing_guards() {
        let mut driver = fixture();
        driver.client.batch_execute("BEGIN; CREATE ROLE rustable_no_plpgsql; REVOKE USAGE ON LANGUAGE plpgsql FROM PUBLIC; SET LOCAL ROLE rustable_no_plpgsql; CREATE TEMP TABLE language_fallback(id int PRIMARY KEY,n text)").unwrap();
        let reviewed = plan(&mut driver, &[edit("language_fallback",1,"old","new")]).unwrap();
        assert!(!reviewed.postgres_server_checks);
        driver.client.batch_execute("ROLLBACK").unwrap();
    }

    #[test]
    #[ignore = "requires disposable PostgreSQL QA"]
    fn multiple_tables_keep_fingerprints_and_fail_atomically() {
        let mut driver = fixture();
        driver.client.batch_execute("CREATE TEMP TABLE multi_a(id int PRIMARY KEY,n text); CREATE TEMP TABLE multi_b(id int PRIMARY KEY,n text); INSERT INTO multi_a VALUES(1,'old'); INSERT INTO multi_b VALUES(1,'old')").unwrap();
        let edits = [edit("multi_b",1,"old","new"),edit("multi_a",1,"old","new")];
        let reviewed = plan(&mut driver,&edits).unwrap();
        let legacy = plan_legacy(&mut driver,&edits).unwrap();
        assert_eq!(serde_json::to_value(&reviewed).unwrap(),serde_json::to_value(legacy).unwrap());
        driver.client.batch_execute("UPDATE multi_a SET n='changed'").unwrap();
        assert!(apply(&mut driver,&reviewed).unwrap_err().contains("row changed"));
        assert_eq!(driver.client.query_one("SELECT n FROM multi_b",&[]).unwrap().get::<_,String>(0),"old");
        assert!(plan(&mut driver,&[edit("multi_a",1,"old","new"),edit("multi_missing",1,"old","new")]).unwrap_err().contains("existing PostgreSQL tables"));
        assert!(driver.require_idle_transaction().is_ok());
        driver.client.batch_execute("UPDATE multi_a SET n='old'").unwrap();
        apply(&mut driver,&reviewed).unwrap();
    }

    #[test]
    #[ignore = "requires disposable PostgreSQL QA"]
    fn cached_preparation_revalidates_schema_and_recovers() {
        let mut driver = fixture();
        driver.client.batch_execute("CREATE TEMP TABLE cached_edit(id int PRIMARY KEY,n text); INSERT INTO cached_edit VALUES(1,'old')").unwrap();
        let mut first = plan(&mut driver,&[edit("cached_edit",1,"old","new")]).unwrap();
        let canonical = first.schemas[0].0.clone();
        let edits = [edit(&canonical,1,"old","new")];
        // Readback retains the caller's table spelling; compare equivalent inputs.
        for readback in &mut first.readbacks { readback.table = canonical.clone(); }
        let cached = plan(&mut driver,&edits).unwrap();
        assert_eq!(serde_json::to_value(&first).unwrap(),serde_json::to_value(&cached).unwrap());
        // Simulate an external schema change without driver invalidation.
        driver.client.batch_execute("ALTER TABLE cached_edit ALTER COLUMN n TYPE varchar").unwrap();
        let stale = plan(&mut driver,&edits).unwrap();
        assert_eq!(stale.schemas,cached.schemas);
        assert!(super::super::apply(&mut driver,&stale).unwrap_err().contains("structure changed"));
        assert!(driver.edit_catalog.is_none());
        let fresh = plan(&mut driver,&edits).unwrap();
        assert_ne!(fresh.schemas,cached.schemas);
        super::super::apply(&mut driver,&fresh).unwrap();
        assert!(driver.edit_catalog.is_some());
        assert_eq!(driver.client.query_one("SELECT n FROM cached_edit",&[]).unwrap().get::<_,String>(0),"new");
        driver.execute("SELECT 1",false,&AtomicBool::new(false));
        assert!(driver.edit_catalog.is_none());
        driver.client.batch_execute("BEGIN").unwrap();
        plan(&mut driver,&[edit(&canonical,1,"new","next")]).unwrap();
        assert!(driver.edit_catalog.is_none(),"uncommitted metadata must not survive rollback in the cache");
        driver.client.batch_execute("ROLLBACK").unwrap();
    }

    #[test]
    #[ignore = "requires disposable PostgreSQL QA"]
    fn lost_commit_response_never_replays_write() {
        use std::io::{Read, Write};
        use std::net::{Shutdown, TcpListener, TcpStream};
        let mut observer = fixture();
        observer.client.batch_execute("DROP SCHEMA IF EXISTS rustable_commit_loss CASCADE; CREATE SCHEMA rustable_commit_loss; CREATE SEQUENCE rustable_commit_loss.probe; CREATE TABLE rustable_commit_loss.t(id int PRIMARY KEY,n text); INSERT INTO rustable_commit_loss.t VALUES(1,'old'); CREATE FUNCTION rustable_commit_loss.probe() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM nextval('rustable_commit_loss.probe'); RETURN NEW; END $$; CREATE TRIGGER probe BEFORE UPDATE ON rustable_commit_loss.t FOR EACH ROW EXECUTE FUNCTION rustable_commit_loss.probe()").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let proxy = std::thread::spawn(move || {
            let (mut downstream, _) = listener.accept().unwrap();
            let mut upstream = TcpStream::connect("127.0.0.1:54329").unwrap();
            let mut input = downstream.try_clone().unwrap();
            let mut output = upstream.try_clone().unwrap();
            let forward = std::thread::spawn(move || { let _ = std::io::copy(&mut input, &mut output); });
            loop {
                let mut header = [0;5];
                upstream.read_exact(&mut header).unwrap();
                let length = u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
                assert!((4..=1024*1024).contains(&length));
                let mut body = vec![0;length-4];
                upstream.read_exact(&mut body).unwrap();
                // Server has committed, but Rustable receives neither the
                // completion nor ReadyForQuery. This is an unknown outcome.
                if header[0] == b'C' && body == b"COMMIT\0" { break; }
                downstream.write_all(&header).unwrap();
                downstream.write_all(&body).unwrap();
            }
            let _ = downstream.shutdown(Shutdown::Both);
            let _ = upstream.shutdown(Shutdown::Both);
            forward.join().unwrap();
        });
        let mut driver = PostgresDriver::open(&crate::connection::PostgresConfig {
            host:"127.0.0.1".into(), port, database:"rustable_qa".into(), user:"postgres".into(),
            password:"rustable_qa_only".into(), tls:crate::connection::PostgresTlsMode::Disable, ssh:None,
        }, Arc::new(AtomicBool::new(false))).unwrap();
        let reviewed = plan(&mut driver, &[edit("rustable_commit_loss.t",1,"old","new")]).unwrap();
        assert!(apply(&mut driver,&reviewed).is_err());
        drop(driver);
        proxy.join().unwrap();
        assert_eq!(observer.client.query_one("SELECT n FROM rustable_commit_loss.t", &[]).unwrap().get::<_,String>(0),"new");
        assert_eq!(observer.client.query_one("SELECT last_value FROM rustable_commit_loss.probe", &[]).unwrap().get::<_,i64>(0),1);
        observer.client.batch_execute("DROP SCHEMA rustable_commit_loss CASCADE").unwrap();
    }
}
