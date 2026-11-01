//! Review generation for data changes. DDL is deliberately a separate workflow.
use super::*;

fn qualified(object: &Object) -> Result<String, String> {
    Ok(format!("{}.{}", quote(object.schema.as_deref().ok_or("Missing database")?), quote(&object.name)))
}
fn row_key(object: &Object, original: &[Value]) -> Result<(String, Vec<Value>), String> {
    if original.len() != object.columns.len() || !object.columns.iter().any(|c| c.pk) {
        return Err("Editing requires a primary key and current row metadata. Refresh the table.".into());
    }
    let mut predicates = Vec::new();
    let mut parameters = Vec::new();
    for (column, value) in object.columns.iter().zip(original).filter(|(c, _)| c.pk) {
        if matches!(value, Value::Null) { return Err("Primary key values cannot be NULL.".into()); }
        predicates.push(format!("{} <=> ?", quote(&column.name)));
        parameters.push(value.clone());
    }
    Ok((predicates.join(" AND "), parameters))
}
fn writable(object: &Object, name: &str, value: &Value) -> Result<(), String> {
    let column = object.columns.iter().find(|c| c.name == name && !c.generated)
        .ok_or("Choose an existing writable column. Refresh the table.")?;
    if column.not_null && matches!(value, Value::Null) { return Err(format!("{} does not allow NULL", column.name)); }
    ::mysql::Value::try_from(value)?;
    Ok(())
}
fn check_row(plan: &mut ChangePlan, object: &Object, original: &[Value]) -> Result<(), String> {
    let (key, mut parameters) = row_key(object, original)?;
    // Binary comparison catches case-only and trailing-space changes even on
    // case-insensitive database collations. NULL remains NULL-safe.
    let predicates = object.columns.iter().zip(original).map(|(column, value)| {
        let name = quote(&column.name);
        if matches!(value, Value::Real(_)) {
            // FLOAT's decimal display is shorter than its exact binary value.
            // Compare in DOUBLE arithmetic to the lossless wire value instead
            // of comparing two differently formatted decimal strings.
            format!("({name} + 0e0) <=> ?")
        } else {
            format!("CAST({name} AS BINARY) <=> CAST(? AS BINARY)")
        }
    }).collect::<Vec<_>>().join(" AND ");
    parameters.extend_from_slice(original);
    plan.preconditions.push(PlannedPrecondition { before_statement: plan.statements.len(),
        sql: format!("SELECT 1 FROM {} WHERE ({key}) AND ({predicates}) FOR UPDATE", qualified(object)?), parameters });
    Ok(())
}
pub(super) fn plan(changes: &[Change], objects: &[Object]) -> Result<ChangePlan, String> {
    let mut plan = ChangePlan::default();
    let object = |table: &str| objects.iter().find(|o| o.name == table && o.kind == "table")
        .ok_or_else(|| "Only existing base tables can be edited.".to_owned());
    type RowEdits = (String, Vec<Value>, Vec<(String, Value)>);
    let mut edits: Vec<RowEdits> = Vec::new();
    for change in changes {
        let table = object(change.table())?;
        match change {
            Change::Cell { table, column, original, value } => {
                writable(object(table)?, column, value)?;
                row_key(object(table)?, original)?;
                let index = edits.iter().position(|(t, r, _)| t == table && r == original).unwrap_or_else(|| {
                    edits.push((table.clone(), original.clone(), Vec::new())); edits.len()-1
                });
                let fields = &mut edits[index].2;
                if let Some((_, previous)) = fields.iter_mut().find(|(name, _)| name == column) { *previous = value.clone(); }
                else { fields.push((column.clone(), value.clone())); }
            }
            Change::InsertRow { fields, .. } => {
                let mut seen = std::collections::HashSet::new();
                for (name, value) in fields {
                    if !seen.insert(name) { return Err("An inserted column was specified twice.".into()); }
                    writable(table, name, value)?;
                }
            }
            Change::DeleteRow { original, .. } => { row_key(table, original)?; }
            _ => return Err("MySQL schema changes require a separate review because DDL can implicitly commit.".into()),
        }
    }
    for (table, original, fields) in edits {
        if changes.iter().any(|c| matches!(c, Change::DeleteRow { table: t, original: r } if t == &table && r == &original)) { continue; }
        let object = object(&table)?;
        let (predicate, key) = row_key(object, &original)?;
        check_row(&mut plan, object, &original)?;
        let set = fields.iter().map(|(name, _)| format!("{} = ?", quote(name))).collect::<Vec<_>>().join(", ");
        plan.statements.push(PlannedStatement { sql: format!("UPDATE {} SET {set} WHERE {predicate}", qualified(object)?),
            parameters: fields.into_iter().map(|(_, value)| value).chain(key).collect(), expected_rows: Some(1) });
    }
    let mut deleted = Vec::new();
    for change in changes {
        let object = object(change.table())?;
        match change {
            Change::DeleteRow { table, original } => {
                if deleted.contains(&(table, original)) { continue; }
                deleted.push((table, original));
                let (predicate, parameters) = row_key(object, original)?;
                check_row(&mut plan, object, original)?;
                plan.statements.push(PlannedStatement { sql: format!("DELETE FROM {} WHERE {predicate}", qualified(object)?), parameters, expected_rows: Some(1) });
            }
            Change::InsertRow { fields, .. } => {
                let columns = fields.iter().map(|(name, _)| quote(name)).collect::<Vec<_>>().join(", ");
                let placeholders = vec!["?"; fields.len()].join(", ");
                plan.statements.push(PlannedStatement { sql: format!("INSERT INTO {} ({columns}) VALUES ({placeholders})", qualified(object)?),
                    parameters: fields.iter().map(|(_, value)| value.clone()).collect(), expected_rows: Some(1) });
            }
            _ => {}
        }
    }
    Ok(plan)
}


