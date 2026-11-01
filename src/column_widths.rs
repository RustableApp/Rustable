//! Persistent grid geometry. Keys describe database identity, never row values or SQL.
use crate::connection_profiles::ProfileId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_WIDTH: f32 = 155.0;
pub const MIN_WIDTH: f32 = 40.0;
pub const MAX_WIDTH: f32 = 4096.0;
const MAX_ENTRIES: usize = 16_384;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scope {
    Table {
        profile: ProfileId,
        table: String,
    },
    /// Results with the same output shape share geometry, independently of tables.
    /// Raw SQL (which can contain passwords or private literal data) is never saved.
    Query {
        profile: ProfileId,
        columns: Vec<String>,
    },
}

/// Exact, collision-free keys prepared once when a grid shape is first rendered.
/// The potentially long query shape is stored once, not repeated for each column.
#[derive(Clone)]
pub struct Prepared {
    scope: String,
    columns: Vec<String>,
}
impl Prepared {
    pub fn new(scope: &Scope, columns: &[String]) -> Self {
        let mut occurrences = std::collections::HashMap::new();
        Self {
            scope: serde_json::to_string(scope).expect("scope is serializable"),
            columns: columns
                .iter()
                .map(|name| {
                    let occurrence = occurrences.entry(name).or_insert(0usize);
                    let key = serde_json::to_string(&(name, *occurrence))
                        .expect("column is serializable");
                    *occurrence += 1;
                    key
                })
                .collect(),
        }
    }
}

#[derive(Clone, Default)]
pub struct ColumnWidths(BTreeMap<String, BTreeMap<String, f32>>);

// Keep the map encoding compatible with eframe's RON storage. This reserved
// key distinguishes deliberate new 155px widths from legacy uniform defaults.
const FORMAT_KEY: &str = "__rustable_column_widths_version";
impl Serialize for ColumnWidths {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.0.len() + 1))?;
        map.serialize_entry(FORMAT_KEY, &2.0f32)?;
        for (scope, columns) in &self.0 {
            map.serialize_entry(scope, columns)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ColumnWidths {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Columns(BTreeMap<String, f32>),
            Legacy(f32),
        }
        let stored = BTreeMap::<String, Stored>::deserialize(deserializer)?;
        let current_format =
            matches!(stored.get(FORMAT_KEY), Some(Stored::Legacy(version)) if *version >= 2.0);
        let mut result = Self::default();
        for (key, value) in stored {
            match value {
                Stored::Columns(columns) => {
                    if !columns.is_empty() {
                        result.0.insert(key, columns);
                    }
                }
                Stored::Legacy(width) => {
                    if let Ok((scope, name, occurrence)) =
                        serde_json::from_str::<(Scope, String, usize)>(&key)
                    {
                        result
                            .0
                            .entry(serde_json::to_string(&scope).map_err(serde::de::Error::custom)?)
                            .or_default()
                            .insert(
                                serde_json::to_string(&(name, occurrence))
                                    .map_err(serde::de::Error::custom)?,
                                width,
                            );
                    }
                }
            }
        }
        if !current_format {
            // Older builds stored the uniform starting geometry as overrides.
            // Recover content sizing only for that exact multi-column pattern;
            // preserve mixed widths and every non-default manual dimension.
            result.0.retain(|_, columns| {
                columns.len() < 2 || columns.values().any(|width| *width != DEFAULT_WIDTH)
            });
        }
        Ok(result)
    }
}

impl ColumnWidths {
    #[cfg(test)]
    pub fn widths(&self, scope: &Scope, columns: &[String]) -> Vec<f32> {
        self.prepared_widths(&Prepared::new(scope, columns))
    }

    pub fn has_width(&self, keys: &Prepared, index: usize) -> bool {
        keys.columns.get(index).is_some_and(|key| {
            self.0
                .get(&keys.scope)
                .is_some_and(|columns| columns.contains_key(key))
        })
    }

