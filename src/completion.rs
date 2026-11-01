//! Bounded, local SQL suggestions; never queries a database on the render thread.
use crate::{
    db::{Column, Object},
    sql::{Dialect, char_to_byte},
};
use eframe::egui;

#[derive(Default)]
pub struct Completion {
    items: Vec<String>,
    identifiers: std::collections::HashSet<String>,
    selected: usize,
    start: usize,
    end: usize,
    open: bool,
    menu_rect: Option<egui::Rect>,
    server_edits: std::collections::HashMap<String, (std::ops::Range<usize>, String)>,
    explicit_requested: bool,
    suppress_server: bool,
}
impl Completion {
    pub fn before(&mut self, ctx: &egui::Context, editor: egui::Id) -> Option<String> {
        if !ctx.memory(|m| m.has_focus(editor)) || !self.open {
            return None;
        }
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.open = false;
                self.suppress_server = true;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                self.selected = (self.selected + 1).min(self.items.len().saturating_sub(1));
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                self.selected = self.selected.saturating_sub(1);
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            {
                self.open = false;
                return self.items.get(self.selected).cloned();
            }
            None
        })
    }
    pub fn refresh(
        &mut self,
        sql: &str,
        cursor: usize,
        objects: &[Object],
        _columns: &[Column],
        explicit: bool,
        dialect: Dialect,
    ) {
        self.items.clear();
        self.suppress_server = false;
        self.explicit_requested = explicit;
        self.server_edits.clear();
        self.identifiers.clear();
        self.selected = 0;
        self.open = false;
        let byte = char_to_byte(sql, cursor);
        let before = &sql[..byte];
        // Suppress suggestions inside SQL strings, quoted identifiers and comments.
        if !local_code_position(before, dialect) {
            return;
        }
        let prefix = before
            .rsplit(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        let qualifier = before[..before.len() - prefix.len()]
            .trim_end()
            .strip_suffix('.')
            .and_then(|s| tokens_for_dialect(s, dialect).last().cloned());
        if prefix.chars().count() < 2 && !explicit && qualifier.is_none() {
            return;
        }
        self.start = cursor - prefix.chars().count();
        self.end = cursor;
        let prefix = prefix.to_lowercase();
        let range = crate::sql::execution_range_for_dialect(sql, cursor..cursor, false, dialect)
            .unwrap_or(0..sql.len());
        let references = scoped_relations(&sql[range.clone()], byte.saturating_sub(range.start), dialect);
        let relevant: Vec<_> = objects
            .iter()
            .filter(|object| {
                references.iter().any(|(table, alias)| {
                    let object_name = object.display_name.as_deref().unwrap_or(&object.name);
                    let relation_matches = table.eq_ignore_ascii_case(object_name)
                        || (dialect == Dialect::Sqlite
                            && object.schema.is_none()
                            && table.eq_ignore_ascii_case(&format!("main.{object_name}")))
                        || object.schema.as_ref().is_some_and(|schema| {
                            table.eq_ignore_ascii_case(&format!("{schema}.{object_name}"))
                        });
                    relation_matches
                        && qualifier.as_ref().is_none_or(|q| {
                            q.eq_ignore_ascii_case(alias)
                        })
                })
            })
            .collect();
        // Borrow candidates and stop after the visible limit. Large catalogs
        // should not allocate names or quoted SQL for every table on each key.
        let columns = relevant.iter().flat_map(|object| object.columns.iter()
            .map(|column| (column.name.as_str(), true, None)));
        let tables = objects.iter().filter(|object| qualifier.as_ref().is_none_or(|q| {
            object.schema.as_ref().is_some_and(|schema| schema.eq_ignore_ascii_case(q))
        })).map(|object| (object.display_name.as_deref().unwrap_or(&object.name), true, Some(object)));
        let schemas = objects.iter().take(if qualifier.is_none() { objects.len() } else { 0 })
            .filter_map(|object| object.schema.as_deref()).map(|schema| (schema, true, None));
        let keywords = KEYWORDS.iter().filter(|_| qualifier.is_none()).map(|word| (*word, false, None));
        for (name, identifier, table) in columns.chain(tables).chain(schemas).chain(keywords) {
            if name.to_lowercase().starts_with(&prefix)
                && (!name.eq_ignore_ascii_case(&prefix)
                    || (identifier && identifier_needs_quotes(name)))
                && !self.items.iter().any(|item| item == name)
            {
                let replacement = table.and_then(|object| {
                    if matches!(dialect, Dialect::Mysql(_)) {
                        Some(match (&qualifier, &object.schema) {
                            (None, Some(schema)) => format!("{}.{}", mysql_identifier(schema), mysql_identifier(name)),
                            _ => mysql_identifier(name),
                        })
                    } else { object.schema.as_ref().map(|_| {
                        if qualifier.is_some() { format!("\"{}\"", name.replace('"', "\"\"")) }
                        else { object.name.clone() }
                    }) }
                });
                let name = name.to_owned();
                self.items.push(name.clone());
                if identifier {
                    self.identifiers.insert(name.clone());
                }
                let replacement = replacement.or_else(|| {
                    if identifier && matches!(dialect, Dialect::Mysql(_)) {
                        return Some(mysql_identifier(&name));
                    }
                    (identifier
                        && dialect == Dialect::Postgres
                        && name.chars().any(char::is_uppercase))
                    .then(|| format!("\"{}\"", name.replace('"', "\"\"")))
                });
                if let Some(replacement) = replacement {
                    self.server_edits
                        .insert(name, (self.start..self.end, replacement));
                }
                if self.items.len() == 12 {
                    break;
                }
            }
        }
        self.open = !self.items.is_empty();
    }
    pub fn apply(&mut self, sql: &mut String, value: &str) -> usize {
        self.open = false;
        self.suppress_server = true;
        if let Some((range, replacement)) = self.server_edits.get(value) {
            let start = char_to_byte(sql, range.start);
            let end = char_to_byte(sql, range.end);
            sql.replace_range(start..end, replacement);
            return range.start + replacement.chars().count();
        }
        let start = char_to_byte(sql, self.start);
        let end = char_to_byte(sql, self.end);
        let replacement = if self.identifiers.contains(value) && identifier_needs_quotes(value) {
            format!("\"{}\"", value.replace('"', "\"\""))
        } else {
            value.to_owned()
        };
        sql.replace_range(start..end, &replacement);
        self.start + replacement.chars().count()
    }

    pub fn receive_server(&mut self, sql: &str, cursor: usize, items: &[serde_json::Value]) {
        if self.suppress_server {
            return;
        }
        let before = &sql[..char_to_byte(sql, cursor)];
        let quoted_identifier = !code_position(before);
        if quoted_identifier && !completion_position(before, true) {
            return;
        }
        let prefix = before
            .rsplit(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        if prefix.chars().count() < 2
            && !before.ends_with('.')
            && !self.open
            && !self.explicit_requested
            && !quoted_identifier
        {
            return;
        }
        let default_range = cursor - prefix.chars().count()..cursor;
        let mut accepted = Vec::new();
        for item in items.iter().take(200) {
            // Snippets and compound edits need a dedicated editor transaction.
            // Keep catalog suggestions when we cannot safely apply an LSP item.
            if item
                .get("insertTextFormat")
                .and_then(serde_json::Value::as_u64)
                == Some(2)
                || item
                    .get("additionalTextEdits")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|edits| !edits.is_empty())
            {
                continue;
            }
            let Some(label) = item.get("label").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let (range, text) = if let Some(edit) = item.get("textEdit") {
                let Some(range) = edit
                    .get("range")
                    .or_else(|| edit.get("replace"))
                    .and_then(|range| crate::editor_language::range(sql, range))
                else {
                    continue;
                };
                let Some(text) = edit.get("newText").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                (range, text)
            } else {
                (
                    default_range.clone(),
                    item.get("insertText")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(label),
                )
            };
            // Inside a quoted identifier, accept only a complete token edit;
            // label-only or prefix-only inserts can duplicate quotes/suffixes.
            if quoted_identifier
                && (item.get("textEdit").is_none()
                    || !sql[char_to_byte(sql, range.start)..].starts_with('"')
                    || quoted_token_end(sql, char_to_byte(sql, range.start))
                        != Some(char_to_byte(sql, range.end))
                    || !text.starts_with('"')
                    || !text.ends_with('"'))
            {
                continue;
            }
            if range.start > cursor || range.end < cursor || text.len() > 65536 {
                continue;
            }
            if !accepted.iter().any(|entry: &String| entry == label) {
                self.server_edits.insert(label.into(), (range, text.into()));
                accepted.push(label.into());
            }
        }
        if accepted.is_empty() {
            return;
        }
        if quoted_identifier {
            // Local keyword candidates belong to the previous unquoted context.
            // Only validated whole-token server edits can be used inside quotes.
            self.items.clear();
            self.identifiers.clear();
        }
        self.start = default_range.start;
        self.end = cursor;
        // Some servers return broad/fuzzy catalog lists even for a keyword
        // prefix. Keep exact-prefix local suggestions ahead of those results,
        // then rank server prefix matches without changing their text edits.
        let prefix = prefix.to_lowercase();
        let mut merged = Vec::new();
        if !prefix.is_empty() {
            merged.extend(
                self.items
                    .iter()
                    .filter(|item| item.to_lowercase().starts_with(&prefix))
                    .cloned(),
            );
            accepted.sort_by_key(|item| !item.to_lowercase().starts_with(&prefix));
        }
        for item in accepted.into_iter().chain(self.items.iter().cloned()) {
            if merged.len() >= 12 {
                break;
            }
            if !merged.contains(&item) {
                merged.push(item);
            }
        }
        self.items = merged;
        self.selected = 0;
        self.open = true;
    }
    pub fn menu(
        &mut self,
        ctx: &egui::Context,
        id: egui::Id,
        position: egui::Pos2,
    ) -> Option<String> {
        if !self.open {
            return None;
        }
        let mut chosen = None;
        let screen = ctx.content_rect().shrink(4.0);
        let font = egui::FontId::monospace(13.0);
        let measured = ctx.fonts_mut(|fonts| {
            self.items
                .iter()
                .map(|item| {
                    fonts
                        .layout_no_wrap(item.clone(), font.clone(), egui::Color32::WHITE)
                        .size()
                        .x
                })
                .fold(28.0_f32, f32::max)
        });
        // Include row padding and frame borders, but no phantom shortcut column.
        let width = (measured + 14.0)
            .min(360.0)
            .min((screen.width() - 8.0).max(1.0));
        let height = self.items.len() as f32 * 20.0 + 8.0;
        let position = egui::pos2(
            position.x.clamp(
                screen.left(),
                (screen.right() - width - 8.0).max(screen.left()),
            ),
            position
                .y
                .clamp(screen.top(), (screen.bottom() - height).max(screen.top())),
        );
        let popup = egui::Area::new(id.with("completion"))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .constrain_to(screen)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(3.0)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (index, item) in self.items.iter().enumerate() {
                            let (rect, response) = ui
                                .allocate_exact_size(egui::vec2(width, 20.0), egui::Sense::click());
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::SelectableLabel,
                                    true,
                                    index == self.selected,
                                    item,
                                )
                            });
                            let visuals = ui
                                .style()
                                .interact_selectable(&response, index == self.selected);
                            if index == self.selected || response.hovered() {
                                ui.painter().rect_filled(rect, 3.0, visuals.weak_bg_fill);
                            }
                            let mut job = egui::text::LayoutJob::simple_singleline(
                                item.clone(),
                                font.clone(),
                                visuals.text_color(),
                            );
                            job.wrap.max_width = (width - 12.0).max(1.0);
                            job.wrap.max_rows = 1;
                            job.wrap.break_anywhere = true;
                            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                            let pos = egui::pos2(
                                rect.left() + 6.0,
                                rect.center().y - galley.size().y * 0.5,
                            );
                            ui.painter().galley(pos, galley, visuals.text_color());
                            if response.on_hover_text(item).clicked() {
                                chosen = Some(item.clone());
                            }
                        }
                    });
            });
        self.menu_rect = Some(popup.response.rect);
        if chosen.is_some() {
            self.open = false;
        }
        chosen
    }
    pub fn pointer_over_menu(&self, ctx: &egui::Context) -> bool {
        self.open
            && self.menu_rect.is_some_and(|rect| {
                ctx.input(|input| {
                    input
                        .pointer
                        .interact_pos()
                        .is_some_and(|pos| rect.contains(pos))
                })
            })
    }

    pub fn cursor_changed(&mut self, cursor: std::ops::Range<usize>) {
        if cursor.start != self.end || cursor.end != self.end {
            self.open = false;
            self.explicit_requested = false;
            // A new LSP response may arrive without a text-change refresh after
            // arrow/click navigation. Never merge candidates or replacement
            // ranges from the previous caret context into that response.
            self.items.clear();
            self.identifiers.clear();
            self.server_edits.clear();
            self.selected = 0;
        }
    }
    pub fn dismiss(&mut self) {
        self.open = false;
        self.explicit_requested = false;
    }
}
// Use the bundled engine's keyword inventory rather than the smaller suggestion
// list. This does not open a connection or perform database work.
fn mysql_identifier(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn identifier_needs_quotes(name: &str) -> bool {
    let mut chars = name.chars();
    if !chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        || !chars.all(|c| c.is_alphanumeric() || c == '_')
    {
        return true;
    }
    let Ok(length) = i32::try_from(name.len()) else {
        return true;
    };
    // SAFETY: SQLite reads exactly length bytes from this live UTF-8 slice;
    // the API does not require a NUL terminator or retain the pointer.
    unsafe { rusqlite::ffi::sqlite3_keyword_check(name.as_ptr().cast(), length) != 0 }
}

// Tokenize incomplete SQL without treating strings or comments as table references.
fn tokens(sql: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '-' && chars.peek() == Some(&'-') {
            for c in chars.by_ref() {
                if c == '\n' {
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
        } else if matches!(c, '\'' | '"' | '`' | '[') {
            let end = if c == '[' { ']' } else { c };
            let mut value = String::new();
            while let Some(c) = chars.next() {
                if c == end {
                    if chars.peek() == Some(&end) {
                        chars.next();
                        value.push(c);
                    } else {
                        break;
                    }
                } else {
                    value.push(c);
                }
            }
            if c != '\'' {
                result.push(value);
            }
        } else if c.is_alphanumeric() || c == '_' {
            let mut value = c.to_string();
            while chars
                .peek()
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
            {
                value.push(chars.next().unwrap());
            }
            result.push(value);
        } else if !c.is_whitespace() {
            result.push(c.to_string());
        }
    }
    result
}
fn tokens_for_dialect(sql: &str, dialect: Dialect) -> Vec<String> {
    relation_tokens(sql, dialect).into_iter().map(|(text, _)| text).collect()
}
fn relation_tokens(sql: &str, dialect: Dialect) -> Vec<(String, bool)> {
    let Dialect::Mysql(mode) = dialect else { return tokens(sql).into_iter().map(|word| (word, false)).collect(); };
    use sqlparser::tokenizer::Tokenizer;
    let lexer = crate::sql::mysql::Lexer(mode);
    let Ok(tokens) = Tokenizer::new(&lexer, sql).tokenize() else { return Vec::new(); };
    tokens.into_iter().filter_map(relation_token).collect()
}
fn relation_token(token: sqlparser::tokenizer::Token) -> Option<(String, bool)> {
    use sqlparser::tokenizer::Token;
    match token {
        Token::Word(word) => Some((word.value, word.quote_style.is_some())),
        Token::Period => Some((".".into(), false)),
        Token::Comma => Some((",".into(), false)),
        Token::LParen => Some(("(".into(), false)),
        Token::RParen => Some((")".into(), false)),
        Token::SemiColon => Some((";".into(), false)),
        _ => None,
    }
}
#[cfg(test)]
fn relations(sql: &str) -> Vec<(String, String)> {
    relations_for_dialect(sql, Dialect::Sqlite)
}
// Resolve the caret's query block first, then correlated outer blocks. Child
// subqueries must not contribute aliases to their parents or sibling queries.
fn scoped_relations(sql: &str, cursor: usize, dialect: Dialect) -> Vec<(String, String)> {
    let Dialect::Mysql(mode) = dialect else { return relations_for_dialect(sql, dialect); };
    use sqlparser::tokenizer::{Token, Tokenizer};
    let lexer = crate::sql::mysql::Lexer(mode);
    let Ok(tokens) = Tokenizer::new(&lexer, sql).tokenize_with_location() else { return Vec::new(); };
    let tokens: Vec<_> = tokens.into_iter().filter(|t| !matches!(t.token, Token::Whitespace(_))).collect();
    let before = &sql[..cursor.min(sql.len())];
    let location = (before.bytes().filter(|b| *b == b'\n').count() as u64 + 1,
        before.rsplit('\n').next().unwrap_or("").chars().count() as u64 + 1);
    let caret = tokens.iter().position(|t| (t.span.start.line, t.span.start.column) >= location).unwrap_or(tokens.len());
    let mut blocks = vec![(0, tokens.len())];
    let mut stack = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        match token.token {
            Token::LParen => stack.push(i),
            Token::RParen => if let Some(start) = stack.pop()
                && tokens.get(start + 1).is_some_and(|t| matches!(&t.token, Token::Word(w) if w.quote_style.is_none() && (w.value.eq_ignore_ascii_case("SELECT") || w.value.eq_ignore_ascii_case("WITH")))) {
                blocks.push((start + 1, i));
            },
            _ => {}
        }
    }
    // Incomplete subqueries are common while typing.
    for start in stack {
        if tokens.get(start + 1).is_some_and(|t| matches!(&t.token, Token::Word(w) if w.quote_style.is_none() && (w.value.eq_ignore_ascii_case("SELECT") || w.value.eq_ignore_ascii_case("WITH")))) {
            blocks.push((start + 1, tokens.len()));
        }
    }
    let mut containing: Vec<_> = blocks.iter().copied().filter(|(start, end)| *start <= caret && caret <= *end).collect();
    containing.sort_by_key(|(start, end)| end - start);
    let mut result: Vec<(String, String)> = Vec::new();
    for (start, end) in containing {
        // Set-operation arms have independent relation namespaces. Split only
        // at this block's top level, leaving nested expressions/subqueries alone.
        let mut arm_start = start;
        let mut arm_end = end;
        let mut depth = 0usize;
        for (i, token) in tokens.iter().enumerate().take(end).skip(start) {
            match &token.token {
                Token::LParen => depth += 1,
                Token::RParen => depth = depth.saturating_sub(1),
                Token::Word(word) if depth == 0 && word.quote_style.is_none()
                    && ["UNION", "INTERSECT", "EXCEPT"].iter().any(|op| word.value.eq_ignore_ascii_case(op)) => {
                    if i < caret { arm_start = i + 1; } else { arm_end = i; break; }
                }
                _ => {}
            }
        }
        let mut lexemes = Vec::new();
        for (i, token) in tokens.iter().enumerate().take(arm_end).skip(arm_start) {
            if blocks.iter().any(|&(child_start, child_end)| child_start > start && child_end <= end && child_start <= i && i < child_end) { continue; }
            if let Some(lexeme) = relation_token(token.token.clone()) { lexemes.push(lexeme); }
        }
        for relation in relations_from_tokens(lexemes) {
            if !result.iter().any(|(_, alias)| alias.eq_ignore_ascii_case(&relation.1)) { result.push(relation); }
        }
    }
    result
}

fn relations_for_dialect(sql: &str, dialect: Dialect) -> Vec<(String, String)> {
    relations_from_tokens(relation_tokens(sql, dialect))
}
fn relations_from_tokens(lexemes: Vec<(String, bool)>) -> Vec<(String, String)> {
    let words = lexemes.iter().map(|(text, _)| text.clone()).collect::<Vec<_>>();
    let mut result = Vec::new();
    let mut i = 0;
    // A comma introduces another relation only within the FROM list at its
    // own nesting level, never inside a function, tuple, or later clause.
    let mut from_lists = vec![false];
    while i < words.len() {
        // Quoted identifiers can spell keywords or punctuation. They do not
        // introduce clauses, joins or nesting while scanning the query body.
        if lexemes[i].1 { i += 1; continue; }
        match words[i].as_str() {
            "(" => {
                from_lists.push(false);
                i += 1;
                continue;
            }
            ")" => {
                if from_lists.len() > 1 {
                    from_lists.pop();
                }
                i += 1;
                continue;
            }
            _ => {}
        }
        if [
            "WHERE",
            "GROUP",
            "ORDER",
            "HAVING",
            "WINDOW",
            "LIMIT",
            "OFFSET",
            "UNION",
            "EXCEPT",
            "INTERSECT",
            "RETURNING",
            "SET",
            "VALUES",
            ";",
        ]
        .iter()
        .any(|k| words[i].eq_ignore_ascii_case(k))
        {
            *from_lists.last_mut().unwrap() = false;
        }
        let comma_relation = words[i] == "," && *from_lists.last().unwrap();
        if words[i].eq_ignore_ascii_case("FROM") {
            *from_lists.last_mut().unwrap() = true;
        }
        if comma_relation
            || ["FROM", "JOIN", "UPDATE", "INTO"]
                .iter()
                .any(|k| words[i].eq_ignore_ascii_case(k))
        {
            i += 1;
            if i >= words.len() || (words[i] == "(" && !lexemes[i].1) {
                continue;
            }
            let mut table = words[i].clone();
            i += 1;
            let mut default_alias = table.clone();
            if words.get(i).is_some_and(|w| w == ".") && !lexemes[i].1 && i + 1 < words.len() {
                default_alias = words[i + 1].clone();
                table = format!("{}.{}", table, words[i + 1]);
                i += 2;
            }
            if words.get(i).is_some_and(|w| w.eq_ignore_ascii_case("AS")) && !lexemes[i].1 {
                i += 1;
            }
            let alias = words
                .get(i)
                .filter(|w| {
                    lexemes[i].1 || (w.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && ![
                            "WHERE",
                            "JOIN",
                            "LEFT",
                            "RIGHT",
                            "INNER",
                            "OUTER",
                            "CROSS",
                            "FULL",
                            "ON",
                            "SET",
                            "VALUES",
                            "GROUP",
                            "ORDER",
                            "LIMIT",
                            "OFFSET",
                            "UNION",
                            "RETURNING",
                            "HAVING",
                            "WINDOW",
                            "EXCEPT",
                            "INTERSECT",
                            "USING",
                            "NATURAL",
                        ]
                        .iter()
                        .any(|k| w.eq_ignore_ascii_case(k)))
                })
                .cloned()
                .unwrap_or(default_alias);
            result.push((table, alias));
        } else {
            i += 1;
        }
    }
    result
}
const KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "ORDER BY",
    "GROUP BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "INSERT INTO",
    "UPDATE",
    "DELETE FROM",
    "VALUES",
    "SET",
    "JOIN",
    "LEFT JOIN",
    "INNER JOIN",
    "ON",
    "AS",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "IS NULL",
    "DISTINCT",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "COALESCE",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "CREATE TABLE",
    "ALTER TABLE",
    "DROP TABLE",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "EXPLAIN",
    "WITH",
    "UNION",
];
fn quoted_token_end(sql: &str, start: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if bytes.get(i + 1) == Some(&b'"') {
                i += 2;
                continue;
            }
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

fn local_code_position(sql: &str, dialect: Dialect) -> bool {
    let Dialect::Mysql(mode) = dialect else { return code_position(sql); };
    // A sentinel remains a Word only when the caret is outside a literal,
    // identifier or comment. Use the same mode-aware lexer as Beautify.
    let input = format!("{sql}__rustable_caret__");
    let lexer = crate::sql::mysql::Lexer(mode);
    let Ok(tokens) = sqlparser::tokenizer::Tokenizer::new(&lexer, &input).tokenize() else { return false; };
    matches!(tokens.last(), Some(sqlparser::tokenizer::Token::Word(word)) if word.value.ends_with("__rustable_caret__"))
}

fn code_position(sql: &str) -> bool {
    completion_position(sql, false)
}
fn completion_position(sql: &str, allow_quoted_identifier: bool) -> bool {
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' | '`' | '[' => {
                let end = if c == '[' { ']' } else { c };
                let mut closed = false;
                while let Some(c) = chars.next() {
                    if c == end {
                        if end != ']' && chars.peek() == Some(&end) {
                            chars.next();
                        } else {
                            closed = true;
                            break;
                        }
                    }
                }
                if !closed {
                    return c == '"' && allow_quoted_identifier;
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                if !chars.by_ref().any(|c| c == '\n') {
                    return false;
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    if c == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}
#[cfg(test)]
mod tests {
    #[test]
    fn server_quoted_identifier_edits_replace_full_token_only() {
        let mut completion = Completion::default();
        let mut sql = "SELECT c.\"Disuffix\" FROM customers c".to_owned();
        let cursor = 12;
        completion.refresh(&sql, cursor, &[], &[], false, Dialect::Postgres);
        completion.receive_server(&sql, cursor, &[serde_json::json!({
            "label":"Display Name", "textEdit":{"range":{"start":{"line":0,"character":9},"end":{"line":0,"character":18}},"newText":"\"Display Name\""}
        })]);
        assert!(
            !completion.open,
            "Partial quoted edits must not leave a closing quote behind"
        );
        completion.items.push("SELECT".into());
        completion.receive_server(&sql, cursor, &[serde_json::json!({
            "label":"Display Name", "textEdit":{"range":{"start":{"line":0,"character":9},"end":{"line":0,"character":19}},"newText":"\"Display Name\""}
        })]);
        assert!(completion.open);
        assert_eq!(completion.items, ["Display Name"]);
        completion.apply(&mut sql, "Display Name");
        assert_eq!(sql, "SELECT c.\"Display Name\" FROM customers c");
        for text in ["SELECT 'Dis", "-- \"Dis", "/* \"Dis"] {
            completion.refresh(
                text,
                text.chars().count(),
                &[],
                &[],
                true,
                Dialect::Postgres,
            );
            completion.receive_server(
                text,
                text.chars().count(),
                &[serde_json::json!({"label":"Display Name"})],
            );
            assert!(!completion.open, "{text}");
        }
        completion.refresh("SELECT \"Dis", 11, &[], &[], true, Dialect::Postgres);
        completion.receive_server(
            "SELECT \"Dis",
            11,
            &[serde_json::json!({"label":"Display Name"})],
        );
        assert!(!completion.open); // No unsafe label-only insertion inside quotes.
    }

    #[test]
    fn accepting_completion_blocks_late_results_until_user_edits_or_requests_again() {
        let mut completion = super::Completion::default();
        let mut sql = "SEL".to_owned();
        completion.refresh(&sql, 3, &[], &[], false, crate::sql::Dialect::Sqlite);
        completion.apply(&mut sql, "SELECT");
        let items = [serde_json::json!({"label":"SELECT"})];
        completion.receive_server(&sql, 6, &items);
        assert!(!completion.open);
        completion.refresh(&sql, 6, &[], &[], true, crate::sql::Dialect::Sqlite);
        completion.receive_server(&sql, 6, &items);
        assert!(completion.open);
    }
    #[test]
    fn server_completion_converts_utf16_edits_without_touching_unicode_prefix() {
        let mut completion = super::Completion::default();
        let mut sql = "-- 😀\nSELECT na".to_owned();
        let cursor = sql.chars().count();
        completion.receive_server(&sql, cursor, &[serde_json::json!({
            "label":"name", "textEdit":{"range":{"start":{"line":1,"character":7},"end":{"line":1,"character":9}},"newText":"\"name\""}
        })]);
        let next = completion.apply(&mut sql, "name");
        assert_eq!(sql, "-- 😀\nSELECT \"name\"");
        assert_eq!(next, sql.chars().count());
    }

    #[test]
    fn cursor_move_discards_previous_context_before_new_server_results() {
        let mut completion = Completion::default();
        completion.refresh("SE", 2, &[], &[], false, Dialect::Postgres);
        assert!(completion.items.contains(&"SELECT".into()));
        // Arrow navigation requests a new LSP context without a text-change
        // refresh. Previous keyword items and their edits must not be merged.
        let sql = "WITH RECURSIVE walk(step) AS (VALUES (1) UNION ALL SELECT w.st FROM walk w) SELECT * FROM walk";
        let cursor = sql.find("w.st").unwrap() + 4;
        completion.cursor_changed(cursor..cursor);
        completion.receive_server(sql, cursor, &[serde_json::json!({"label":"step", "insertText":"step"})]);
        assert_eq!(completion.items, vec!["step"]);
        let mut applied = sql.to_owned();
        completion.apply(&mut applied, "step");
        assert!(applied.contains("SELECT w.step FROM walk"));
        // Same-position observations must not discard an active popup.
        completion.refresh("SE", 2, &[], &[], false, Dialect::Postgres);
        completion.cursor_changed(2..2);
        assert!(completion.open && completion.items.contains(&"SELECT".into()));
    }

    #[test]
    fn fuzzy_server_results_cannot_crowd_out_local_prefix_matches() {
        let mut completion = Completion::default();
        completion.refresh("SE", 2, &[], &[], false, Dialect::Sqlite);
        let mut server: Vec<_> = (0..20)
            .map(|i| serde_json::json!({"label":format!("customer_{i}")}))
            .collect();
        server
            .push(serde_json::json!({"label":"server_keyword", "insertText":"server_replacement"}));
        completion.receive_server("SE", 2, &server);
        assert_eq!(&completion.items[..2], &["SELECT", "SET"]);
        assert_eq!(completion.items[2], "server_keyword");
        assert_eq!(completion.items.len(), 12);
        let mut sql = "SE".to_owned();
        completion.apply(&mut sql, "server_keyword");
        assert_eq!(sql, "server_replacement");
        // Explicit completion without a prefix keeps upstream ranking.
        completion.refresh("", 0, &[], &[], true, Dialect::Sqlite);
        completion.receive_server("", 0, &server);
        assert_eq!(completion.items[0], "customer_0");
    }

    #[test]
    fn unsupported_server_edits_preserve_local_suggestions() {
        let mut completion = super::Completion::default();
        completion.refresh("SEL", 3, &[], &[], false, crate::sql::Dialect::Sqlite);
        let before = completion.items.clone();
        completion.receive_server("SEL", 3, &[
            serde_json::json!({"label":"snippet", "insertTextFormat":2, "insertText":"${1:SELECT}"}),
            serde_json::json!({"label":"bad", "textEdit":{"range":{"start":{"line":90,"character":0},"end":{"line":90,"character":1}},"newText":"bad"}}),
        ]);
        assert_eq!(completion.items, before);
    }
    use super::*;
    fn object(name: &str, fields: &[&str]) -> Object {
        Object {
            schema: None,
            display_name: None,
            metadata_error: None,
            name: name.into(),
            kind: "table".into(),
            ddl: String::new(),
            columns: fields
                .iter()
                .map(|name| Column {
                    name: (*name).into(),
                    data_type: "TEXT".into(),
                    not_null: false,
                    default: String::new(),
                    pk: false,
                    generated: false,
                })
                .collect(),
        }
    }
    #[test]
    fn mysql_completion_respects_comments_and_session_string_escapes() {
        let normal = Dialect::Mysql(crate::sql::mysql::Mode::default());
        let no_escape = Dialect::Mysql(crate::sql::mysql::Mode { no_backslash_escapes: true, ansi_quotes: false });
        for sql in ["SELECT 1 # FR", "SELECT 1 -- FR", "SELECT /* FR", "SELECT 'FR", "SELECT `FR", "SELECT \"FR"] {
            assert!(!local_code_position(sql, normal), "{sql}");
        }
        assert!(local_code_position("SELECT 1 # ignored\n FR", normal));
        assert!(local_code_position("SELECT 1--2 + FR", normal));
        assert_eq!(relations_for_dialect("SELECT 'escaped \\' FROM fake' FROM customers # JOIN bogus\n", normal), vec![("customers".into(), "customers".into())]);
        assert_eq!(relations_for_dialect("SELECT \"FROM fake\" FROM customers", normal), vec![("customers".into(), "customers".into())]);
        assert!(!local_code_position("SELECT 'escaped \\' FR", normal));
        assert!(local_code_position("SELECT 'escaped \\' FR", no_escape));
        assert!(!local_code_position("SELECT 'escaped \\' FR", Dialect::Mysql(crate::sql::mysql::Mode { no_backslash_escapes: false, ansi_quotes: true })));
    }

    #[test]
    #[ignore = "manual large-catalog completion timing"]
    fn large_catalog_completion_timing() {
        let mut objects: Vec<_> = (0..20_000).map(|i| {
            let mut table = object(&format!("table_{i:05}"), &["id", "customer_name", "order_total"]);
            table.schema = Some("public".into());
            table
        }).collect();
        objects.push(object("customers", &["customer_name"]));
        let mut completion = Completion::default();
        for sql in ["SELECT * FROM ta", "SELECT c.cu FROM customers c", "SELECT nonexistent"] {
            let cursor = sql.find(" FROM customers").unwrap_or(sql.len());
            let mut samples = Vec::new();
            for _ in 0..50 {
                let start = std::time::Instant::now();
                completion.refresh(sql, cursor, &objects, &[], true, Dialect::Mysql(crate::sql::mysql::Mode::default()));
                samples.push(start.elapsed());
                assert!(completion.items.len() <= 12);
            }
            samples.sort();
            eprintln!("{sql}: median={:?}, p95={:?}", samples[25], samples[47]);
            if sql.ends_with(" ta") { assert_eq!(completion.items.len(), 12); }
            if sql.contains("c.cu") { assert_eq!(completion.items, vec!["customer_name"]); }
        }
    }

    #[test]
    fn mysql_completion_isolates_set_operation_arms() {
        let objects = [object("customers", &["customer_name"]), object("orders", &["order_total"])];
        for operator in ["UNION", "UNION ALL", "INTERSECT", "EXCEPT"] {
            for (marked, expected) in [
                (format!("SELECT x.| FROM customers x {operator} SELECT x.order_total FROM orders x"), vec!["customer_name"]),
                (format!("SELECT x.customer_name FROM customers x {operator} SELECT x.| FROM orders x"), vec!["order_total"]),
                (format!("SELECT c.customer_name FROM customers c {operator} SELECT c.| FROM orders o"), vec![]),
                (format!("SELECT c.| FROM customers c WHERE EXISTS (SELECT 1 FROM orders o {operator} SELECT 1 FROM customers x)"), vec!["customer_name"]),
                (format!("SELECT 1 FROM customers c WHERE EXISTS (SELECT x.customer_name FROM customers x {operator} SELECT x.| FROM orders x)"), vec!["order_total"]),
                (format!("SELECT 1 FROM customers c {operator} SELECT 1 FROM orders o WHERE EXISTS (SELECT c.| FROM orders x)"), vec![]),
            ] {
                let cursor = marked.find('|').unwrap();
                let sql = marked.replace('|', "");
                let mut completion = Completion::default();
                completion.refresh(&sql, cursor, &objects, &[], true, Dialect::Mysql(crate::sql::mysql::Mode::default()));
                assert_eq!(completion.items, expected, "{sql}");
            }
        }
        let dialect = Dialect::Mysql(crate::sql::mysql::Mode::default());
        assert_eq!(scoped_relations("SELECT `UNION`.customer_name FROM customers `UNION`", 15, dialect), vec![("customers".into(), "UNION".into())]);
    }

    #[test]
    fn explicit_alias_hides_table_name_and_wins_over_catalog_names() {
        let objects = [object("customers", &["customer_name"]),object("orders", &["order_total"])];
        for dialect in [Dialect::Sqlite,Dialect::Postgres,Dialect::Mysql(crate::sql::mysql::Mode::default())] {
            for (marked,expected) in [
                ("SELECT orders.| FROM customers AS orders JOIN orders AS o ON 1=1",vec!["customer_name"]),
                ("SELECT customers.| FROM customers AS c",vec![]),
                ("SELECT customers.| FROM customers",vec!["customer_name"]),
                ("SELECT o.| FROM customers AS orders JOIN orders AS o ON 1=1",vec!["order_total"]),
            ] {
                let cursor = marked.find('|').unwrap();
                let sql = marked.replace('|', "");
                let mut completion = Completion::default();
                completion.refresh(&sql,cursor,&objects,&[],true,dialect);
                assert_eq!(completion.items,expected,"{dialect:?}: {sql}");
            }
        }
        let sql = "SELECT * FROM customers c WHERE EXISTS (SELECT customers. FROM orders customers)";
        let cursor = sql.find("customers. FROM").unwrap()+"customers.".len();
        let mut completion = Completion::default();
        completion.refresh(sql,cursor,&objects,&[],true,Dialect::Mysql(crate::sql::mysql::Mode::default()));
        assert_eq!(completion.items,["order_total"],"inner alias must hide outer table's catalog name");
    }

    #[test]
    fn mysql_completion_isolates_nested_aliases_and_keeps_correlations() {
        let objects = [object("customers", &["customer_name"]), object("orders", &["order_total"])];
        for (marked, expected) in [
            ("SELECT x.| FROM customers x WHERE EXISTS (SELECT x.order_total FROM orders x)", vec!["customer_name"]),
            ("SELECT * FROM customers x WHERE EXISTS (SELECT x.| FROM orders x)", vec!["order_total"]),
            ("SELECT * FROM customers c WHERE EXISTS (SELECT c.| FROM orders o)", vec!["customer_name"]),
            ("SELECT * FROM customers c WHERE EXISTS (SELECT o.| FROM orders o", vec!["order_total"]),
            ("SELECT * FROM customers c WHERE EXISTS (SELECT o.| FROM orders o) AND EXISTS (SELECT o.customer_name FROM customers o)", vec!["order_total"]),
        ] {
            let cursor = marked.find('|').unwrap();
            let sql = marked.replace('|', "");
            let mut completion = Completion::default();
            completion.refresh(&sql, cursor, &objects, &[], true, Dialect::Mysql(crate::sql::mysql::Mode::default()));
            assert_eq!(completion.items, expected, "{sql}");
        }
    }

    #[test]
    fn mysql_quoted_aliases_resolve_join_columns() {
        let objects = [object("customers", &["customer_name", "tick`field"]), object("orders", &["order_total"])];
        for mode in [crate::sql::mysql::Mode::default(), crate::sql::mysql::Mode { ansi_quotes: true, no_backslash_escapes: true }] {
            let dialect = Dialect::Mysql(mode);
            let mut aliases = vec!["`customer alias`", "`where`", "`tick``alias`", "`客戶`", "`(`", "`.`"];
            if mode.ansi_quotes { aliases.push("\"customer alias\""); }
            for alias in aliases {
                for (prefix, label, inserted) in [("cu", "customer_name", "`customer_name`"), ("tic", "tick`field", "`tick``field`")] {
                    let head = format!("SELECT {alias}.{prefix}");
                    let tail = format!(" FROM customers AS {alias} JOIN orders AS o ON 1=1");
                    let mut sql = format!("{head}{tail}");
                    let mut completion = Completion::default();
                    completion.refresh(&sql, head.chars().count(), &objects, &[], true, dialect);
                    assert_eq!(completion.items, vec![label], "{sql}");
                    completion.apply(&mut sql, label);
                    assert_eq!(sql, format!("SELECT {alias}.{inserted}{tail}"));
                }
            }
        }
    }

    #[test]
    fn mysql_quoted_keywords_do_not_introduce_relations() {
        let dialect = Dialect::Mysql(crate::sql::mysql::Mode::default());
        assert_eq!(relations_for_dialect("SELECT `FROM`, `JOIN`, `(` FROM `(` AS `where` JOIN orders AS o ON 1=1", dialect), vec![("(".into(), "where".into()), ("orders".into(), "o".into())]);
    }

    #[test]
    fn mysql_schema_table_and_column_candidates_insert_backtick_identifiers() {
        let mut table = object("order details", &["rank", "display name", "tick`field"]);
        table.schema = Some("shop`data".into());
        for mode in [crate::sql::mysql::Mode::default(), crate::sql::mysql::Mode { ansi_quotes: true, no_backslash_escapes: true }] {
            let dialect = Dialect::Mysql(mode);
            for (input, label, expected) in [
                ("SELECT * FROM ord", "order details", "SELECT * FROM `shop``data`.`order details`"),
                ("SELECT * FROM `shop``data`.ord", "order details", "SELECT * FROM `shop``data`.`order details`"),
                ("SELECT ra FROM `order details`", "rank", "SELECT `rank` FROM `order details`"),
                ("SELECT dis FROM `order details`", "display name", "SELECT `display name` FROM `order details`"),
                ("SELECT tic FROM `order details`", "tick`field", "SELECT `tick``field` FROM `order details`"),
                ("SELECT * FROM sho", "shop`data", "SELECT * FROM `shop``data`"),
            ] {
                let cursor = input.find(" FROM `order details`").unwrap_or(input.len());
                let mut completion = Completion::default();
                completion.refresh(input, cursor, &[table.clone()], &[], true, dialect);
                assert!(completion.items.iter().any(|item| item == label), "{input}: {:?}", completion.items);
                let mut sql = input.to_owned();
                completion.apply(&mut sql, label);
                assert_eq!(sql, expected);
            }
        }
    }

    #[test]
    fn popup_labels_share_left_edge() {
        let ctx = egui::Context::default();
        let mut completion = Completion {
            items: vec!["id".into(), "customer_name".into()],
            open: true,
            ..Default::default()
        };
        let mut positions = Vec::new();
        for _ in 0..2 {
            let output = ctx.run(Default::default(), |ctx| {
                completion.menu(ctx, egui::Id::new("alignment"), egui::pos2(20.0, 20.0));
            });
            positions = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if ["id", "customer_name"].contains(&text.galley.text()) =>
                    {
                        Some(text.pos.x)
                    }
                    _ => None,
                })
                .collect();
        }
        assert_eq!(positions.len(), 2);
        assert!((positions[0] - positions[1]).abs() < 1.0);
    }

    #[test]
    fn popup_preserves_full_keyword_and_stays_on_screen() {
        let ctx = egui::Context::default();
        let mut completion = Completion {
            items: vec!["SELECT".into(), "customer_name".into()],
            open: true,
            ..Default::default()
        };
        let mut painted_keyword = false;
        for _ in 0..3 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    completion.menu(ctx, egui::Id::new("full-label"), egui::pos2(395.0, 295.0));
                },
            );
            if let Some(text) = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "SELECT" => Some(text),
                _ => None,
            }) {
                let painted: String = text
                    .galley
                    .rows
                    .iter()
                    .flat_map(|row| row.glyphs.iter().map(|glyph| glyph.chr))
                    .collect();
                assert_eq!(painted, "SELECT", "keyword must not be ellipsized");
                painted_keyword = true;
            }
        }
        assert!(painted_keyword, "keyword rendered");
        let rect = completion.menu_rect.unwrap();
        assert!(rect.right() <= 400.0 && rect.bottom() <= 300.0, "{rect:?}");
    }

    #[test]
    fn popup_hugs_short_suggestions_and_grows_to_fit_long_names() {
        fn size(item: &str) -> egui::Vec2 {
            let ctx = egui::Context::default();
            let mut completion = Completion {
                items: vec![item.into()],
                open: true,
                ..Default::default()
            };
            for _ in 0..2 {
                let _ = ctx.run(Default::default(), |ctx| {
                    completion.menu(ctx, egui::Id::new("geometry"), egui::pos2(20.0, 20.0));
                });
            }
            completion.menu_rect.unwrap().size()
        }
        let short = size("id");
        let long = size("customer_lifetime_value");
        assert!(short.x < 90.0, "{short:?}");
        assert!(short.y <= 28.0, "{short:?}");
        assert!(long.x > short.x && long.x < 250.0, "{long:?}");
        assert_eq!(long.y, short.y);
    }

    #[test]
    fn postgres_catalog_suggestions_match_qualified_relations_and_aliases() {
        let mut table = object("\"public\".\"address\"", &["available", "lastSeen"]);
        table.schema = Some("public".into());
        table.display_name = Some("address".into());
        let objects = [table];
        let mut completion = Completion::default();
        for sql in [
            "SELECT * FROM address a WHERE a.av",
            "SELECT av FROM public.address",
            "SELECT av FROM \"public\".\"address\"",
        ] {
            let cursor = sql.find("av").unwrap() + 2;
            completion.refresh(sql, cursor, &objects, &[], false, Dialect::Postgres);
            assert!(
                completion.items.contains(&"available".into()),
                "{sql}: {:?}",
                completion.items
            );
        }
        for (sql, expected) in [
            ("SELECT * FROM ad", "SELECT * FROM \"public\".\"address\""),
            (
                "SELECT * FROM public.ad",
                "SELECT * FROM public.\"address\"",
            ),
        ] {
            let mut sql = sql.to_owned();
            completion.refresh(&sql, sql.len(), &objects, &[], false, Dialect::Postgres);
            assert!(completion.items.contains(&"address".into()));
            completion.apply(&mut sql, "address");
            assert_eq!(sql, expected);
        }
        let mut sql = "SELECT la FROM address".to_owned();
        completion.refresh(&sql, 9, &objects, &[], false, Dialect::Postgres);
        completion.apply(&mut sql, "lastSeen");
        assert_eq!(sql, "SELECT \"lastSeen\" FROM address");
        let sql = "SELECT av FROM private.address";
        completion.refresh(sql, 9, &objects, &[], false, Dialect::Postgres);
        assert!(!completion.items.contains(&"available".into()));
    }

    #[test]
    fn postgres_completion_uses_postgres_statement_boundaries() {
        let objects = [object("customers", &["name"]), object("orders", &["total"])];
        let sql = "DO $$ BEGIN SELECT 'FROM orders'; END $$; SELECT na FROM customers";
        let cursor = sql.find("na FROM").unwrap() + 2;
        let mut completion = Completion::default();
        completion.refresh(sql, cursor, &objects, &[], false, Dialect::Postgres);
        assert_eq!(completion.items, vec!["name"]);
    }
    #[test]
    fn columns_follow_query_tables_and_aliases() {
        let objects = [
            object("customers", &["id", "name", "email"]),
            object("orders", &["id", "total"]),
        ];
        let mut completion = Completion::default();
        for (sql, prefix, expected) in [
            (
                "SELECT c.na FROM customers AS c JOIN orders o ON o.id = c.id",
                "c.na",
                "name",
            ),
            (
                "SELECT * FROM customers c JOIN orders o ON c.id = o.id WHERE o.to",
                "o.to",
                "total",
            ),
            ("SELECT em FROM customers", "em", "email"),
            ("SELECT * FROM customers c WHERE c.", "c.", "email"),
        ] {
            let cursor = sql[..sql.find(prefix).unwrap() + prefix.len()]
                .chars()
                .count();
            completion.refresh(sql, cursor, &objects, &[], false, Dialect::Sqlite);
            assert!(
                completion.items.contains(&expected.into()),
                "{sql}: {:?}",
                completion.items
            );
        }
        let sql = "SELECT * FROM customers; SELECT to";
        completion.refresh(sql, sql.len(), &objects, &[], false, Dialect::Sqlite);
        assert!(!completion.items.contains(&"total".into()));
        assert_eq!(
            relations("SELECT 'FROM orders' FROM customers -- JOIN orders\n"),
            vec![("customers".into(), "customers".into())]
        );
    }
    #[test]
    fn comma_relations_respect_expression_and_clause_boundaries() {
        let names = |sql| {
            relations(sql)
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            relations(
                "SELECT c.name, o.total FROM main.customers AS c, orders o WHERE c.id = o.id"
            ),
            vec![
                ("main.customers".into(), "c".into()),
                ("orders".into(), "o".into())
            ]
        );
        for sql in [
            "SELECT coalesce(name, orders) FROM customers WHERE id IN (1, orders)",
            "SELECT * FROM customers ORDER BY name, orders",
            "SELECT * FROM customers GROUP BY name, orders HAVING count(*) > 1",
            "SELECT * FROM customers LIMIT 1, orders",
            "SELECT * FROM customers JOIN products ON coalesce(customers.id, orders) = products.id",
        ] {
            assert!(!names(sql).contains(&"orders".into()), "{sql}");
        }
        assert_eq!(
            names("SELECT * FROM customers c JOIN products p USING (id, name), orders o"),
            vec!["customers", "products", "orders"]
        );
        let objects = [object("customers", &["name"]), object("orders", &["total"])];
        let sql = "SELECT o.to FROM customers c, orders o WHERE o.id = c.id";
        let mut completion = Completion::default();
        completion.refresh(
            sql,
            "SELECT o.to".len(),
            &objects,
            &[],
            false,
            Dialect::Sqlite,
        );
        assert_eq!(completion.items, vec!["total"]);
    }

    #[test]
    fn identifier_completions_quote_engine_keywords_and_round_trip() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for name in [
            "order",
            "SELECT",
            "returning",
            "two words",
            "a\"b",
            "123value",
            "日本",
        ] {
            let quoted = format!("\"{}\"", name.replace('"', "\"\""));
            conn.execute_batch(&format!(
                "CREATE TABLE t ({quoted} TEXT); INSERT INTO t VALUES ('ok');"
            ))
            .unwrap();
            let objects = [object("t", &[name])];
            let mut completion = Completion::default();
            let mut sql = "SELECT t. FROM t".to_owned();
            completion.refresh(
                &sql,
                "SELECT t.".len(),
                &objects,
                &[],
                false,
                Dialect::Sqlite,
            );
            assert!(completion.items.contains(&name.to_owned()));
            let caret = completion.apply(&mut sql, name);
            assert_eq!(&sql[char_to_byte(&sql, caret)..], " FROM t");
            assert_eq!(
                conn.query_row(&sql, [], |row| row.get::<_, String>(0))
                    .unwrap(),
                "ok",
                "{sql}"
            );
            conn.execute_batch("DROP TABLE t").unwrap();
        }
        let objects = [object("customers", &["order"])];
        let mut completion = Completion::default();
        let sql = "SELECT order FROM customers";
        completion.refresh(
            sql,
            "SELECT order".len(),
            &objects,
            &[],
            false,
            Dialect::Sqlite,
        );
        assert!(completion.items.contains(&"order".into()));
        let mut sql = "sel".to_owned();
        completion.refresh(&sql, 3, &[], &[], false, Dialect::Sqlite);
        completion.apply(&mut sql, "SELECT");
        assert_eq!(sql, "SELECT");
    }

    #[test]
    fn unicode_replacement_and_comment_suppression() {
        let mut completion = Completion::default();
        let mut sql = "SELECT '日本', se".to_owned();
        completion.refresh(&sql, sql.chars().count(), &[], &[], false, Dialect::Sqlite);
        assert!(completion.items.contains(&"SELECT".into()));
        completion.apply(&mut sql, "SELECT");
        assert_eq!(sql, "SELECT '日本', SELECT");
        for sql in ["-- sel", "SELECT 'sel", "/* sel", "SELECT \"sel"] {
            completion.refresh(sql, sql.chars().count(), &[], &[], true, Dialect::Sqlite);
            assert!(!completion.open);
        }
    }
}