/// SHOW CREATE includes the next AUTO_INCREMENT value, which is data state,
/// not a change to the reviewed table layout. Ignore only that table option;
/// retain column attributes, quoted text, comments and all other options.
pub(super) fn schema_signature(ddl: &str, mode: Mode) -> Result<Vec<String>, String> {
    use sqlparser::tokenizer::{Token, Tokenizer, Whitespace};
    let lexer = crate::sql::mysql::Lexer(mode);
    let tokens = Tokenizer::new(&lexer, ddl).with_unescape(false).tokenize()
        .map_err(|error| format!("Cannot fingerprint MySQL table definition: {error}"))?
        .into_iter().filter(|token| !matches!(token, Token::Whitespace(
            Whitespace::Space | Whitespace::Newline | Whitespace::Tab))).collect::<Vec<_>>();
    let (mut depth, mut body_seen, mut i) = (0usize, false, 0usize);
    let mut signature = Vec::new();
    while i < tokens.len() {
        if body_seen && depth == 0
            && matches!(&tokens[i], Token::Word(word) if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("AUTO_INCREMENT"))
            && matches!(tokens.get(i+1), Some(Token::Eq))
            && matches!(tokens.get(i+2), Some(Token::Number(value, false)) if !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit())) {
            i += 3;
            continue;
        }
        match tokens[i] {
            Token::LParen => { depth += 1; body_seen = true; }
            Token::RParen => { depth = depth.checked_sub(1).ok_or("Unbalanced table definition")?; }
            _ => {}
        }
        // Debug encoding preserves token kinds, quote styles and exact literal
        // contents; serializing the vector keeps token boundaries unambiguous.
        signature.push(format!("{:?}", tokens[i]));
        i += 1;
    }
    if depth != 0 || !body_seen { return Err("Incomplete table definition".into()); }
    Ok(signature)
}


fn snapshot_table(key: &str) -> Result<(String, String), String> {
    use sqlparser::tokenizer::{Token, Tokenizer};
    let tokens = Tokenizer::new(&crate::sql::mysql::Lexer(Mode::default()), key)
        .tokenize().map_err(|error| error.to_string())?;
    match tokens.as_slice() {
        [Token::Word(schema), Token::Period, Token::Word(table)]
            if schema.quote_style == Some('`') && table.quote_style == Some('`') =>
            Ok((schema.value.clone(), table.value.clone())),
        _ => Err("Invalid reviewed table identity. Review the changes again.".into()),
    }
}

