//! PostgreSQL generated writes use server-native casts from bound text, avoiding
//! lossy numeric conversion or pretending opaque binary values are text.
mod structure;
use super::*;
use ::postgres::GenericClient;
use std::collections::BTreeMap;

#[derive(Clone)]
struct EditColumn {
    name: String,
    cast: String,
    native_oid: u32,
    pk: bool,
    generated: bool,
}
#[derive(Clone)]
struct Table {
    name: String,
    columns: Vec<EditColumn>,
    fingerprint: String,
}

fn native_equality(oid: u32) -> bool {
    matches!(
        oid,
        16 | 17
            | 19
            | 20
            | 21
            | 23
            | 25
            | 26
            | 700
            | 701
            | 1042
            | 1043
            | 1082
            | 1083
            | 1114
            | 1184
            | 1700
            | 2950
            | 3802
    )
}
fn metadata(client: &mut impl GenericClient, input: &str, data: bool) -> Result<Table, String> {
    let row=client.query_opt("SELECT c.oid,n.nspname,c.relname FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE c.oid=pg_catalog.to_regclass($1) AND c.relkind IN ('r','p')",&[&input]).map_err(error)?.ok_or("Only existing PostgreSQL tables can be edited")?;
    let oid: u32 = row.get(0);
    let schema: String = row.get(1);
    let relation: String = row.get(2);
    let name = format!("{}.{}", quote(&schema), quote(&relation));
    let rows=client.query("SELECT a.attname,a.atttypid,a.atttypmod,a.attnotnull,COALESCE(pg_catalog.pg_get_expr(d.adbin,d.adrelid),''), EXISTS(SELECT 1 FROM pg_catalog.pg_index i WHERE i.indrelid=a.attrelid AND i.indisprimary AND a.attnum=ANY(i.indkey)),(a.attgenerated<>'' OR a.attidentity<>''),tn.nspname,t.typname,t.typtype::text,a.attcollation,(WITH RECURSIVE domain_base AS (SELECT t.oid,t.typbasetype UNION ALL SELECT bt.oid,bt.typbasetype FROM pg_catalog.pg_type bt JOIN domain_base d ON bt.oid=d.typbasetype) SELECT oid FROM domain_base WHERE typbasetype=0 LIMIT 1) FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_type t ON t.oid=a.atttypid JOIN pg_catalog.pg_namespace tn ON tn.oid=t.typnamespace LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE a.attrelid=$1 AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum",&[&oid]).map_err(error)?;
    let mut columns = Vec::new();
    let mut signatures = Vec::new();
    for row in rows {
        let column: String = row.get(0);
        let type_oid: u32 = row.get(1);
        let pk: bool = row.get(5);
        let generated: bool = row.get(6);
        let type_kind: &str = row.get(9);
        if data && matches!(type_kind, "p" | "c") {
            return Err(format!(
                "PostgreSQL visual editing does not yet support composite/pseudo column {column}. Use SQL for this table."
            ));
        }
        let cast = format!("{}.{}", quote(row.get(7)), quote(row.get(8)));
        signatures.push((
            column.clone(),
            type_oid,
            row.get::<_, i32>(2),
            row.get::<_, bool>(3),
            row.get::<_, String>(4),
            pk,
            generated,
            cast.clone(),
            row.get::<_, u32>(10),
        ));
        columns.push(EditColumn {
            name: column,
            cast,
            native_oid: row.get(11),
            pk,
            generated,
        });
    }
    if data && !columns.iter().any(|c| c.pk) {
        return Err("PostgreSQL visual editing requires a primary key".into());
    }
    let constraints=client.query("SELECT pg_catalog.pg_get_constraintdef(oid) FROM pg_catalog.pg_constraint WHERE conrelid=$1 ORDER BY oid",&[&oid]).map_err(error)?.into_iter().map(|r|r.get::<_,String>(0)).collect::<Vec<_>>();
    let triggers=client.query("SELECT pg_catalog.pg_get_triggerdef(t.oid),pg_catalog.pg_get_functiondef(t.tgfoid) FROM pg_catalog.pg_trigger t WHERE t.tgrelid=$1 AND NOT t.tgisinternal ORDER BY t.oid",&[&oid]).map_err(error)?.into_iter().map(|r|(r.get::<_,String>(0),r.get::<_,String>(1))).collect::<Vec<_>>();
    let types = type_fingerprints(client, oid)?;
    let indexes=client.query("SELECT pg_catalog.pg_get_indexdef(indexrelid) FROM pg_catalog.pg_index WHERE indrelid=$1 ORDER BY indexrelid",&[&oid]).map_err(error)?.into_iter().map(|r|r.get::<_,String>(0)).collect::<Vec<_>>();
    let fingerprint =
        serde_json::to_string(&(oid, signatures, constraints, triggers, types, indexes))
            .map_err(|e| e.to_string())?;
    Ok(Table {
        name,
        columns,
        fingerprint,
    })
}
fn type_fingerprints(
    client: &mut impl GenericClient,
    relation: u32,
) -> Result<Vec<(u32, String)>, String> {
    let sql = type_catalog_sql(
        "SELECT atttypid FROM pg_catalog.pg_attribute WHERE attrelid=$1 AND attnum>0 AND NOT attisdropped",
    );
    let rows = client.query(&sql, &[&relation]).map_err(error)?;
    Ok(rows
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect())
}
/// Includes nested enum labels, domain constraints and range/array dependencies.
fn type_catalog_sql(seed: &str) -> String {
    format!(
        r#"WITH RECURSIVE dependencies(oid) AS (
        {seed}
        UNION
        SELECT dep.oid FROM dependencies d JOIN pg_catalog.pg_type t ON t.oid=d.oid
        CROSS JOIN LATERAL (VALUES (t.typbasetype),(t.typelem),(COALESCE((SELECT r.rngsubtype FROM pg_catalog.pg_range r WHERE r.rngtypid=t.oid OR r.rngmultitypid=t.oid),0::oid))) AS dep(oid)
        WHERE dep.oid<>0
    ) SELECT t.oid AS type_oid, pg_catalog.jsonb_build_object(
        'type',pg_catalog.to_jsonb(t),'schema',n.nspname,
        'enum',(SELECT pg_catalog.jsonb_agg(e.enumlabel ORDER BY e.enumsortorder) FROM pg_catalog.pg_enum e WHERE e.enumtypid=t.oid),
        'constraints',(SELECT pg_catalog.jsonb_agg(pg_catalog.pg_get_constraintdef(c.oid) ORDER BY c.oid) FROM pg_catalog.pg_constraint c WHERE c.contypid=t.oid),
        'input',pg_catalog.pg_get_functiondef(t.typinput),
        'output',pg_catalog.pg_get_functiondef(t.typoutput)
    )::text AS signature FROM dependencies d JOIN pg_catalog.pg_type t ON t.oid=d.oid JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace ORDER BY t.oid"#
    )
}

