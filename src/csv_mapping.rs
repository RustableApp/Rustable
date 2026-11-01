//! Explicit source-to-destination mappings for the import wizard.
use crate::db::Column;

#[derive(Default)]
pub struct Mapping {
    pub table: String,
    pub schema: Option<String>,
    pub columns: Vec<Column>,
    pub destinations: Vec<Option<usize>>,
}
impl Mapping {
    pub fn new(table: String, columns: Vec<Column>, source: &[String], header: bool) -> Self {
        let mut mapping = Self {
            schema: None,
            table,
            columns,
            destinations: vec![None; source.len()],
        };
        if header {
            for (index, name) in source.iter().enumerate() {
                let matches: Vec<_> = mapping
                    .columns
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| !c.generated && c.name.eq_ignore_ascii_case(name))
                    .map(|(i, _)| i)
                    .collect();
                if matches.len() == 1 && !mapping.destinations.contains(&Some(matches[0])) {
                    mapping.destinations[index] = Some(matches[0]);
                }
            }
        }
        mapping
    }

    pub fn validate(&self) -> Result<usize, String> {
        let mut used = std::collections::HashSet::new();
        for index in self.destinations.iter().flatten() {
            let column = self
                .columns
                .get(*index)
                .ok_or("Mapping references a missing column")?;
            if column.generated {
                return Err(format!("{} is a generated column", column.name));
            }
            if !used.insert(*index) {
                return Err(format!("{} is mapped more than once", column.name));
            }
        }
        if used.is_empty() {
            return Err("Choose at least one destination column".into());
        }
        Ok(used.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn column(name: &str, generated: bool) -> Column {
        Column {
            name: name.into(),
            data_type: "TEXT".into(),
            not_null: false,
            default: String::new(),
            pk: false,
            generated,
        }
    }
    #[test]
    fn matching_does_not_guess_unknown_duplicate_or_generated_fields() {
        let columns = vec![
            column("id", false),
            column("name", false),
            column("computed", true),
        ];
        let source = ["NAME", "id", "unknown", "name", "computed"].map(String::from);
        let mut m = Mapping::new("t".into(), columns.clone(), &source, true);
        assert_eq!(m.destinations, [Some(1), Some(0), None, None, None]);
        assert_eq!(m.validate(), Ok(2));
        m.destinations[3] = Some(1);
        assert!(m.validate().unwrap_err().contains("more than once"));
        assert_eq!(
            Mapping::new("t".into(), columns, &source, false).destinations,
            vec![None; 5]
        );
        let ambiguous = Mapping::new(
            "t".into(),
            vec![column("a", false), column("A", false)],
            &["a".into()],
            true,
        );
        assert_eq!(ambiguous.destinations, [None]);
    }
}