pub(super) fn grantee(account: &str) -> Option<String> {
    let (user, host) = account.rsplit_once('@')?;
    if user.contains(['\'', '\\']) || host.contains(['\'', '\\']) { return None; }
    Some(format!("'{user}'@'{host}'"))
}

// Fail closed when trigger visibility cannot be established. Role-derived and
// wildcard database grants need a separate effective-privilege implementation.
pub(super) fn require_trigger_visibility(driver: &mut MysqlDriver, schema: &str, table: &str) -> Result<(), String> {
    let account: String = driver.with_connection(|conn| conn.query_first("SELECT CURRENT_USER()"))?
        .ok_or("Missing current database account")?;
    let grantee = grantee(&account).ok_or("Cannot yet establish trigger visibility for this account name.")?;
    let table_grant: Option<u8> = driver.with_connection(|conn| conn.exec_first(
        "SELECT 1 FROM information_schema.TABLE_PRIVILEGES WHERE GRANTEE=? AND TABLE_SCHEMA=? AND TABLE_NAME=? AND PRIVILEGE_TYPE='TRIGGER' LIMIT 1",
        (&grantee, schema, table)))?;
    let schema_grants: Vec<String> = driver.with_connection(|conn| conn.exec(
        "SELECT TABLE_SCHEMA FROM information_schema.SCHEMA_PRIVILEGES WHERE GRANTEE=? AND PRIVILEGE_TYPE='TRIGGER'", (&grantee,)))?;
    let schema_grant = schema_grants.iter().any(|pattern| {
        // MySQL exposes escaped database patterns (e.g. rustable\_test).
        // Recognize exact names; broader wildcard/role grants remain unsupported.
        let mut chars = pattern.chars();
        let mut name = String::new();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some(c @ ('_' | '%' | '\\')) => name.push(c),
                    _ => return false,
                }
            } else { name.push(ch); }
        }
        name == schema
    });
    // Global grants are sufficient only when schema-level partial revokes cannot
    // hide triggers. Do not infer effective role or restricted global privileges.
    let global_grant = if table_grant.is_none() && !schema_grant && driver.global_trigger_at_connect {
        let unrestricted = if driver.engine == "MariaDB" { true } else {
            driver.with_connection(|conn| conn.query_first::<u8, _>("SELECT @@GLOBAL.partial_revokes"))? == Some(0)
        };
        unrestricted && driver.with_connection(|conn| conn.exec_first::<u8, _, _>(
            "SELECT 1 FROM information_schema.USER_PRIVILEGES WHERE GRANTEE=? AND PRIVILEGE_TYPE='TRIGGER' LIMIT 1",
            (&grantee,)))?.is_some()
    } else { false };
    if table_grant.is_none() && !schema_grant && !global_grant {
        return Err("Cannot verify all table triggers with this account's direct privileges. Reviewed MySQL saves are unavailable for this account.".into());
    }
    Ok(())
}

// SQL-layer foreign keys in newer MySQL versions can execute child triggers.
// Until indirect trigger effects are reviewed, require the established behavior;
// never change the user's foreign-key or trigger settings to make a save work.
fn require_supported_cascades(driver: &mut MysqlDriver, plan: &ChangePlan) -> Result<(), String> {
    if driver.engine != "MySQL" || !plan.statements.iter().any(|statement|
        statement.sql.starts_with("UPDATE ") || statement.sql.starts_with("DELETE FROM ")) {
        return Ok(());
    }
    let variables: Vec<(String,String)> = driver.with_connection(|conn| conn.query(
        "SHOW SESSION VARIABLES WHERE Variable_name IN ('innodb_native_foreign_keys','enable_cascade_triggers')"))?;
    supported_cascade_settings(&variables)
}

fn supported_cascade_settings(variables: &[(String,String)]) -> Result<(), String> {
    let value = |name: &str| -> Result<Option<bool>,String> {
        variables.iter().find(|(key,_)| key.eq_ignore_ascii_case(name)).map(|(_,value)| {
            match value.to_ascii_uppercase().as_str() {
                "ON" | "1" => Ok(true),
                "OFF" | "0" => Ok(false),
                _ => Err(format!("Cannot verify MySQL cascade setting {name}.")),
            }
        }).transpose()
    };
    let native = value("innodb_native_foreign_keys")?;
    let triggers = value("enable_cascade_triggers")?;
    if (native.is_none() && triggers.is_none()) || native == Some(true) || triggers == Some(false) {
        return Ok(());
    }
    Err("Reviewed updates and deletes do not yet support this server's cascade-trigger mode. No changes were saved; server settings were left unchanged.".into())
}

