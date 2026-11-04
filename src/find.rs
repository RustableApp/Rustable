//! Literal SQL text search. Scans on actions; the displayed match count is cached.
use eframe::egui;
use regex::{NoExpand, Regex, RegexBuilder};
use std::{ops::Range, sync::Arc};

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    #[default]
    Contains,
    StartsWith,
    FullWord,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Options {
    pub ignore_case: bool,
    pub wrap: bool,
    pub mode: Mode,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            ignore_case: false,
            wrap: true,
            mode: Mode::Contains,
        }
    }
}

#[derive(Clone, Copy)]
pub enum Action {
    Next,
    Previous,
    Replace,
    All,
}
#[derive(Default)]
pub struct Search {
    pub open: bool,
    pub focus: bool,
    pub replacement_visible: bool,
    pub needle: String,
    pub replacement: String,
    pub action: Option<Action>,
    pub scroll: bool,
    pub options: Options,
}
impl Search {
    pub fn matcher(&self, ctx: &egui::Context) -> Result<Arc<Matcher>, String> {
        type Cache = egui::cache::FrameCache<Result<Arc<Matcher>, String>, Compiler>;
        ctx.memory_mut(|m| {
            m.caches
                .cache::<Cache>()
                .get((self.needle.as_str(), self.options))
        })
    }

    pub fn show(&mut self, ui: &mut egui::Ui, sql: &str) -> Option<Action> {
        let mut action = self.action.take();
        if !self.open {
            return action;
        }
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.needle)
                    .hint_text("Find")
                    .desired_width(180.0),
            );
            if self.focus {
                field.request_focus();
                self.focus = false;
            }
            if field.has_focus() || field.lost_focus() {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                    self.open = false;
                }
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter)) {
                    action = Some(Action::Previous);
                } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                    action = Some(Action::Next);
                }
            }
            if ui
                .add(crate::icons::widget("chevron-left"))
                .on_hover_text(format!("Previous · {}", crate::icons::shortcut(ui.ctx(), true, "G")))
                .clicked()
            {
                action = Some(Action::Previous);
            }
            if ui.add(crate::icons::widget("chevron-right")).on_hover_text(format!("Next · {}", crate::icons::shortcut(ui.ctx(), false, "G"))).clicked() {
                action = Some(Action::Next);
            }
            ui.menu_button("Options", |ui| {
                ui.checkbox(&mut self.options.ignore_case, "Ignore Case");
                ui.checkbox(&mut self.options.wrap, "Wrap Around");
                ui.separator();
                ui.selectable_value(&mut self.options.mode, Mode::Contains, "Contains");
                ui.selectable_value(&mut self.options.mode, Mode::StartsWith, "Starts With");
                ui.selectable_value(&mut self.options.mode, Mode::FullWord, "Full Word");
            });
            match self.matcher(ui.ctx()) {
                Ok(matcher) => {
                    ui.weak(format!("{} matches", match_count(ui.ctx(), sql, &matcher)));
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::RED, error);
                }
            }
            ui.checkbox(&mut self.replacement_visible, "Replace");
            if ui.button("Done").clicked() {
                self.open = false;
            }
        });
        if self.replacement_visible {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.replacement)
                        .hint_text("Replace with")
                        .desired_width(180.0),
                );
                ui.add_enabled_ui(!self.needle.is_empty(), |ui| {
                    if ui.button("Replace").clicked() {
                        action = Some(Action::Replace);
                    }
                    if ui
                        .button("All")
                        .on_hover_text("Replace all literal matches in this query tab")
                        .clicked()
                    {
                        action = Some(Action::All);
                    }
                });
            });
        }
        action
    }
}