    /// Keep content-derived defaults ephemeral. Only a changed rendered width
    /// (manual drag or Fit columns) becomes an explicit saved preference.
    pub fn record_changed(
        &mut self,
        keys: &Prepared,
        widths: &[f32],
        previous: &[f32],
        resized_columns: &[usize],
    ) -> bool {
        let changed: Vec<f32> = widths
            .iter()
            .zip(previous)
            .enumerate()
            .map(|(index, (width, old))| {
                if resized_columns.contains(&index) && (*width - *old).abs() > f32::EPSILON {
                    *width
                } else {
                    f32::NAN
                }
            })
            .collect();
        self.record_prepared(keys, &changed)
    }

    pub fn prepared_widths(&self, keys: &Prepared) -> Vec<f32> {
        let scope = self.0.get(&keys.scope);
        keys.columns
            .iter()
            .map(|key| {
                scope
                    .and_then(|columns| columns.get(key))
                    .copied()
                    .filter(|width| width.is_finite())
                    .unwrap_or(DEFAULT_WIDTH)
                    .clamp(MIN_WIDTH, MAX_WIDTH)
            })
            .collect()
    }

    #[cfg(test)]
    pub fn record(&mut self, scope: &Scope, columns: &[String], widths: &[f32]) -> bool {
        self.record_prepared(&Prepared::new(scope, columns), widths)
    }

