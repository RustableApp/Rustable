use super::*;
use egui::text::{CCursor, CCursorRange};

struct Editor {
    ctx: egui::Context,
    text: String,
    selection: CCursorRange,
    tick: usize,
    enabled: bool,
    limit: usize,
}
impl Editor {
    fn new(text: &str, selection: std::ops::Range<usize>) -> Self {
        let mut editor = Self {
            ctx: egui::Context::default(),
            text: text.into(),
            selection: CCursorRange::two(
                CCursor::new(selection.start),
                CCursor::new(selection.end),
            ),
            tick: 0,
            enabled: true,
            limit: usize::MAX,
        };
        editor.frame(vec![]);
        let id = egui::Id::new("assist-editor");
        let mut state = egui::TextEdit::load_state(&editor.ctx, id).unwrap();
        state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(selection.start),
            CCursor::new(selection.end),
        )));
        state.store(&editor.ctx, id);
        editor.ctx.memory_mut(|m| m.request_focus(id));
        editor
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.tick += 1;
        self.ctx.run(
            egui::RawInput {
                events,
                time: Some(self.tick as f64 / 60.0),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 480.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let allowed =
                        |prefix: &str| parentheses_allowed(prefix, crate::sql::Dialect::Postgres);
                    let edit = egui::TextEdit::multiline(&mut self.text)
                        .id(egui::Id::new("assist-editor"))
                        .code_editor()
                        .char_limit(self.limit);
                    let edit = if self.enabled {
                        edit.auto_indent(true).parenthesis_pairing(&allowed)
                    } else {
                        edit
                    };
                    let output = edit.show(ui);
                    if let Some(selection) = output.cursor_range {
                        self.selection = selection;
                    }
                });
            },
        )
    }
    fn text(&mut self, s: &str) {
        self.frame(vec![egui::Event::Text(s.into())]);
    }
    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        self.frame(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
    }
    fn undo(&mut self) {
        self.key(egui::Key::Z, egui::Modifiers::COMMAND);
    }
    fn redo(&mut self) {
        self.key(
            egui::Key::Z,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        );
    }
}
#[test]
fn enter_retains_tabs_spaces_and_mixed_indentation_atomically() {
    for original in [
        "\t\tSELECT 1;",
        "    SELECT 1;",
        " \t  SELECT '雪';",
        "\t\t",
    ] {
        let end = original.chars().count();
        let mut e = Editor::new(original, end..end);
        e.tick += 1800; // First Enter after 30 seconds without input.
        e.key(egui::Key::Enter, egui::Modifiers::NONE);
        let indent: String = original
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .collect();
        assert_eq!(e.text, format!("{original}\n{indent}"));
        e.undo();
        assert_eq!(e.text, original);
        e.redo();
        assert_eq!(e.text, format!("{original}\n{indent}"));
    }
}
#[test]
fn pair_wrap_overtype_backspace_and_undo() {
    let mut e = Editor::new("SELECT ", 7..7);
    e.text("(");
    assert_eq!(e.text, "SELECT ()");
    assert_eq!(e.selection.primary.index, 8);
    e.text("(");
    assert_eq!(e.text, "SELECT (())");
    e.text(")");
    assert_eq!(e.text, "SELECT (())");
    assert_eq!(e.selection.primary.index, 10);
    e.text(")");
    assert_eq!(e.selection.primary.index, 11);
    e.undo();
    assert_eq!(e.text, "SELECT ()");
    e.undo();
    assert_eq!(e.text, "SELECT ");
    e.redo();
    assert_eq!(e.text, "SELECT ()");
    let mut e = Editor::new("雪 SELECT", 0..1);
    e.text("(");
    assert_eq!(e.text, "(雪) SELECT");
    assert_eq!(e.selection.as_sorted_char_range(), 1..2);
    e.undo();
    assert_eq!(e.text, "雪 SELECT");
    let mut e = Editor::new("()", 1..1);
    e.key(egui::Key::Backspace, egui::Modifiers::NONE);
    assert_eq!(e.text, "");
    e.undo();
    assert_eq!(e.text, "()");
}
#[test]
fn enter_between_parentheses_positions_inner_and_closing_lines() {
    let mut e = Editor::new("\tEXISTS ()", 9..9); // between ( and )
    e.key(egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(e.text, "\tEXISTS (\n\t\t\n\t)");
    assert_eq!(e.selection.primary.index, 12);
    e.undo();
    assert_eq!(e.text, "\tEXISTS ()");
}
#[test]
fn paste_and_plain_text_widgets_are_not_paired() {
    let mut e = Editor::new("", 0..0);
    e.frame(vec![egui::Event::Paste("SELECT (".into())]);
    assert_eq!(e.text, "SELECT (");
    let mut e = Editor::new("\tSELECT", 7..7);
    e.enabled = false;
    e.key(egui::Key::Enter, egui::Modifiers::NONE);
    e.text("(");
    assert_eq!(e.text, "\tSELECT\n(");
}
#[test]
fn dialect_guards_strings_comments_and_dollar_bodies() {
    use crate::sql::Dialect;
    for sql in [
        "SELECT '",
        "SELECT 'it''s (",
        "SELECT \"",
        "-- comment (",
        "/* outer /* inner */ (",
        "DO $$ BEGIN (",
        "SELECT $tag$(",
        "SELECT $雪$(",
        "SELECT E'a\\'(",
    ] {
        assert!(!parentheses_allowed(sql, Dialect::Postgres), "{sql}");
        let mut e = Editor::new(sql, sql.chars().count()..sql.chars().count());
        e.text("(");
        assert_eq!(e.text, format!("{sql}("));
    }
    for sql in [
        "SELECT 'x'",
        "-- comment\nSELECT ",
        "/* a /* b */ c */ SELECT ",
        "SELECT $$x$$, ",
        "SELECT $雪$x$雪$, ",
    ] {
        assert!(parentheses_allowed(sql, Dialect::Postgres), "{sql}");
    }
    let mode = crate::sql::mysql::Mode::default();
    assert!(!parentheses_allowed("# comment", Dialect::Mysql(mode)));
    assert!(!parentheses_allowed("SELECT 'a\\'(", Dialect::Mysql(mode)));
    assert!(parentheses_allowed(
        "SELECT 'a\\'(",
        Dialect::Mysql(crate::sql::mysql::Mode {
            no_backslash_escapes: true,
            ..mode
        })
    ));
    assert!(!parentheses_allowed("SELECT [name(", Dialect::Sqlite));
}

#[test]
fn limits_and_ime_do_not_partially_wrap_or_rewrite_input() {
    let mut e = Editor::new("雪x", 0..1);
    e.limit = 3;
    e.text("(");
    assert_eq!(e.text, "雪x");
    let mut e = Editor::new("\tX", 2..2);
    e.limit = 3;
    e.key(egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(e.text, "\tX");
    let mut e = Editor::new("", 0..0);
    e.frame(vec![egui::Event::Ime(egui::ImeEvent::Enabled)]);
    e.frame(vec![egui::Event::Ime(egui::ImeEvent::Preedit("(".into()))]);
    e.frame(vec![egui::Event::Ime(egui::ImeEvent::Commit("(".into()))]);
    e.frame(vec![egui::Event::Ime(egui::ImeEvent::Disabled)]);
    assert_eq!(e.text, "(");
}

#[test]
#[ignore = "release-mode editor input benchmark"]
fn editor_assist_input_benchmark() {
    for lines in [10, 1000] {
        let original = "-- context\n\tSELECT 1;\n".repeat(lines);
        let end = original.chars().count();
        let mut e = Editor::new(&original, end..end);
        let mut samples = Vec::new();
        for sample in 0..120 {
            let start = std::time::Instant::now();
            let output = e.frame(vec![egui::Event::Text("(".into())]);
            let _ = e.ctx.tessellate(output.shapes, output.pixels_per_point);
            let output = e.frame(vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            let _ = e.ctx.tessellate(output.shapes, output.pixels_per_point);
            e.undo();
            e.undo();
            assert_eq!(e.text, original);
            if sample >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        crate::ui_benchmark::report(&format!("editor/indent-pair-undo/{lines}"), &samples);
    }
}

#[test]
fn literal_typing_groups_normally_and_pair_deletion_is_atomic() {
    let mut e = Editor::new("SELECT '", 8..8);
    e.text("abc");
    e.text("(");
    e.text("def");
    e.undo();
    assert_eq!(e.text, "SELECT '");
    let mut e = Editor::new("x()", 2..2);
    e.key(egui::Key::Backspace, egui::Modifiers::NONE);
    e.key(egui::Key::Backspace, egui::Modifiers::NONE);
    assert_eq!(e.text, "");
    e.undo();
    assert_eq!(e.text, "x");
    e.undo();
    assert_eq!(e.text, "x()");
}