pub struct Matcher {
    regex: Option<Regex>,
    options: Options,
}
impl std::hash::Hash for Matcher {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.options.hash(state);
        self.regex.as_ref().map(Regex::as_str).hash(state);
    }
}
impl Matcher {
    pub fn new(needle: &str, options: Options) -> Result<Self, String> {
        let escaped = regex::escape(needle);
        let pattern = match options.mode {
            Mode::Contains => escaped,
            Mode::StartsWith => format!(r"\b{escaped}"),
            Mode::FullWord => format!(r"\b{escaped}\b"),
        };
        let regex = if needle.is_empty() {
            None
        } else {
            Some(
                RegexBuilder::new(&pattern)
                    .case_insensitive(options.ignore_case)
                    .build()
                    .map_err(|_| "Search text is too large to compile".to_owned())?,
            )
        };
        Ok(Self { regex, options })
    }
}
#[derive(Default)]
struct Compiler;
impl egui::cache::ComputerMut<(&str, Options), Result<Arc<Matcher>, String>> for Compiler {
    fn compute(&mut self, (needle, options): (&str, Options)) -> Result<Arc<Matcher>, String> {
        Matcher::new(needle, options).map(Arc::new)
    }
}
#[derive(Default)]
struct Count;
impl egui::cache::ComputerMut<(&str, &Matcher), usize> for Count {
    fn compute(&mut self, (sql, matcher): (&str, &Matcher)) -> usize {
        matcher
            .regex
            .as_ref()
            .map(|r| r.find_iter(sql).count())
            .unwrap_or(0)
    }
}
fn match_count(ctx: &egui::Context, sql: &str, matcher: &Matcher) -> usize {
    type Cache = egui::cache::FrameCache<usize, Count>;
    ctx.memory_mut(|m| m.caches.cache::<Cache>().get((sql, matcher)))
}
fn byte_at(sql: &str, character: usize) -> usize {
    sql.char_indices()
        .nth(character)
        .map(|(i, _)| i)
        .unwrap_or(sql.len())
}
fn characters(sql: &str, bytes: Range<usize>) -> Range<usize> {
    let start = sql[..bytes.start].chars().count();
    start..start + sql[bytes].chars().count()
}
/// Returns character coordinates suitable for egui, wrapping at either end.
pub fn locate(
    sql: &str,
    matcher: &Matcher,
    cursor: Range<usize>,
    previous: bool,
) -> Option<Range<usize>> {
    let regex = matcher.regex.as_ref()?;
    let boundary = byte_at(sql, if previous { cursor.start } else { cursor.end });
    let mut first = None;
    let mut last = None;
    let mut candidate = None;
    for found in regex.find_iter(sql) {
        let range = found.range();
        if first.is_none() {
            first = Some(range.clone());
        }
        last = Some(range.clone());
        if previous {
            if range.end <= boundary {
                candidate = Some(range);
            }
        } else if range.start >= boundary {
            return Some(characters(sql, range));
        }
    }
    candidate
        .or({
            if matcher.options.wrap {
                if previous { last } else { first }
            } else {
                None
            }
        })
        .map(|r| characters(sql, r))
}

