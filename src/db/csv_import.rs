use super::{Column, Value, csv_preview};
use std::sync::Arc;

#[derive(Clone)]
#[cfg_attr(target_arch = "wasm32", derive(serde::Serialize, serde::Deserialize))]
pub struct Request {
    pub source: Arc<csv_preview::Source>,
    pub options: csv_preview::Options,
    pub table: String,
    #[cfg_attr(target_arch = "wasm32", serde(default))]
    pub schema: Option<String>,
    pub columns: Vec<Column>,
    pub destinations: Vec<Option<usize>>,
    pub empty_as_null: bool,
    pub postgres_inserts: bool,
}
impl Request {
    pub fn sql(&self) -> Result<String, String> {
        let mut seen = std::collections::HashSet::new();
        let mut names = Vec::new();
        for index in self.destinations.iter().flatten() {
            let column = self
                .columns
                .get(*index)
                .ok_or("Missing destination column")?;
            if column.generated || !seen.insert(index) {
                return Err("Generated or duplicate destination column".into());
            }
            names.push(super::sqlite::quote(&column.name));
        }
        if names.is_empty() {
            return Err("Choose at least one destination column".into());
        }
        Ok(format!(
            "INSERT INTO {} ({}) VALUES ({});",
            super::sqlite::quote(&self.table),
            names.join(", "),
            vec!["?"; names.len()].join(", ")
        ))
    }
    fn postgres_target(&self) -> Result<(String, String), String> {
        self.sql()?;
        use sqlparser::{dialect::PostgreSqlDialect, parser::Parser, tokenizer::Token};
        let mut parser = Parser::new(&PostgreSqlDialect {})
            .try_with_sql(&self.table)
            .map_err(|e| e.to_string())?;
        let table = parser.parse_object_name(false).map_err(|e| e.to_string())?;
        if parser.peek_token().token != Token::EOF {
            return Err("Invalid import destination".into());
        }
        let columns = self
            .destinations
            .iter()
            .flatten()
            .map(|index| super::sqlite::quote(&self.columns[*index].name))
            .collect::<Vec<_>>()
            .join(", ");
        Ok((table.to_string(), columns))
    }

    pub fn postgres_copy_sql(&self) -> Result<String, String> {
        let (table, columns) = self.postgres_target()?;
        Ok(format!(
            "COPY {table} ({columns}) FROM STDIN WITH (FORMAT text)"
        ))
    }

    pub fn postgres_sql(&self) -> Result<String, String> {
        if !self.postgres_inserts {
            return self.postgres_copy_sql();
        }
        let (table, columns) = self.postgres_target()?;
        let projection = self
            .destinations
            .iter()
            .flatten()
            .map(|i| format!("source.{}", super::sqlite::quote(&self.columns[*i].name)))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(format!(
            "INSERT INTO {table} ({columns}) SELECT {projection} FROM pg_catalog.json_populate_recordset(NULL::{table}, $1::pg_catalog.text::pg_catalog.json) AS source"
        ))
    }

    /// Display mapped source text without implying database type conversion.
    pub fn source_sample(&self, row: &[String]) -> String {
        let names = self.destinations.iter().flatten().map(|index| self.columns[*index].name.as_str());
        named_source_sample(names.zip(self.values(row)))
    }

    pub fn values(&self, row: &[String]) -> Vec<Value> {
        self.destinations
            .iter()
            .enumerate()
            .filter_map(|(i, target)| {
                target.map(|_| {
                    if self.empty_as_null && row[i].is_empty() {
                        Value::Null
                    } else {
                        Value::Text(row[i].clone())
                    }
                })
            })
            .collect()
    }
}

fn named_source_sample<'a>(fields: impl Iterator<Item = (&'a str, Value)>) -> String {
    let fields = fields.map(|(name, value)| {
        let name = serde_json::to_string(name).expect("string serialization");
        let value = match value {
            Value::Null => "null".into(),
            Value::Text(text) => serde_json::to_string(&text).expect("string serialization"),
            _ => unreachable!("CSV source values are text or null"),
        };
        format!("{name}: {value}")
    }).collect::<Vec<_>>();
    format!("{{{}}}", fields.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_sample_preserves_names_text_null_and_escaping() {
        let sample = named_source_sample([
            ("label\"", Value::Text("α\n\"quoted\"\\N".into())),
            ("id", Value::Text("9007199254740993".into())),
            ("empty", Value::Text(String::new())),
            ("missing", Value::Null),
        ].into_iter());
        let decoded: serde_json::Value = serde_json::from_str(&sample).unwrap();
        assert_eq!(decoded["label\""], "α\n\"quoted\"\\N");
        assert_eq!(decoded["id"], "9007199254740993");
        assert_eq!(decoded["empty"], "");
        assert!(decoded["missing"].is_null());
        assert!(sample.starts_with("{\"label"));
        assert!(!sample.contains('\n'));
    }
}
