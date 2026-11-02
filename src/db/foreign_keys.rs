//! Declared relationships, never guessed from column names.
use super::*;
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ForeignKey {
    pub name: String,
    pub table: String,
    pub schema: Option<String>,
    pub columns: Vec<String>,
    pub referenced_columns: Vec<String>,
}
impl ForeignKey {
    pub fn can_follow(&self, columns: &[String], row: &[Value]) -> bool {
        !self.columns.is_empty()
            && self.columns.len() == self.referenced_columns.len()
            && self.columns.iter().all(|source| {
                columns
                    .iter()
                    .position(|c| c == source)
                    .and_then(|index| row.get(index))
                    .is_some_and(|value| match value {
                        Value::Null | Value::Blob(_) => false,
                        Value::Real(n) => n.is_finite(),
                        _ => true,
                    })
            })
    }

    pub fn filters(&self, columns: &[String], row: &[Value]) -> Option<Vec<filters::Filter>> {
        if self.columns.is_empty() || self.columns.len() != self.referenced_columns.len() {
            return None;
        }
        self.columns
            .iter()
            .zip(&self.referenced_columns)
            .map(|(source, target)| {
                let value = row.get(columns.iter().position(|c| c == source)?)?;
                let value = match value {
                    Value::Null | Value::Blob(_) => return None,
                    Value::Text(s) => s.clone(),
                    Value::Integer(n) => n.to_string(),
                    Value::Real(n) if n.is_finite() => n.to_string(),
                    _ => return None,
                };
                Some(filters::Filter {
                    enabled: true,
                    column: target.clone(),
                    operator: "=".into(),
                    value,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composite_keys_match_by_name_and_never_navigate_nulls() {
        let key = ForeignKey {
            name: "pair".into(),
            table: "target".into(),
            schema: None,
            columns: vec!["tenant".into(), "parent".into()],
            referenced_columns: vec!["tenant_id".into(), "id".into()],
        };
        let names = vec!["parent".into(), "tenant".into()];
        let filters = key
            .filters(&names, &[Value::Integer(3), Value::Text("O'Reilly".into())])
            .unwrap();
        assert_eq!(filters[0].column, "tenant_id");
        assert_eq!(filters[0].value, "O'Reilly");
        assert_eq!(filters[1].column, "id");
        assert_eq!(filters[1].value, "3");
        assert!(
            key.filters(&names, &[Value::Null, Value::Integer(1)])
                .is_none()
        );
        assert!(key.filters(&[], &[]).is_none());
    }
}