pub(super) fn apply(driver: &mut MysqlDriver, plan: &ChangePlan) -> Result<(), String> {
    driver.require_idle()?;
    if plan.statements.is_empty() { return Ok(()); }
    if plan.rebuild || plan.schemas.is_empty() || plan.preconditions.iter().any(|c| c.before_statement >= plan.statements.len()) {
        return Err("Invalid data review. Review the changes again.".into());
    }
    require_supported_cascades(driver, plan)?;
    let checks_sql = if driver.engine == "MariaDB" {
        "SELECT @@SESSION.foreign_key_checks, @@SESSION.unique_checks, @@SESSION.check_constraint_checks"
    } else {
        "SELECT @@SESSION.foreign_key_checks, @@SESSION.unique_checks, 1"
    };
    let checks: Option<(u8, u8, u8)> = driver.with_connection(|conn| conn.query_first(checks_sql))?;
    if checks != Some((1, 1, 1)) {
        return Err("Reviewed saves require foreign_key_checks, unique_checks and CHECK constraint enforcement to be enabled. Restore the session settings and review again.".into());
    }
    let mut tables = plan.schemas.iter().map(|(key, snapshot)| {
        snapshot_table(key).map(|(schema, table)| (schema, table, snapshot))
    }).collect::<Result<Vec<_>,_>>()?;
    tables.sort_by(|a,b| (&a.0,&a.1).cmp(&(&b.0,&b.1)));
    let result = (|| -> Result<(), String> {
        driver.with_connection(|conn| conn.query_drop("START TRANSACTION"))?;
        for (schema, table, expected) in &tables {
            driver.with_connection(|conn| conn.query_drop(format!("SELECT * FROM {}.{} LIMIT 0", quote(schema), quote(table))))?;
            require_trigger_visibility(driver, schema, table)?;
            let engine: Option<String> = driver.with_connection(|conn| conn.exec_first(
                "SELECT ENGINE FROM information_schema.TABLES WHERE TABLE_SCHEMA=? AND TABLE_NAME=?", (schema, table)))?;
            if engine.as_deref() != Some("InnoDB") { return Err("Reviewed saves require InnoDB base tables.".into()); }
            let triggers: Option<u64> = driver.with_connection(|conn| conn.exec_first(
                "SELECT COUNT(*) FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA=? AND EVENT_OBJECT_TABLE=?", (schema, table)))?;
            if triggers != Some(0) {
                return Err("Reviewed saves for tables with triggers are not implemented yet. No changes were saved.".into());
            }
            if driver.edit_schema_snapshot(schema, table)? != **expected {
                return Err("Table definition changed since review. Refresh and review again.".into());
            }
            expressions::validate_managed_write(driver, schema, table)?;
        }
        for (index, statement) in plan.statements.iter().enumerate() {
            let update = statement.sql.starts_with("UPDATE ");
            if !update && !statement.sql.starts_with("INSERT INTO ") && !statement.sql.starts_with("DELETE FROM ") {
                return Err("Only reviewed row changes can use this transaction.".into());
            }
            let checks = plan.preconditions.iter().filter(|c| c.before_statement == index).collect::<Vec<_>>();
            if (update || statement.sql.starts_with("DELETE FROM ")) && checks.len() != 1 {
                return Err("Missing original-row check. Review changes again.".into());
            }
            for check in checks {
                let parameters = check.parameters.iter().map(::mysql::Value::try_from).collect::<Result<Vec<_>,_>>()?;
                let rows: Vec<u8> = driver.with_connection(|conn| conn.exec(&check.sql, parameters))?;
                if rows != [1] { return Err("Row changed or was deleted since loading. Refresh and review again.".into()); }
            }
            let parameters = statement.parameters.iter().map(::mysql::Value::try_from).collect::<Result<Vec<_>,_>>()?;
            let (affected, warnings) = driver.with_connection(|conn| {
                conn.exec_drop(&statement.sql, parameters)?;
                Ok((conn.affected_rows(), conn.warnings()))
            })?;
            if warnings != 0 { return Err("Server reported a conversion or other warning. The batch was not saved.".into()); }
            // MySQL reports changed rows, not matched rows by default. The
            // locked original-row check already established the one-row match.
            if statement.expected_rows != Some(affected as usize) && !(update && affected == 0 && statement.expected_rows == Some(1)) {
                return Err("Unexpected affected row count. The batch was not saved.".into());
            }
        }
        if driver.cancel.load(Ordering::Acquire) { return Err("Save cancelled before commit.".into()); }
        Ok(())
    })();
    if let Err(error) = result {
        // Cancellation must not cancel cleanup. Never replay an uncertain write.
        let rollback = driver.conn().and_then(|conn| conn.query_drop("ROLLBACK").map_err(|e| e.to_string()));
        if let Err(rollback_error) = rollback {
            driver.connection = None;
            return Err(format!("{error}\nRollback could not be confirmed: {rollback_error}. Reconnect and verify data before retrying."));
        }
        return Err(error);
    }
    // Do not issue KILL QUERY during COMMIT: once sent, its result must be
    // observed. Transport loss is an unknown outcome, never an automatic retry.
    if let Err(error) = driver.conn()?.query_drop("COMMIT") {
        driver.connection = None;
        return Err(format!("Commit outcome is unknown: {error}. Reconnect and verify saved data before retrying."));
    }
    Ok(())
}

