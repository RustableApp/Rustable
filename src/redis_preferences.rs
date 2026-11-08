use serde::{Deserialize, Serialize};
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    pub tree: bool,
    pub separator: String,
    pub snippets: Vec<Snippet>,
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
        }
    }
}
impl Preferences {
    pub fn sanitize(&mut self) {
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
    fn persisted_preferences_are_bounded_and_keep_named_snippets() {
        let mut p = Preferences {
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