    /// Changed geometry only; host autosave performs persistence off the drawing path.
    pub fn record_prepared(&mut self, keys: &Prepared, widths: &[f32]) -> bool {
        if !widths
            .iter()
            .take(keys.columns.len())
            .any(|width| width.is_finite())
        {
            return false;
        }
        let mut changed = false;
        // Lookup by reference on ordinary frames; clone the scope only when new.
        if !self.0.contains_key(&keys.scope) {
            self.0.insert(keys.scope.clone(), BTreeMap::new());
        }
        let columns = self.0.get_mut(&keys.scope).expect("scope was inserted");
        for (key, width) in keys.columns.iter().zip(widths.iter().copied()) {
            if !width.is_finite() {
                continue;
            }
            let width = width.clamp(MIN_WIDTH, MAX_WIDTH);
            if columns
                .get(key)
                .is_some_and(|old| (*old - width).abs() <= f32::EPSILON)
            {
                continue;
            }
            columns.insert(key.clone(), width);
            changed = true;
        }
        if changed {
            let mut excess = self
                .0
                .values()
                .map(BTreeMap::len)
                .sum::<usize>()
                .saturating_sub(MAX_ENTRIES);
            while excess > 0 {
                let Some(mut scope) = self.0.first_entry() else {
                    break;
                };
                if scope.get_mut().pop_first().is_some() {
                    excess -= 1;
                }
                if scope.get().is_empty() {
                    scope.remove();
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn table(profile: u64, name: &str) -> Scope {
        Scope::Table {
            profile: ProfileId(profile),
            table: name.into(),
        }
    }
    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).into()).collect()
    }

    #[test]
    fn restart_retains_profile_table_and_column_identity() {
        let columns = names(&["id", "name", "email"]);
        let scope = table(11, "\"public\".\"customers\"");
        let mut widths = ColumnWidths::default();
        assert!(widths.record(&scope, &columns, &[80.0, 230.0, 300.0]));
        let restored: ColumnWidths =
            serde_json::from_str(&serde_json::to_string(&widths).unwrap()).unwrap();
        assert_eq!(
            restored.widths(&scope, &names(&["email", "id"])),
            vec![300.0, 80.0]
        );
        assert_eq!(
            restored.widths(&scope, &names(&["name", "new"])),
            vec![230.0, DEFAULT_WIDTH]
        );
        assert_eq!(
            restored.widths(&table(12, "\"public\".\"customers\""), &columns),
            vec![DEFAULT_WIDTH; 3]
        );
        assert_eq!(
            restored.widths(&table(11, "\"private\".\"customers\""), &columns),
            vec![DEFAULT_WIDTH; 3]
        );
    }

    #[test]
    fn query_namespaces_and_duplicate_names_are_independent() {
        let columns = names(&["id", "id"]);
        let query = Scope::Query {
            profile: ProfileId(11),
            columns: columns.clone(),
        };
        let mut widths = ColumnWidths::default();
        widths.record(&query, &columns, &[110.0, 240.0]);
        assert_eq!(widths.widths(&query, &columns), vec![110.0, 240.0]);
        assert_eq!(
            widths.widths(&table(11, "query"), &columns),
            vec![DEFAULT_WIDTH; 2]
        );
        let other = Scope::Query {
            profile: ProfileId(11),
            columns: names(&["id", "email"]),
        };
        assert_eq!(widths.widths(&other, &columns), vec![DEFAULT_WIDTH; 2]);
    }

    #[test]
    fn actual_eframe_ron_storage_roundtrips_and_migrates_legacy_widths() {
        #[derive(Default)]
        struct Storage(BTreeMap<String, String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn flush(&mut self) {}
        }
        #[derive(Default, Serialize, Deserialize)]
        #[serde(default)]
        struct Workspace {
            column_widths: ColumnWidths,
            history: Vec<String>,
        }
        #[derive(Serialize)]
        struct LegacyWorkspace {
            column_widths: BTreeMap<String, f32>,
            history: Vec<String>,
        }
        let mut storage = Storage::default();
        let scope = table(7, "\"public\".\"customers\"");
        let columns = names(&["id", "name"]);
        let mut workspace = Workspace {
            history: vec!["SELECT 1".into()],
            ..Default::default()
        };
        workspace
            .column_widths
            .record(&scope, &columns, &[85.0, 230.0]);
        eframe::set_value(&mut storage, "workspace", &workspace);
        let restored: Workspace = eframe::get_value(&storage, "workspace")
            .expect("actual eframe RON width preferences must deserialize");
        assert_eq!(
            restored.column_widths.widths(&scope, &columns),
            vec![85.0, 230.0]
        );
        assert_eq!(restored.history, workspace.history);
        let legacy_key = serde_json::to_string(&(&scope, "name", 0)).unwrap();
        let legacy = LegacyWorkspace {
            column_widths: BTreeMap::from([(legacy_key, 275.0)]),
            history: vec!["SELECT 2".into()],
        };
        eframe::set_value(&mut storage, "workspace", &legacy);
        let migrated: Workspace = eframe::get_value(&storage, "workspace")
            .expect("legacy RON width preferences must migrate without dropping workspace");
        assert_eq!(
            migrated.column_widths.widths(&scope, &columns),
            vec![DEFAULT_WIDTH, 275.0]
        );
        assert_eq!(migrated.history, legacy.history);
        eframe::set_value(&mut storage, "workspace", &migrated);
        let restarted: Workspace =
            eframe::get_value(&storage, "workspace").expect("migrated RON must roundtrip");
        assert_eq!(
            restarted.column_widths.widths(&scope, &columns),
            vec![DEFAULT_WIDTH, 275.0]
        );
        assert_eq!(restarted.history, legacy.history);
    }

    #[test]
    fn automatic_geometry_changes_are_not_saved_without_resize_intent() {
        let scope = table(1, "customers");
        let columns = names(&["id", "email"]);
        let keys = Prepared::new(&scope, &columns);
        let mut store = ColumnWidths::default();
        assert!(!store.record_changed(&keys, &[70.0, 155.0], &[40.0, 80.0], &[]));
        assert!(!store.has_width(&keys, 0));
        assert!(!store.has_width(&keys, 1));
        assert!(store.record_changed(&keys, &[70.0, 155.0], &[40.0, 80.0], &[0]));
        assert!(store.has_width(&keys, 0));
        assert!(!store.has_width(&keys, 1));
    }

    #[test]
    fn legacy_uniform_defaults_reset_but_manual_widths_and_new_defaults_survive() {
        #[derive(Default)]
        struct Storage(BTreeMap<String, String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn flush(&mut self) {}
        }
        let mut storage = Storage::default();
        let columns = names(&["id", "address", "available"]);
        let scope = table(1, "public.address");
        let keys = Prepared::new(&scope, &columns);
        let mut legacy = ColumnWidths::default();
        legacy.record(&scope, &columns, &[155.0; 3]);
        eframe::set_value(&mut storage, "widths", &legacy.0);
        let restored: ColumnWidths = eframe::get_value(&storage, "widths").unwrap();
        assert!((0..3).all(|index| !restored.has_width(&keys, index)));
        // New explicit widths are marked, even when the user chooses exactly
        // the former default for every column.
        eframe::set_value(&mut storage, "widths", &legacy);
        let restored: ColumnWidths = eframe::get_value(&storage, "widths").unwrap();
        assert!((0..3).all(|index| restored.has_width(&keys, index)));
        legacy.record(&scope, &columns, &[65.0, 155.0, 310.0]);
        eframe::set_value(&mut storage, "widths", &legacy.0);
        let restored: ColumnWidths = eframe::get_value(&storage, "widths").unwrap();
        assert_eq!(restored.widths(&scope, &columns), vec![65.0, 155.0, 310.0]);
    }

    #[test]
    fn large_query_keys_are_linear_and_legacy_layout_migrates() {
        let columns: Vec<String> = (0..1000).map(|i| format!("column_{i}")).collect();
        let scope = Scope::Query {
            profile: ProfileId(1),
            columns: columns.clone(),
        };
        let prepared = Prepared::new(&scope, &columns);
        let key_bytes =
            prepared.scope.len() + prepared.columns.iter().map(String::len).sum::<usize>();
        assert!(
            key_bytes < 50_000,
            "query shape should be stored only once: {key_bytes}"
        );
        let mut store = ColumnWidths::default();
        assert!(store.record_prepared(&prepared, &vec![200.0; 1000]));
        assert!(!store.record_prepared(&prepared, &vec![200.0; 1000]));
        assert!(serde_json::to_string(&store).unwrap().len() < 60_000);
        let key = serde_json::to_string(&(table(1, "t"), "name", 0)).unwrap();
        let legacy = serde_json::to_string(&BTreeMap::from([(key, 230.0)])).unwrap();
        let migrated: ColumnWidths = serde_json::from_str(&legacy).unwrap();
        assert_eq!(
            migrated.widths(&table(1, "t"), &names(&["name"])),
            vec![230.0]
        );
    }

    #[test]
    fn geometry_updates_are_bounded_and_do_not_dirty_unchanged_frames() {
        let scope = table(1, "t");
        let columns = names(&["a", "b", "c"]);
        let mut widths = ColumnWidths::default();
        assert!(widths.record(&scope, &columns, &[1.0, f32::MAX, f32::NAN]));
        assert_eq!(
            widths.widths(&scope, &columns),
            vec![MIN_WIDTH, MAX_WIDTH, DEFAULT_WIDTH]
        );
        assert!(!widths.record(&scope, &columns, &[MIN_WIDTH, MAX_WIDTH]));
        assert!(!widths.record(&scope, &columns, &[f32::INFINITY]));
        assert!(widths.record(&scope, &columns, &[200.0]));
        assert!(widths.record(&scope, &columns, &[200.25]));
        assert_eq!(widths.widths(&scope, &columns)[0], 200.25);
        let mut empty = ColumnWidths::default();
        assert!(!empty.record_changed(
            &Prepared::new(&scope, &columns),
            &[70.0, 80.0, 90.0],
            &[70.0, 80.0, 90.0],
            &[0]
        ));
        assert!(
            empty.0.is_empty(),
            "unchanged automatic geometry must not allocate scopes"
        );
    }
}