fn encoded(value: &Value, column: &EditColumn) -> Result<Value, String> {
    let value = match value {
        Value::Null => return Ok(Value::Null),
        Value::Text(value) => value.clone(),
        Value::Integer(value) => value.to_string(),
        Value::Real(value) => value.to_string(),
        Value::Blob(bytes) if column.native_oid == 17 => {
            use std::fmt::Write;
            let mut text = String::from("\\x");
            for byte in bytes {
                write!(text, "{byte:02x}").unwrap();
            }
            text
        }
        Value::Blob(_) => {
            return Err(format!(
                "Opaque binary value in {} cannot be safely edited",
                column.name
            ));
        }
    };
    if value.contains('\0') {
        return Err(format!(
            "PostgreSQL text cannot contain a NUL byte ({})",
            column.name
        ));
    }
    Ok(Value::Text(value))
}
fn parameter(
    parameters: &mut Vec<Value>,
    value: &Value,
    column: &EditColumn,
) -> Result<String, String> {
    parameters.push(encoded(value, column)?);
    Ok(format!(
        "${}::pg_catalog.text::{}",
        parameters.len(),
        column.cast
    ))
}
fn predicates(
    table: &Table,
    original: &[Value],
    parameters: &mut Vec<Value>,
    primary_only: bool,
) -> Result<String, String> {
    if original.len() != table.columns.len() {
        return Err("Table columns changed. Refresh before editing.".into());
    }
    let mut parts = Vec::new();
    for (column, value) in table.columns.iter().zip(original) {
        if primary_only && !column.pk {
            continue;
        }
        if column.pk && matches!(value, Value::Null) {
            return Err("Original PostgreSQL primary key is NULL; refresh the table".into());
        }
        let bound = parameter(parameters, value, column)?;
        // Complex/custom types may lack an equality operator (json arrays are
        // one example). Compare their server-rendered representation instead.
        let text_equality = !column.pk && !native_equality(column.native_oid);
        let suffix = if text_equality {
            "::pg_catalog.text"
        } else {
            ""
        };
        // Primary keys cannot be NULL. Equality keeps both the locking check
        // and write eligible for an index lookup on large tables.
        let operator = if column.pk {
            "="
        } else {
            "IS NOT DISTINCT FROM"
        };
        parts.push(format!(
            "{}{suffix} {operator} {bound}{suffix}",
            quote(&column.name)
        ));
    }
    Ok(parts.join(" AND "))
}
fn precondition(plan: &mut ChangePlan, table: &Table, original: &[Value]) -> Result<(), String> {
    let mut parameters = Vec::new();
    let predicate = predicates(table, original, &mut parameters, false)?;
    plan.preconditions.push(PlannedPrecondition {
        before_statement: plan.statements.len(),
        sql: format!(
            "SELECT COUNT(*) FROM (SELECT 1 FROM {} WHERE {} FOR UPDATE) AS rustable_original;",
            table.name, predicate
        ),
        parameters,
    });
    Ok(())
}
fn same_row(left: &[Value], right: &[Value]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| {
            a == b || matches!((a,b), (Value::Real(a), Value::Real(b)) if a.is_nan() && b.is_nan())
        })
}
pub(super) fn plan(driver: &mut PostgresDriver, changes: &[Change]) -> Result<ChangePlan, String> {
    driver.check_tunnel()?;
    let _watch = driver.watch();
    let data_count = changes
        .iter()
        .filter(|change| {
            matches!(
                change,
                Change::Cell { .. } | Change::InsertRow { .. } | Change::DeleteRow { .. }
            )
        })
        .count();
    if data_count != changes.len() {
        if data_count != 0 {
            return Err("Save data edits and structure changes separately for PostgreSQL".into());
        }
        return structure::plan(driver, changes);
    }
    let mut tables = BTreeMap::new();
    for change in changes {
        if !tables.contains_key(change.table()) {
            tables.insert(
                change.table().to_owned(),
                metadata(&mut driver.client, change.table(), true)?,
            );
        }
    }
    let mut plan = ChangePlan {
        schemas: tables
            .values()
            .map(|t| (t.name.clone(), t.fingerprint.clone()))
            .collect(),
        ..Default::default()
    };
    type RowEdits = (String, Vec<Value>, Vec<(String, Value)>);
    let mut edits: Vec<RowEdits> = Vec::new();
    for change in changes {
        if let Change::Cell {
            table,
            column,
            original,
            value,
        } = change
        {
            let meta = &tables[table];
            if !meta
                .columns
                .iter()
                .any(|c| c.name == *column && !c.generated)
            {
                return Err("Column is generated or no longer exists".into());
            }
            let index = edits
                .iter()
                .position(|(t, row, _)| t == table && same_row(row, original))
                .unwrap_or_else(|| {
                    edits.push((table.clone(), original.clone(), Vec::new()));
                    edits.len() - 1
                });
            if let Some((_, old)) = edits[index].2.iter_mut().find(|(name, _)| name == column) {
                *old = value.clone();
            } else {
                edits[index].2.push((column.clone(), value.clone()));
            }
        }
    }
    for (table, original, fields) in edits {
        if changes
            .iter()
            .any(|c| matches!(c,Change::DeleteRow{table:t,original:o} if *t==table&&same_row(o, &original)))
        {
            continue;
        }
        let meta = &tables[&table];
        let mut parameters = Vec::new();
        let mut assignments = Vec::new();
        for (name, value) in fields {
            let column = meta.columns.iter().find(|c| c.name == name).unwrap();
            assignments.push(format!(
                "{} = {}",
                quote(&name),
                parameter(&mut parameters, &value, column)?
            ));
        }
        let predicate = predicates(meta, &original, &mut parameters, true)?;
        precondition(&mut plan, meta, &original)?;
        plan.statements.push(PlannedStatement {
            sql: format!(
                "UPDATE {} SET {} WHERE {};",
                meta.name,
                assignments.join(", "),
                predicate
            ),
            parameters,
            expected_rows: Some(1),
        });
    }
    let mut deleted: Vec<(&String, &Vec<Value>)> = Vec::new();
    for change in changes {
        match change {
            Change::DeleteRow { table, original } => {
                if deleted
                    .iter()
                    .any(|(t, row)| *t == table && same_row(row, original))
                {
                    continue;
                }
                deleted.push((table, original));
                let meta = &tables[table];
                let mut parameters = Vec::new();
                let predicate = predicates(meta, original, &mut parameters, true)?;
                precondition(&mut plan, meta, original)?;
                plan.statements.push(PlannedStatement {
                    sql: format!("DELETE FROM {} WHERE {};", meta.name, predicate),
                    parameters,
                    expected_rows: Some(1),
                });
            }
            Change::InsertRow { table, fields, .. } => {
                let meta = &tables[table];
                let mut parameters = Vec::new();
                let mut names = Vec::new();
                let mut values = Vec::new();
                for (name, value) in fields {
                    let column = meta
                        .columns
                        .iter()
                        .find(|c| c.name == *name && !c.generated)
                        .ok_or("Insert column is generated or no longer exists")?;
                    let quoted = quote(name);
                    if names.contains(&quoted) {
                        return Err("Duplicate insert column".into());
                    }
                    names.push(quoted);
                    values.push(parameter(&mut parameters, value, column)?);
                }
                let sql = if fields.is_empty() {
                    format!("INSERT INTO {} DEFAULT VALUES;", meta.name)
                } else {
                    format!(
                        "INSERT INTO {} ({}) VALUES ({});",
                        meta.name,
                        names.join(", "),
                        values.join(", ")
                    )
                };
                plan.statements.push(PlannedStatement {
                    sql,
                    parameters,
                    expected_rows: Some(1),
                });
            }
            _ => {}
        }
    }
    Ok(plan)
}
fn bindings(parameters: &[Value]) -> Result<Vec<Option<String>>, String> {
    parameters
        .iter()
        .map(|value| match value {
            Value::Null => Ok(None),
            Value::Text(text) => Ok(Some(text.clone())),
            _ => Err("Invalid PostgreSQL reviewed parameter encoding; regenerate the plan".into()),
        })
        .collect()
}
pub(super) fn apply(driver: &mut PostgresDriver, plan: &ChangePlan) -> Result<(), String> {
    driver.require_idle_transaction()?;
    driver.check_tunnel()?;
    let _watch = driver.watch();
    if plan.rebuild || plan.schema_version.is_some() {
        return Err("Invalid PostgreSQL change plan".into());
    }
    let mut tx = driver
        .client
        .build_transaction()
        .read_only(false)
        .start()
        .map_err(error)?;
    let schema_plan = plan
        .statements
        .iter()
        .all(|statement| statement.expected_rows.is_none());
    let mut schemas = plan.schemas.iter().collect::<Vec<_>>();
    schemas.sort_by(|a, b| a.0.cmp(&b.0));
    for (table, expected) in schemas {
        let current = metadata(&mut tx, table, !schema_plan)?;
        tx.batch_execute(&format!(
            "LOCK TABLE {} IN {} MODE",
            current.name,
            if schema_plan {
                "ACCESS EXCLUSIVE"
            } else {
                "ROW EXCLUSIVE"
            }
        ))
        .map_err(error)?;
        if metadata(&mut tx, table, !schema_plan)?.fingerprint != *expected {
            return Err("Table structure changed since review. Nothing was saved; refresh and review again.".into());
        }
    }
    for (index, statement) in plan.statements.iter().enumerate() {
        if driver.cancel.load(Ordering::Relaxed) {
            return Err("Cancelled. Nothing was saved.".into());
        }
        for check in plan
            .preconditions
            .iter()
            .filter(|check| check.before_statement == index)
        {
            let values = bindings(&check.parameters)?;
            let parameters = values
                .iter()
                .map(|v| v as &(dyn ToSql + Sync))
                .collect::<Vec<_>>();
            let count: i64 = tx.query_one(&check.sql, &parameters).map_err(error)?.get(0);
            if count != 1 {
                return Err(if schema_plan {"A type definition changed since review. Nothing was saved; review the structure changes again."} else {"A row changed since it was loaded. Nothing was saved; pending changes are retained."}.into());
            }
        }
        let values = bindings(&statement.parameters)?;
        let parameters = values
            .iter()
            .map(|v| v as &(dyn ToSql + Sync))
            .collect::<Vec<_>>();
        let count = tx.execute(&statement.sql, &parameters).map_err(error)?;
        if statement
            .expected_rows
            .is_some_and(|expected| count != expected as u64)
        {
            return Err("A write did not affect exactly one row. Nothing was saved; pending changes are retained.".into());
        }
    }
    if driver.cancel.load(Ordering::Relaxed) {
        return Err("Cancelled. Nothing was saved.".into());
    }
    tx.commit().map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binary_is_never_guessed_and_native_equality_is_explicit() {
        assert!(!native_equality(1007));
        assert!(!native_equality(114));
        assert!(native_equality(3802));
        let column = EditColumn {
            name: "b".into(),
            cast: "\"pg_catalog\".\"bytea\"".into(),
            native_oid: 17,
            pk: false,
            generated: false,
        };
        assert_eq!(
            encoded(&Value::Blob(vec![0, 255]), &column).unwrap(),
            Value::Text("\\x00ff".into())
        );
        let text = EditColumn {
            cast: "\"pg_catalog\".\"text\"".into(),
            native_oid: 25,
            ..column
        };
        assert!(encoded(&Value::Blob(vec![0, 255]), &text).is_err());
    }
    fn fixture() -> PostgresDriver {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        PostgresDriver::open(
            &crate::connection::PostgresConfig {
                host: "127.0.0.1".into(),
                port: 54329,
                database: "rustable_qa".into(),
                user: "postgres".into(),
                password: "rustable_qa_only".into(),
                tls: false,
                ssh: None,
            },
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    }
    fn row(driver: &mut PostgresDriver, sql: &str) -> Vec<Value> {
        decode_row(&driver.client.query_one(sql, &[]).unwrap()).unwrap()
    }
    fn cell(table: &str, column: &str, original: &[Value], value: Value) -> Change {
        Change::Cell {
            table: table.into(),
            column: column.into(),
            original: original.to_vec(),
            value,
        }
    }
    #[test]
    #[ignore = "requires disposable PostgreSQL QA on localhost54329 with RUSTABLE_POSTGRES_QA=1"]
    fn live_postgres_generated_writes_preserve_types_and_atomicity() {
        let mut driver = fixture();
        driver.client.batch_execute(r#"DROP SCHEMA IF EXISTS "rustable.edit.qa" CASCADE;
        CREATE SCHEMA "rustable.edit.qa";
        CREATE TABLE "rustable.edit.qa"."typed.table" (id integer PRIMARY KEY, name text NOT NULL DEFAULT 'default', amount numeric(30,8), bytes bytea, yes boolean, day date, stamp timestamp, zoned timestamptz, clock time, uid uuid, j json, jb jsonb, document xml, f real, d double precision);
        INSERT INTO "rustable.edit.qa"."typed.table" VALUES(1,'before',12345678901234567890.12345678,decode('00ff','hex'),true,'2024-02-29','2024-02-29 12:34:56.123456','2024-02-29 12:34:56.123456+03','12:34:56.123456','00000000-0000-0000-0000-000000000001','{ "x": 1 }','{"x":1}','<x>hello</x>','NaN'::real,2.5);
        INSERT INTO "rustable.edit.qa"."typed.table"(id,name) VALUES(2,'second');"#).unwrap();
        let table = "\"rustable.edit.qa\".\"typed.table\"";
        let original = row(&mut driver, &format!("SELECT * FROM {table} WHERE id=1"));
        let reviewed = plan(
            &mut driver,
            &[
                cell(
                    table,
                    "name",
                    &original,
                    Value::Text("O'Reilly \\ exact".into()),
                ),
                cell(
                    table,
                    "amount",
                    &original,
                    Value::Text("98765432109876543210.87654321".into()),
                ),
                cell(table, "bytes", &original, Value::Blob(vec![0, 1, 255])),
                cell(table, "day", &original, Value::Text("2025-03-01".into())),
                cell(
                    table,
                    "uid",
                    &original,
                    Value::Text("ffffffff-ffff-ffff-ffff-ffffffffffff".into()),
                ),
            ],
        )
        .unwrap();
        assert_eq!(reviewed.statements.len(), 1);
        let where_sql = reviewed.statements[0].sql.split(" WHERE ").nth(1).unwrap();
        assert!(where_sql.contains("\"id\""));
        assert!(!where_sql.contains("name"));
        assert!(reviewed.preconditions[0].sql.contains("FOR UPDATE"));
        apply(&mut driver, &reviewed).unwrap();
        let updated = row(&mut driver, &format!("SELECT * FROM {table} WHERE id=1"));
        assert_eq!(updated[1], Value::Text("O'Reilly \\ exact".into()));
        assert_eq!(
            updated[2],
            Value::Text("98765432109876543210.87654321".into())
        );
        assert_eq!(updated[3], Value::Blob(vec![0, 1, 255]));
        assert_eq!(updated[5], Value::Text("2025-03-01".into()));
        // Full-row optimistic guards successfully bound every untouched native
        // value, including JSON/XML which lack native equality operators.
        for index in [4, 6, 7, 8, 10, 11, 12, 13, 14] {
            assert!(same_row(&updated[index..=index], &original[index..=index]));
        }
        // A stale later row rolls back an earlier valid write in the same save.
        let second = row(&mut driver, &format!("SELECT * FROM {table} WHERE id=2"));
        let stale = plan(
            &mut driver,
            &[
                cell(table, "name", &updated, Value::Text("must rollback".into())),
                cell(table, "name", &second, Value::Text("stale".into())),
            ],
        )
        .unwrap();
        let mut other = fixture();
        other
            .client
            .batch_execute(&format!("UPDATE {table} SET name='concurrent' WHERE id=2"))
            .unwrap();
        assert!(
            apply(&mut driver, &stale)
                .unwrap_err()
                .contains("row changed")
        );
        assert_eq!(
            row(&mut driver, &format!("SELECT name FROM {table} WHERE id=1"))[0],
            updated[1]
        );
        // Invalid casts also roll back all preceding writes, preserving precision.
        let invalid = plan(
            &mut driver,
            &[
                Change::InsertRow {
                    id: 1,
                    table: table.into(),
                    fields: vec![("id".into(), Value::Integer(3))],
                },
                Change::InsertRow {
                    id: 2,
                    table: table.into(),
                    fields: vec![
                        ("id".into(), Value::Integer(4)),
                        ("uid".into(), Value::Text("not-a-uuid".into())),
                    ],
                },
            ],
        )
        .unwrap();
        assert!(apply(&mut driver, &invalid).is_err());
        assert_eq!(
            driver
                .client
                .query_one(
                    &format!("SELECT COUNT(*) FROM {table} WHERE id IN (3,4)"),
                    &[]
                )
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        let insert = plan(
            &mut driver,
            &[Change::InsertRow {
                id: 3,
                table: table.into(),
                fields: vec![
                    ("id".into(), Value::Integer(3)),
                    ("amount".into(), Value::Null),
                ],
            }],
        )
        .unwrap();
        apply(&mut driver, &insert).unwrap();
        let inserted = row(&mut driver, &format!("SELECT * FROM {table} WHERE id=3"));
        assert_eq!(inserted[1], Value::Text("default".into()));
        assert_eq!(inserted[2], Value::Null);
        let delete = plan(
            &mut driver,
            &[Change::DeleteRow {
                table: table.into(),
                original: inserted,
            }],
        )
        .unwrap();
        apply(&mut driver, &delete).unwrap();
        // Composite keys use original values even while a key itself is edited.
        driver.client.batch_execute(r#"CREATE TABLE "rustable.edit.qa".composite(a text,b int,n text,PRIMARY KEY(a,b));INSERT INTO "rustable.edit.qa".composite VALUES('old',7,'before');"#).unwrap();
        let composite = "\"rustable.edit.qa\".composite";
        let original = row(&mut driver, &format!("SELECT * FROM {composite}"));
        let key = plan(
            &mut driver,
            &[
                cell(composite, "a", &original, Value::Text("new".into())),
                cell(composite, "n", &original, Value::Text("after".into())),
            ],
        )
        .unwrap();
        apply(&mut driver, &key).unwrap();
        assert_eq!(
            row(&mut driver, &format!("SELECT * FROM {composite}")),
            vec![
                Value::Text("new".into()),
                Value::Integer(7),
                Value::Text("after".into())
            ]
        );
        // Review is invalidated by changed structure, not silently reinterpreted.
        let fresh = row(&mut driver, &format!("SELECT * FROM {table} WHERE id=1"));
        let changed = plan(
            &mut driver,
            &[cell(table, "name", &fresh, Value::Text("invalid".into()))],
        )
        .unwrap();
        driver
            .client
            .batch_execute(&format!("ALTER TABLE {table} ADD COLUMN extra text"))
            .unwrap();
        assert!(
            apply(&mut driver, &changed)
                .unwrap_err()
                .contains("structure changed")
        );
        driver.client.batch_execute(r#"CREATE TYPE "rustable.edit.qa".pair AS (a int,b text);CREATE TABLE "rustable.edit.qa".unsupported(id int PRIMARY KEY,a "rustable.edit.qa".pair);INSERT INTO "rustable.edit.qa".unsupported VALUES(1,ROW(1,'two'));"#).unwrap();
        assert!(
            plan(
                &mut driver,
                &[Change::InsertRow {
                    id: 4,
                    table: "\"rustable.edit.qa\".unsupported".into(),
                    fields: vec![("id".into(), Value::Integer(2))]
                }]
            )
            .unwrap_err()
            .contains("does not yet support")
        );
        driver
            .client
            .batch_execute("DROP SCHEMA \"rustable.edit.qa\" CASCADE")
            .unwrap();
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    #[test]
    #[ignore = "requires disposable PostgreSQL QA on localhost54329 with RUSTABLE_POSTGRES_QA=1"]
    fn live_postgres_triggers_rowcount_and_cancellation_rollback() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let config = crate::connection::PostgresConfig {
            host: "127.0.0.1".into(),
            port: 54329,
            database: "rustable_qa".into(),
            user: "postgres".into(),
            password: "rustable_qa_only".into(),
            tls: false,
            ssh: None,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut driver = PostgresDriver::open(&config, cancel.clone()).unwrap();
        driver.client.batch_execute(r#"DROP SCHEMA IF EXISTS rustable_edit_safety CASCADE;CREATE SCHEMA rustable_edit_safety;
        CREATE TABLE rustable_edit_safety.t(id int PRIMARY KEY,n text);INSERT INTO rustable_edit_safety.t VALUES(1,'one'),(2,'two');
        CREATE FUNCTION rustable_edit_safety.touch_other() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.id=1 THEN UPDATE rustable_edit_safety.t SET n='trigger changed' WHERE id=2; END IF; RETURN NEW; END $$;
        CREATE TRIGGER touch_other AFTER UPDATE ON rustable_edit_safety.t FOR EACH ROW EXECUTE FUNCTION rustable_edit_safety.touch_other();"#).unwrap();
        let original = driver
            .client
            .query("SELECT * FROM rustable_edit_safety.t ORDER BY id", &[])
            .unwrap()
            .iter()
            .map(decode_row)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let changes = original
            .iter()
            .map(|row| Change::Cell {
                table: "rustable_edit_safety.t".into(),
                column: "n".into(),
                original: row.clone(),
                value: Value::Text("edited".into()),
            })
            .collect::<Vec<_>>();
        let reviewed = plan(&mut driver, &changes).unwrap();
        assert!(
            apply(&mut driver, &reviewed)
                .unwrap_err()
                .contains("row changed")
        );
        assert_eq!(
            driver
                .client
                .query("SELECT * FROM rustable_edit_safety.t ORDER BY id", &[])
                .unwrap()
                .iter()
                .map(decode_row)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            original
        );
        driver.client.batch_execute("DROP TRIGGER touch_other ON rustable_edit_safety.t;CREATE FUNCTION rustable_edit_safety.suppress() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$;CREATE TRIGGER suppress BEFORE UPDATE ON rustable_edit_safety.t FOR EACH ROW EXECUTE FUNCTION rustable_edit_safety.suppress();").unwrap();
        let suppressed = plan(&mut driver, &changes[..1]).unwrap();
        assert!(
            apply(&mut driver, &suppressed)
                .unwrap_err()
                .contains("exactly one row")
        );
        driver
            .client
            .batch_execute("DROP TRIGGER suppress ON rustable_edit_safety.t")
            .unwrap();
        let reviewed = plan(&mut driver, &changes[..1]).unwrap();
        let mut other = PostgresDriver::open(&config, Arc::new(AtomicBool::new(false))).unwrap();
        other
            .client
            .batch_execute("BEGIN;UPDATE rustable_edit_safety.t SET n=n WHERE id=1")
            .unwrap();
        let signal = cancel.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(120));
            signal.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let result = apply(&mut driver, &reviewed);
        canceller.join().unwrap();
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        cancel.store(false, Ordering::Relaxed);
        other.client.batch_execute("ROLLBACK").unwrap();
        assert_eq!(
            driver
                .client
                .query_one("SELECT n FROM rustable_edit_safety.t WHERE id=1", &[])
                .unwrap()
                .get::<_, String>(0),
            "one"
        );
        driver
            .client
            .batch_execute("DROP SCHEMA rustable_edit_safety CASCADE")
            .unwrap();
    }
}

#[cfg(test)]
mod complex_type_tests {
    use super::*;
    #[test]
    #[ignore = "requires disposable PostgreSQL QA on localhost54329 with RUSTABLE_POSTGRES_QA=1"]
    fn live_complex_type_casts_domains_arrays_and_review_invalidation() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let config = crate::connection::PostgresConfig {
            host: "127.0.0.1".into(),
            port: 54329,
            database: "rustable_qa".into(),
            user: "postgres".into(),
            password: "rustable_qa_only".into(),
            tls: false,
            ssh: None,
        };
        let mut driver = PostgresDriver::open(&config, Arc::new(AtomicBool::new(false))).unwrap();
        driver.client.batch_execute(r#"DROP SCHEMA IF EXISTS "rustable.type.qa" CASCADE;CREATE SCHEMA "rustable.type.qa";
        CREATE TYPE "rustable.type.qa"."status.with""quote" AS ENUM ('new','ready','quoted '' label');
        CREATE DOMAIN "rustable.type.qa".positive AS numeric(30,8) CHECK(VALUE>0);
        CREATE DOMAIN "rustable.type.qa".binary_domain AS bytea CHECK(octet_length(VALUE)>0);
        CREATE TABLE "rustable.type.qa".t(id integer PRIMARY KEY,n text, mood "rustable.type.qa"."status.with""quote", tags text[], matrix integer[], amount "rustable.type.qa".positive, payload "rustable.type.qa".binary_domain, elapsed interval, span int4range, time_zone timetz, document json[], moods "rustable.type.qa"."status.with""quote"[]);
        INSERT INTO "rustable.type.qa".t VALUES(1,'before','new',ARRAY['a,b','quote"','back\slash',NULL,'NULL','日本'],'[0:1][2:3]={{1,NULL},{3,4}}',12345678901234567890.12345678,decode('00ff','hex'),'2 days 03:04:05.123456','[1,5)','12:34:56+03',ARRAY['{"x":1}'::json,NULL],ARRAY['new'::"rustable.type.qa"."status.with""quote"]);"#).unwrap();
        let table = "\"rustable.type.qa\".t";
        let browse = Browse {
            table: table.into(),
            ..Default::default()
        };
        let original = driver.browse(&browse).unwrap().grid.rows[0].clone();
        assert!(
            matches!(&original[3],Value::Text(text) if text.contains("日本")&&text.contains("NULL"))
        );
        assert!(matches!(&original[4],Value::Text(text) if text.starts_with("[0:1][2:3]=")));
        assert_eq!(original[6], Value::Blob(vec![0, 255]));
        let edit = |column: &str, value: Value| Change::Cell {
            table: table.into(),
            column: column.into(),
            original: original.clone(),
            value,
        };
        let reviewed = plan(
            &mut driver,
            &[
                edit("n", Value::Text("after".into())),
                edit("mood", Value::Text("quoted ' label".into())),
                edit(
                    "elapsed",
                    Value::Text("1 year 2 mons 3 days 04:05:06.123456".into()),
                ),
                edit(
                    "amount",
                    Value::Text("98765432109876543210.87654321".into()),
                ),
                edit("payload", Value::Blob(vec![1, 0, 255])),
                // Re-submit exact server output: dimensions, escaped strings and
                // SQL NULL elements must survive without client-side parsing.
                edit("tags", original[3].clone()),
                edit("matrix", Value::Text("[0:1][2:3]={{9,NULL},{3,4}}".into())),
                edit("span", Value::Text("[10,20)".into())),
                edit("moods", Value::Text("{ready,NULL}".into())),
            ],
        )
        .unwrap();
        assert!(
            reviewed.statements[0]
                .sql
                .contains("\"rustable.type.qa\".\"status.with\"\"quote\"")
        );
        apply(&mut driver, &reviewed).unwrap();
        let updated = driver.browse(&browse).unwrap().grid.rows[0].clone();
        assert_eq!(updated[2], Value::Text("quoted ' label".into()));
        assert_eq!(updated[3], original[3]);
        assert_eq!(
            updated[4],
            Value::Text("[0:1][2:3]={{9,NULL},{3,4}}".into())
        );
        assert_eq!(
            updated[5],
            Value::Text("98765432109876543210.87654321".into())
        );
        assert_eq!(updated[6], Value::Blob(vec![1, 0, 255]));
        assert_eq!(updated[9], original[9]);
        assert_eq!(updated[10], original[10]);
        assert_eq!(updated[8], Value::Text("[10,20)".into()));
        assert_eq!(updated[11], Value::Text("{ready,NULL}".into()));
        // Invalid domain input rolls back a preceding valid insert.
        let invalid = plan(
            &mut driver,
            &[
                Change::InsertRow {
                    id: 1,
                    table: table.into(),
                    fields: vec![
                        ("id".into(), Value::Integer(2)),
                        ("amount".into(), Value::Text("1.25".into())),
                    ],
                },
                Change::InsertRow {
                    id: 2,
                    table: table.into(),
                    fields: vec![
                        ("id".into(), Value::Integer(3)),
                        ("amount".into(), Value::Text("-1".into())),
                    ],
                },
            ],
        )
        .unwrap();
        assert!(apply(&mut driver, &invalid).is_err());
        assert_eq!(
            driver
                .client
                .query_one("SELECT COUNT(*) FROM \"rustable.type.qa\".t", &[])
                .unwrap()
                .get::<_, i64>(0),
            1
        );
        // Both enum labels and constraints of a domain are part of review.
        let pending = || Change::Cell {
            table: table.into(),
            column: "n".into(),
            original: updated.clone(),
            value: Value::Text("should not save".into()),
        };
        let enum_changed = plan(&mut driver, &[pending()]).unwrap();
        driver
            .client
            .batch_execute(
                "ALTER TYPE \"rustable.type.qa\".\"status.with\"\"quote\" ADD VALUE 'later'",
            )
            .unwrap();
        assert!(
            apply(&mut driver, &enum_changed)
                .unwrap_err()
                .contains("structure changed")
        );
        let domain_changed = plan(&mut driver, &[pending()]).unwrap();
        driver.client.batch_execute("ALTER DOMAIN \"rustable.type.qa\".positive ADD CONSTRAINT positive_again CHECK(VALUE>0)").unwrap();
        assert!(
            apply(&mut driver, &domain_changed)
                .unwrap_err()
                .contains("structure changed")
        );
        // An enum nested only in an array is included recursively too.
        driver.client.batch_execute("CREATE TABLE \"rustable.type.qa\".nested(id int PRIMARY KEY, moods \"rustable.type.qa\".\"status.with\"\"quote\"[]); INSERT INTO \"rustable.type.qa\".nested VALUES(1,ARRAY['new'::\"rustable.type.qa\".\"status.with\"\"quote\"])").unwrap();
        let nested = Browse {
            table: "\"rustable.type.qa\".nested".into(),
            ..Default::default()
        };
        let row = driver.browse(&nested).unwrap().grid.rows[0].clone();
        let nested_plan = plan(
            &mut driver,
            &[Change::DeleteRow {
                table: nested.table,
                original: row,
            }],
        )
        .unwrap();
        driver
            .client
            .batch_execute(
                "ALTER TYPE \"rustable.type.qa\".\"status.with\"\"quote\" ADD VALUE 'latest'",
            )
            .unwrap();
        assert!(
            apply(&mut driver, &nested_plan)
                .unwrap_err()
                .contains("structure changed")
        );
        driver
            .client
            .batch_execute("DROP SCHEMA \"rustable.type.qa\" CASCADE")
            .unwrap();
    }
}
