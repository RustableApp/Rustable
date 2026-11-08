use serde::{Deserialize, Serialize};
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    pub tree: bool,
    pub separator: String,
    pub snippets: Vec<Snippet>,
    pub record_history: bool,
    pub histories: std::collections::BTreeMap<u64, crate::redis_history::History>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Snippet {
    pub name: String,
    pub command: String,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            tree: true,
            separator: ":".into(),
            snippets: vec![],
            record_history: true,
            histories: Default::default(),
        }
    }
}
impl Preferences {
    pub fn limit_profiles(&mut self) {
        while self.histories.len() > crate::redis_history::MAX_PROFILES {
            let id = *self
                .histories
                .iter()
                .min_by_key(|(_, history)| history.updated_ms)
                .unwrap()
                .0;
            self.histories.remove(&id);
        }
    }
    pub fn sanitize(&mut self) {
        for history in self.histories.values_mut() {
            history.sanitize();
        }
        self.limit_profiles();
        self.separator = self.separator.chars().take(8).collect();
        self.snippets.truncate(20);
        self.snippets.retain(|s| s.command.len() <= 65536);
        for snippet in &mut self.snippets {
            snippet.name = snippet.name.chars().take(80).collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_eviction_preserves_global_opt_out_and_legacy_preferences_load() {
        let legacy: Preferences =
            serde_json::from_str(r#"{"tree":false,"separator":":"}"#).unwrap();
        assert!(legacy.record_history);
        assert!(legacy.histories.is_empty());
        let mut preferences = Preferences {
            record_history: false,
            ..Default::default()
        };
        for profile in 0..20 {
            preferences.histories.insert(
                profile,
                crate::redis_history::History {
                    commands: [format!("GET profile:{profile}")].into(),
                    updated_ms: profile,
                },
            );
        }
        preferences.sanitize();
        assert_eq!(
            preferences.histories.len(),
            crate::redis_history::MAX_PROFILES
        );
        assert!(!preferences.histories.contains_key(&0));
        assert!(preferences.histories.contains_key(&19));
        let restored: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&preferences).unwrap()).unwrap();
        assert!(!restored.record_history);
        assert_eq!(restored.histories, preferences.histories);
    }
    #[test]
    fn persisted_preferences_are_bounded_and_keep_named_snippets() {
        let mut p = Preferences {
            record_history: true,
            histories: Default::default(),
            tree: false,
            separator: "::::::::::".into(),
            snippets: vec![
                Snippet {
                    name: "Read profile".into(),
                    command: "GET profile".into()
                };
                25
            ],
        };
        p.sanitize();
        assert_eq!(p.separator.len(), 8);
        assert_eq!(p.snippets.len(), 20);
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert!(!restored.tree);
        assert_eq!(restored.snippets[0].command, "GET profile");
    }
}
