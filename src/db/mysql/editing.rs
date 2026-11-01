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
    let predicates = object.columns.iter().map(|c| format!("CAST({} AS BINARY) <=> CAST(? AS BINARY)", quote(&c.name))).collect::<Vec<_>>().join(" AND ");
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