#[cfg(test)]
mod signature_tests {
    use super::*;
    #[test]
    fn cascade_settings_preserve_legacy_and_explicitly_disabled_trigger_behavior() {
        let settings = |pairs: &[(&str,&str)]| pairs.iter().map(|(k,v)| (k.to_string(),v.to_string())).collect::<Vec<_>>();
        for pairs in [
            vec![],
            vec![("innodb_native_foreign_keys","ON"),("enable_cascade_triggers","ON")],
            vec![("innodb_native_foreign_keys","OFF"),("enable_cascade_triggers","OFF")],
            vec![("enable_cascade_triggers","0")],
        ] { assert!(supported_cascade_settings(&settings(&pairs)).is_ok(),"{pairs:?}"); }
        for pairs in [
            vec![("innodb_native_foreign_keys","OFF"),("enable_cascade_triggers","ON")],
            vec![("innodb_native_foreign_keys","OFF")],
            vec![("enable_cascade_triggers","1")],
            vec![("enable_cascade_triggers","unknown")],
        ] { assert!(supported_cascade_settings(&settings(&pairs)).is_err(),"{pairs:?}"); }
    }

    #[test]
    fn auto_increment_counter_is_not_schema_but_attributes_and_literals_are() {
        let base = "CREATE TABLE `counter` (`id` bigint AUTO_INCREMENT, `label` varchar(80) DEFAULT 'AUTO_INCREMENT=22', PRIMARY KEY (`id`)) ENGINE=InnoDB";
        let signature = |ddl: &str| schema_signature(ddl, Mode::default()).unwrap();
        assert_eq!(signature(base), signature(&format!("{base} AUTO_INCREMENT=27")));
        assert_eq!(signature(&format!("{base} AUTO_INCREMENT=27")), signature(&format!("{base} AUTO_INCREMENT=999")));
        assert_ne!(signature(base), signature(&base.replace("bigint AUTO_INCREMENT", "bigint")));
        assert_ne!(signature(base), signature(&base.replace("'AUTO_INCREMENT=22'", "'AUTO_INCREMENT=23'")));
        assert_ne!(signature(base), signature(&format!("{base} COMMENT='AUTO_INCREMENT=27'")));
        assert_ne!(signature(base), signature(&format!("{base} /* AUTO_INCREMENT=27 */")));
        assert_ne!(signature(base), signature(&base.replace("InnoDB", "MyISAM")));
        assert!(schema_signature("CREATE TABLE broken (", Mode::default()).is_err());
    }
}
