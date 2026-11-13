//! Transactional PostgreSQL DDL. User type text is parsed as a restricted type
//! grammar, resolved through pg_type, and reconstructed from quoted identifiers.
use super::*;
use std::sync::OnceLock;

struct TypeSpec {
    sql: String,
    native_oid: u32,
    guard_sql: String,
    signature: String,
}
fn identifier(name: &str) -> Result<String, String> {
    if name.is_empty() || name.contains('\0') || name.len() > 63 {
        return Err("PostgreSQL identifiers must contain 1–63 bytes and no NUL characters".into());
    }
    Ok(quote(name))
}
fn type_parts(input: &str) -> Result<(String, String, String), String> {
    static GRAMMAR: OnceLock<regex::Regex> = OnceLock::new();
    let regex=GRAMMAR.get_or_init(||regex::Regex::new(r#"(?ix)^\s*(?P<name>(?:"(?:[^"]|"")+"|[\pL_][\pL\pN_$]*)(?:\s*\.\s*(?:"(?:[^"]|"")+"|[\pL_][\pL\pN_$]*))?|double\s+precision|character\s+varying|bit\s+varying)(?P<modifier>\s*\(\s*[+-]?\d+\s*(?:,\s*[+-]?\d+\s*)?\))?(?P<zone>\s+(?:with|without)\s+time\s+zone)?(?P<arrays>(?:\s*\[\s*\])*)\s*$"#).unwrap());
    if input.len() > 512 || input.contains('\0') {
        return Err("Invalid PostgreSQL type declaration".into());
    }
    let parts = regex
        .captures(input)
        .ok_or("Enter a PostgreSQL type name with optional numeric modifiers and [] dimensions")?;
    let name = format!(
        "{}{}",
        &parts["name"],
        parts.name("zone").map_or("", |value| value.as_str())
    );
    let modifier = if let Some(value) = parts.name("modifier") {
        let value = value.as_str().trim();
        let numbers = value[1..value.len() - 1]
            .split(',')
            .map(|part| {
                part.trim()
                    .parse::<i32>()
                    .map(|n| n.to_string())
                    .map_err(|_| "Type modifier is out of range".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        format!("({})", numbers.join(","))
    } else {
        String::new()
    };
    let arrays = "[]".repeat(parts["arrays"].bytes().filter(|byte| *byte == b'[').count());
    Ok((name, modifier, arrays))
}
fn resolve_type(client: &mut Client, input: &str) -> Result<TypeSpec, String> {
    let (name, modifier, arrays) = type_parts(input)?;
    let row=client.query_opt("SELECT t.oid,n.nspname,t.typname,t.typtype::text,(WITH RECURSIVE base AS (SELECT t.oid,t.typbasetype UNION ALL SELECT b.oid,b.typbasetype FROM pg_catalog.pg_type b JOIN base ON b.oid=base.typbasetype) SELECT oid FROM base WHERE typbasetype=0 LIMIT 1) FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace WHERE t.oid=pg_catalog.to_regtype($1)",&[&name]).map_err(error)?.ok_or("PostgreSQL type does not exist")?;
    let oid: u32 = row.get(0);
    let kind: &str = row.get(3);
    if kind == "p" {
        return Err("PostgreSQL pseudo-types cannot be used as table columns".into());
    }
    let sql = format!(
        "{}.{}{modifier}{arrays}",
        quote(row.get(1)),
        quote(row.get(2))
    );
    // Prepare only: validate server type modifiers/cast syntax without running SQL.
    client
        .prepare(&format!("SELECT NULL::{sql}"))
        .map_err(error)?;
    let types = type_catalog_sql(&format!("SELECT {oid}::oid"));
    let guard_sql = format!(
        "SELECT COALESCE(pg_catalog.jsonb_agg(pg_catalog.jsonb_build_array(type_oid,signature) ORDER BY type_oid),'[]'::jsonb)::text AS signature FROM ({types}) AS type_definitions"
    );
    let signature: String = client.query_one(&guard_sql, &[]).map_err(error)?.get(0);
    Ok(TypeSpec {
        sql,
        native_oid: if arrays.is_empty() { row.get(4) } else { 0 },
        guard_sql,
        signature,
    })
}
fn constant(value: &Value, ty: &TypeSpec) -> Result<String, String> {
    let column = EditColumn {
        name: "default".into(),
        cast: ty.sql.clone(),
        native_oid: ty.native_oid,
        pk: false,
        generated: false,
    };
    let encoded = encoded(value, &column)?;
    let literal = match encoded {
        Value::Null => "NULL".into(),
        Value::Text(text) => format!("E'{}'", text.replace('\\', "\\\\").replace('\'', "''")),
        _ => unreachable!(),
    };
    Ok(format!("{literal}::{}", ty.sql))
}
fn push(plan: &mut ChangePlan, sql: String) {
    plan.statements.push(PlannedStatement {
        sql,
        parameters: Vec::new(),
        expected_rows: None,
    });
}
fn guard(plan: &mut ChangePlan, ty: &TypeSpec) {
    plan.preconditions.push(PlannedPrecondition {
        before_statement: 0,
        sql: format!(
            "SELECT COUNT(*) FROM ({}) AS reviewed_type WHERE signature=$1::pg_catalog.text;",
            ty.guard_sql
        ),
        parameters: vec![Value::Text(ty.signature.clone())],
    });
}
pub(super) fn plan(driver: &mut PostgresDriver, changes: &[Change]) -> Result<ChangePlan, String> {
    let mut tables = BTreeMap::new();
    for change in changes {
        if !tables.contains_key(change.table()) {
            tables.insert(
                change.table().to_owned(),
                metadata(&mut driver.client, change.table(), false)?,
            );
        }
    }
    let mut plan = ChangePlan {
        schemas: tables
            .values()
            .map(|table| (table.name.clone(), table.fingerprint.clone()))
            .collect(),
        ..Default::default()
    };
    // UI stages edits under the original name; rename only after other actions.
    let ordered = changes
        .iter()
        .filter(|c| !matches!(c, Change::RenameColumn { .. }))
        .chain(
            changes
                .iter()
                .filter(|c| matches!(c, Change::RenameColumn { .. })),
        );
    for change in ordered {
        let table = tables.get_mut(change.table()).unwrap();
        match change {
            Change::RenameColumn { from, to, .. } => {
                let target = identifier(to)?;
                if table.columns.iter().any(|column| column.name == *to) {
                    return Err("A column with that name already exists".into());
                }
                let column = table
                    .columns
                    .iter_mut()
                    .find(|column| column.name == *from)
                    .ok_or("Column no longer exists")?;
                push(
                    &mut plan,
                    format!(
                        "ALTER TABLE {} RENAME COLUMN {} TO {target};",
                        table.name,
                        quote(from)
                    ),
                );
                column.name = to.clone();
            }
            Change::AddColumn { definition, .. } => {
                let name = identifier(&definition.name)?;
                if table
                    .columns
                    .iter()
                    .any(|column| column.name == definition.name)
                {
                    return Err("A column with that name already exists".into());
                }
                let ty = resolve_type(&mut driver.client, &definition.data_type)?;
                guard(&mut plan, &ty);
                let default = definition
                    .default
                    .as_ref()
                    .map(|value| constant(value, &ty).map(|literal| format!(" DEFAULT {literal}")))
                    .transpose()?
                    .unwrap_or_default();
                let nullable = if definition.not_null { " NOT NULL" } else { "" };
                push(
                    &mut plan,
                    format!(
                        "ALTER TABLE {} ADD COLUMN {name} {}{default}{nullable};",
                        table.name, ty.sql
                    ),
                );
                table.columns.push(EditColumn {
                    name: definition.name.clone(),
                    cast: ty.sql,
                    native_oid: ty.native_oid,
                    pk: false,
                    generated: false,
                });
            }
            Change::AlterColumn {
                column,
                data_type,
                not_null,
                default,
                ..
            } => {
                let current = table
                    .columns
                    .iter_mut()
                    .find(|entry| entry.name == *column)
                    .ok_or("Column no longer exists")?;
                if current.generated && (data_type.is_some() || default.is_some()) {
                    return Err("Changing generated/identity column types or defaults is not supported in the structure editor yet".into());
                }
                let column = quote(column);
                if default.is_some() {
                    push(
                        &mut plan,
                        format!(
                            "ALTER TABLE {} ALTER COLUMN {column} DROP DEFAULT;",
                            table.name
                        ),
                    );
                }
                if let Some(data_type) = data_type {
                    let ty = resolve_type(&mut driver.client, data_type)?;
                    guard(&mut plan, &ty);
                    push(
                        &mut plan,
                        format!(
                            "ALTER TABLE {} ALTER COLUMN {column} TYPE {};",
                            table.name, ty.sql
                        ),
                    );
                    current.cast = ty.sql;
                    current.native_oid = ty.native_oid;
                }
                if let Some(Some(value)) = default {
                    let ty = TypeSpec {
                        sql: current.cast.clone(),
                        native_oid: current.native_oid,
                        guard_sql: String::new(),
                        signature: String::new(),
                    };
                    push(
                        &mut plan,
                        format!(
                            "ALTER TABLE {} ALTER COLUMN {column} SET DEFAULT {};",
                            table.name,
                            constant(value, &ty)?
                        ),
                    );
                }
                if let Some(not_null) = not_null {
                    push(
                        &mut plan,
                        format!(
                            "ALTER TABLE {} ALTER COLUMN {column} {} NOT NULL;",
                            table.name,
                            if *not_null { "SET" } else { "DROP" }
                        ),
                    );
                }
            }
            Change::CreateIndex {
                name,
                columns,
                unique,
                ..
            } => {
                let name = identifier(name)?;
                if columns.is_empty() {
                    return Err("Select at least one index column".into());
                }
                let mut names = Vec::new();
                for (name, descending) in columns {
                    if !table.columns.iter().any(|column| column.name == *name) {
                        return Err("Index column no longer exists".into());
                    }
                    names.push(format!(
                        "{} {}",
                        quote(name),
                        if *descending { "DESC" } else { "ASC" }
                    ));
                }
                push(
                    &mut plan,
                    format!(
                        "CREATE {}INDEX {name} ON {} ({});",
                        if *unique { "UNIQUE " } else { "" },
                        table.name,
                        names.join(", ")
                    ),
                );
            }
            Change::DropIndex { name, .. } => {
                let row=driver.client.query_opt("SELECT n.nspname,c.relname,EXISTS(SELECT 1 FROM pg_catalog.pg_constraint con WHERE con.conindid=c.oid) FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c ON c.oid=i.indexrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE i.indrelid=pg_catalog.to_regclass($1) AND c.relname=$2",&[&table.name,name]).map_err(error)?.ok_or("Index does not belong to this table or no longer exists")?;
                if row.get::<_, bool>(2) {
                    return Err("This index owns a primary-key/unique constraint; change the constraint explicitly in SQL".into());
                }
                push(
                    &mut plan,
                    format!("DROP INDEX {}.{};", quote(row.get(0)), quote(row.get(1))),
                );
            }
            _ => {
                return Err(
                    "Save data edits and structure changes separately for PostgreSQL".into(),
                );
            }
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn type_grammar_cannot_escape_ddl() {
        for input in [
            "integer; DROP TABLE x",
            "integer DEFAULT 1",
            "text USING evil()",
            "int -- comment",
            "numeric(2+2)",
            "text COLLATE x",
            "int) ; SELECT 1",
            "record",
            "pg_catalog.int4 WHERE true",
        ] {
            if input != "record" {
                assert!(type_parts(input).is_err(), "{input}");
            }
        }
        assert_eq!(
            type_parts("timestamp(6) with time zone[]").unwrap(),
            ("timestamp with time zone".into(), "(6)".into(), "[]".into())
        );
        assert!(type_parts("\"a.b\".\"type; with\"\"quote\"[]").is_ok());
        assert!(type_parts("numeric(30, -2)").is_ok());
        assert!(type_parts("double precision").is_ok());
        assert!(type_parts("character varying(42)").is_ok());
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    #[test]
    #[ignore = "requires disposable PostgreSQL QA on localhost54329 with RUSTABLE_POSTGRES_QA=1"]
    fn transactional_structure_changes_no_pk_composites_and_guards() {
        assert_eq!(std::env::var("RUSTABLE_POSTGRES_QA").as_deref(), Ok("1"));
        let config = crate::connection::PostgresConfig {
            host: "127.0.0.1".into(),
            port: 54329,
            database: "rustable_qa".into(),
            user: "postgres".into(),
            password: "rustable_qa_only".into(),
            tls: crate::connection::PostgresTlsMode::Disable,
            ssh: None,
        };
        let mut driver = PostgresDriver::open(&config, Arc::new(AtomicBool::new(false))).unwrap();
        driver.client.batch_execute(r#"DROP SCHEMA IF EXISTS "rustable.structure.qa" CASCADE;CREATE SCHEMA "rustable.structure.qa";
        CREATE TYPE "rustable.structure.qa".pair AS (a int,b text);
        CREATE TYPE "rustable.structure.qa"."Mood.Type" AS ENUM('new','ready');
        CREATE DOMAIN "rustable.structure.qa".mood_domain AS "rustable.structure.qa"."Mood.Type";
        CREATE TABLE "rustable.structure.qa".t(n text,count integer,complex "rustable.structure.qa".pair);
        INSERT INTO "rustable.structure.qa".t VALUES('hello',5,ROW(1,'pair'));
        CREATE TABLE "rustable.structure.qa".keyed(id int PRIMARY KEY);"#).unwrap();
        let table = "\"rustable.structure.qa\".t";
        let changes = [
            Change::RenameColumn {
                table: table.into(),
                from: "n".into(),
                to: "ti\"tle".into(),
            },
            Change::AlterColumn {
                table: table.into(),
                column: "n".into(),
                data_type: Some("varchar(20)".into()),
                not_null: Some(true),
                default: Some(Some(Value::Text("O'Reilly\\path".into()))),
            },
            Change::AlterColumn {
                table: table.into(),
                column: "count".into(),
                data_type: Some("bigint".into()),
                not_null: Some(false),
                default: Some(Some(Value::Integer(9))),
            },
            Change::AddColumn {
                table: table.into(),
                definition: ColumnDefinition {
                    name: "amount".into(),
                    data_type: "numeric(12,2)".into(),
                    not_null: true,
                    default: Some(Value::Text("123.45".into())),
                },
            },
            Change::AddColumn {
                table: table.into(),
                definition: ColumnDefinition {
                    name: "state".into(),
                    data_type: "\"rustable.structure.qa\".\"Mood.Type\"".into(),
                    not_null: false,
                    default: Some(Value::Text("new".into())),
                },
            },
            Change::AddColumn {
                table: table.into(),
                definition: ColumnDefinition {
                    name: "amounts".into(),
                    data_type: "numeric(12,2)[]".into(),
                    not_null: false,
                    default: Some(Value::Text("{1.25,NULL}".into())),
                },
            },
            Change::CreateIndex {
                table: table.into(),
                name: "index.with\"quote".into(),
                columns: vec![("n".into(), true)],
                unique: false,
            },
        ];
        let reviewed = super::super::plan(&mut driver, &changes).unwrap();
        assert!(
            reviewed
                .statements
                .iter()
                .all(|statement| statement.expected_rows.is_none())
        );
        assert!(
            reviewed
                .statements
                .last()
                .unwrap()
                .sql
                .contains("RENAME COLUMN")
        );
        assert!(
            reviewed
                .statements
                .iter()
                .all(|statement| !statement.sql.contains(" USING "))
        );
        super::super::apply(&mut driver, &reviewed).unwrap();
        let row=driver.client.query_one("SELECT \"ti\"\"tle\",amount::text,state::text,amounts::text,count,complex::text FROM \"rustable.structure.qa\".t",&[]).unwrap();
        assert_eq!(row.get::<_, String>(0), "hello");
        assert_eq!(row.get::<_, String>(1), "123.45");
        assert_eq!(row.get::<_, String>(2), "new");
        assert_eq!(row.get::<_, String>(3), "{1.25,NULL}");
        assert_eq!(row.get::<_, i64>(4), 5);
        assert_eq!(row.get::<_, String>(5), "(1,pair)");
        driver
            .client
            .batch_execute("INSERT INTO \"rustable.structure.qa\".t DEFAULT VALUES")
            .unwrap();
        let default: String = driver
            .client
            .query_one(
                "SELECT \"ti\"\"tle\" FROM \"rustable.structure.qa\".t WHERE count=9",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(default, "O'Reilly\\path");
        let drop = super::super::plan(
            &mut driver,
            &[Change::DropIndex {
                table: table.into(),
                name: "index.with\"quote".into(),
            }],
        )
        .unwrap();
        super::super::apply(&mut driver, &drop).unwrap();
        assert!(
            super::super::plan(
                &mut driver,
                &[Change::DropIndex {
                    table: "\"rustable.structure.qa\".keyed".into(),
                    name: "keyed_pkey".into()
                }]
            )
            .unwrap_err()
            .contains("constraint")
        );
        // A narrowing assignment must error rather than silently truncate data.
        let narrow = super::super::plan(
            &mut driver,
            &[Change::AlterColumn {
                table: table.into(),
                column: "ti\"tle".into(),
                data_type: Some("varchar(2)".into()),
                not_null: None,
                default: Some(None),
            }],
        )
        .unwrap();
        assert!(super::super::apply(&mut driver, &narrow).is_err());
        assert_eq!(driver.client.query_one("SELECT character_maximum_length FROM information_schema.columns WHERE table_schema='rustable.structure.qa' AND table_name='t' AND column_name='ti\"tle'",&[]).unwrap().get::<_,i32>(0),20);
        driver
            .client
            .batch_execute("INSERT INTO \"rustable.structure.qa\".t DEFAULT VALUES")
            .unwrap();
        let add = || Change::AddColumn {
            table: table.into(),
            definition: ColumnDefinition {
                name: "must_rollback".into(),
                data_type: "integer".into(),
                not_null: false,
                default: None,
            },
        };
        let rollback = super::super::plan(
            &mut driver,
            &[
                add(),
                Change::CreateIndex {
                    table: table.into(),
                    name: "duplicate_title".into(),
                    columns: vec![("ti\"tle".into(), false)],
                    unique: true,
                },
            ],
        )
        .unwrap();
        assert!(super::super::apply(&mut driver, &rollback).is_err());
        assert_eq!(driver.client.query_one("SELECT count(*) FROM information_schema.columns WHERE table_schema='rustable.structure.qa' AND table_name='t' AND column_name='must_rollback'",&[]).unwrap().get::<_,i64>(0),0);
        let changed = super::super::plan(&mut driver, &[add()]).unwrap();
        driver
            .client
            .batch_execute(
                "ALTER TABLE \"rustable.structure.qa\".t ADD COLUMN external_change text",
            )
            .unwrap();
        assert!(
            super::super::apply(&mut driver, &changed)
                .unwrap_err()
                .contains("structure changed")
        );
        // Target types not yet used by the table have recursive review guards.
        driver.client.batch_execute("CREATE TYPE \"rustable.structure.qa\".future_enum AS ENUM('one'); CREATE DOMAIN \"rustable.structure.qa\".future_domain AS \"rustable.structure.qa\".future_enum").unwrap();
        let nested = super::super::plan(
            &mut driver,
            &[Change::AddColumn {
                table: table.into(),
                definition: ColumnDefinition {
                    name: "nested".into(),
                    data_type: "\"rustable.structure.qa\".future_domain[]".into(),
                    not_null: false,
                    default: None,
                },
            }],
        )
        .unwrap();
        driver
            .client
            .batch_execute("ALTER TYPE \"rustable.structure.qa\".future_enum ADD VALUE 'later'")
            .unwrap();
        assert!(
            super::super::apply(&mut driver, &nested)
                .unwrap_err()
                .contains("type definition changed")
        );
        assert!(
            super::super::plan(
                &mut driver,
                &[
                    add(),
                    Change::InsertRow {
                        id: 1,
                        table: table.into(),
                        fields: vec![]
                    }
                ]
            )
            .unwrap_err()
            .contains("separately")
        );
        for data_type in [
            "integer; DROP TABLE keyed",
            "varchar(10) DEFAULT evil()",
            "int USING 1",
        ] {
            let invalid = Change::AddColumn {
                table: table.into(),
                definition: ColumnDefinition {
                    name: "injection".into(),
                    data_type: data_type.into(),
                    not_null: false,
                    default: None,
                },
            };
            assert!(super::super::plan(&mut driver, &[invalid]).is_err());
        }
        driver
            .client
            .batch_execute("DROP SCHEMA \"rustable.structure.qa\" CASCADE")
            .unwrap();
    }
}