/// Replace only a selected full match; otherwise navigate to the next match.
/// The caller adds an editor undo point before invoking this operation.
pub fn replace(
    sql: &mut String,
    matcher: &Matcher,
    replacement: &str,
    cursor: Range<usize>,
    all: bool,
) -> Range<usize> {
    let Some(regex) = &matcher.regex else {
        return cursor;
    };
    if all {
        *sql = regex.replace_all(sql, NoExpand(replacement)).into_owned();
        let caret = cursor.start.min(sql.chars().count());
        return caret..caret;
    }
    let bytes = byte_at(sql, cursor.start)..byte_at(sql, cursor.end);
    if regex
        .find_at(sql, bytes.start)
        .is_some_and(|found| found.range() == bytes)
    {
        let caret = cursor.start + replacement.chars().count();
        sql.replace_range(bytes, replacement);
        locate(sql, matcher, caret..caret, false).unwrap_or(caret..caret)
    } else {
        locate(sql, matcher, cursor.clone(), false).unwrap_or(cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn locate(
        sql: &str,
        needle: &str,
        cursor: Range<usize>,
        previous: bool,
    ) -> Option<Range<usize>> {
        super::locate(
            sql,
            &Matcher::new(needle, Options::default()).unwrap(),
            cursor,
            previous,
        )
    }
    fn replace(
        sql: &mut String,
        needle: &str,
        replacement: &str,
        cursor: Range<usize>,
        all: bool,
    ) -> Range<usize> {
        super::replace(
            sql,
            &Matcher::new(needle, Options::default()).unwrap(),
            replacement,
            cursor,
            all,
        )
    }
    #[test]
    fn unicode_selection_and_bidirectional_wrap() {
        let sql = "🙂日本 x 日本";
        assert_eq!(locate(sql, "日本", 0..0, false), Some(1..3));
        assert_eq!(locate(sql, "日本", 1..3, false), Some(6..8));
        assert_eq!(locate(sql, "日本", 6..8, false), Some(1..3));
        assert_eq!(locate(sql, "日本", 1..3, true), Some(6..8));
        assert_eq!(locate(sql, "missing", 0..0, false), None);
        assert_eq!(locate(sql, "", 0..0, false), None);
    }
    #[test]
    fn replacements_are_literal_and_only_replace_selected_matches() {
        let mut sql = "é foo foo".to_owned();
        assert_eq!(replace(&mut sql, "foo", "$1", 0..0, false), 2..5);
        assert_eq!(sql, "é foo foo");
        assert_eq!(replace(&mut sql, "foo", "🙂", 2..5, false), 4..7);
        assert_eq!(sql, "é 🙂 foo");
        replace(&mut sql, "foo", "foofoo", 4..7, true);
        assert_eq!(sql, "é 🙂 foofoo");
        replace(&mut sql, "", "bad", 0..0, true);
        assert_eq!(sql, "é 🙂 foofoo");
    }
}

#[cfg(test)]
mod option_tests {
    use super::*;
    #[test]
    fn unicode_case_matching_preserves_original_coordinates_and_literal_replacement() {
        let matcher = Matcher::new(
            "été",
            Options {
                ignore_case: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut sql = "🙂 ÉTÉ été".to_owned();
        assert_eq!(locate(&sql, &matcher, 0..0, false), Some(2..5));
        assert_eq!(locate(&sql, &matcher, 2..5, false), Some(6..9));
        replace(&mut sql, &matcher, "$1", 2..5, true);
        assert_eq!(sql, "🙂 $1 $1");
        let sigma = Matcher::new(
            "σ",
            Options {
                ignore_case: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(sigma.regex.as_ref().unwrap().find_iter("Σ σ ς").count(), 3);
    }
    #[test]
    fn word_modes_respect_unicode_identifier_boundaries_and_selected_context() {
        let sql = "user users superuser _user user_id user";
        let full = Matcher::new(
            "user",
            Options {
                mode: Mode::FullWord,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(full.regex.as_ref().unwrap().find_iter(sql).count(), 2);
        let starts = Matcher::new(
            "user",
            Options {
                mode: Mode::StartsWith,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(starts.regex.as_ref().unwrap().find_iter(sql).count(), 4);
        let mut selected_prefix = "users user".to_owned();
        assert_eq!(
            replace(&mut selected_prefix, &full, "person", 0..4, false),
            6..10
        );
        assert_eq!(selected_prefix, "users user");
    }
    #[test]
    fn wrap_option_and_regex_metacharacters_are_literal() {
        let matcher = Matcher::new(
            "[x]",
            Options {
                wrap: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(locate("[x] x", &matcher, 0..0, false), Some(0..3));
        assert_eq!(locate("[x] x", &matcher, 0..3, false), None);
        assert_eq!(locate("[x] x", &matcher, 0..3, true), None);
    }
}
