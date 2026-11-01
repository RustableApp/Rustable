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

#[cfg(test)]
mod signature_tests {
    use super::*;
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
