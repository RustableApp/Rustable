//! Calendar and time picker for date/time cells.
//!
//! Values are read and written in the ISO text forms the drivers already use
//! (`YYYY-MM-DD`, `YYYY-MM-DD HH:MM:SS[.ffffff]`, optional `±HH[:MM]` offset)
//! and are cast by the server on save, exactly like typed edits. Fractional
//! seconds are written only when non-zero, so second-precision columns such as
//! MySQL DATETIME are not rounded by a value the user did not choose.
use crate::db::Value;
use crate::json_editor::Target;
use chrono::{Datelike, NaiveDate, Timelike};
use eframe::egui::{self, RichText};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Date,
    DateTime,
    /// PostgreSQL `timestamp with time zone`: an instant with an explicit offset.
    DateTimeTz,
    Time,
}

impl Kind {
    pub(crate) fn has_date(self) -> bool {
        self != Kind::Time
    }
    pub(crate) fn has_time(self) -> bool {
        self != Kind::Date
    }
}

/// Column types the picker understands. `time with time zone` and MySQL TIME
/// durations (negative or over 24 hours) are left to the text editor.
pub(crate) fn kind(data_type: &str) -> Option<Kind> {
    let lower = data_type.trim().to_ascii_lowercase();
    // Drop precision modifiers: timestamp(3) without time zone, datetime(6).
    let mut base = String::with_capacity(lower.len());
    let mut depth = 0;
    for c in lower.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ if depth == 0 => base.push(c),
            _ => {}
        }
    }
    let base = base.split_whitespace().collect::<Vec<_>>().join(" ");
    match base.as_str() {
        "date" => Some(Kind::Date),
        "timestamptz" | "timestamp with time zone" => Some(Kind::DateTimeTz),
        "timestamp" | "timestamp without time zone" | "datetime" => Some(Kind::DateTime),
        "time" | "time without time zone" => Some(Kind::Time),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Moment {
    pub date: NaiveDate,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub micros: u32,
    /// Offset east of UTC in minutes, for `DateTimeTz` only.
    pub offset: Option<i32>,
}

fn digits(text: &str, count: usize) -> Option<(u32, &str)> {
    let head = text.get(..count)?;
    head.bytes().all(|b| b.is_ascii_digit()).then(|| (head.parse().ok().unwrap(), &text[count..]))
}

fn parse_time(text: &str) -> Option<(u32, u32, u32, u32, &str)> {
    let (hour, rest) = digits(text, 2)?;
    let rest = rest.strip_prefix(':')?;
    let (minute, mut rest) = digits(rest, 2)?;
    let mut second = 0;
    let mut micros = 0;
    if let Some(tail) = rest.strip_prefix(':') {
        let (s, tail) = digits(tail, 2)?;
        second = s;
        rest = tail;
        if let Some(tail) = rest.strip_prefix('.') {
            let count = tail.bytes().take_while(u8::is_ascii_digit).count();
            if count == 0 || count > 6 {
                return None;
            }
            let fraction = &tail[..count];
            micros = format!("{fraction:0<6}").parse().ok()?;
            rest = &tail[count..];
        }
    }
    (hour < 24 && minute < 60 && second < 60).then_some((hour, minute, second, micros, rest))
}

fn parse_offset(text: &str) -> Option<i32> {
    if text == "Z" || text == "z" {
        return Some(0);
    }
    let sign = match text.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (hours, rest) = digits(&text[1..], 2)?;
    let minutes = match rest {
        "" => 0,
        _ => digits(rest.strip_prefix(':').unwrap_or(rest), 2).filter(|(_, tail)| tail.is_empty())?.0,
    };
    (hours <= 15 && minutes < 60).then(|| sign * (hours as i32 * 60 + minutes as i32))
}

pub(crate) fn parse(text: &str, kind: Kind) -> Option<Moment> {
    let text = text.trim();
    let midnight = NaiveDate::from_ymd_opt(2000, 1, 1)?;
    if kind == Kind::Time {
        let (hour, minute, second, micros, rest) = parse_time(text)?;
        return rest.is_empty().then_some(Moment { date: midnight, hour, minute, second, micros, offset: None });
    }
    let (year, rest) = digits(text, 4)?;
    let (month, rest) = digits(rest.strip_prefix('-')?, 2)?;
    let (day, rest) = digits(rest.strip_prefix('-')?, 2)?;
    let date = NaiveDate::from_ymd_opt(i32::try_from(year).ok().filter(|y| *y >= 1)?, month, day)?;
    let mut moment = Moment { date, hour: 0, minute: 0, second: 0, micros: 0, offset: None };
    if kind == Kind::Date {
        return rest.is_empty().then_some(moment);
    }
    let rest = match rest.strip_prefix(' ').or_else(|| rest.strip_prefix('T')) {
        Some(time) => {
            let (hour, minute, second, micros, rest) = parse_time(time)?;
            (moment.hour, moment.minute, moment.second, moment.micros) = (hour, minute, second, micros);
            rest
        }
        None if rest.is_empty() => rest,
        None => return None,
    };
    match kind {
        Kind::DateTimeTz => {
            moment.offset = Some(if rest.is_empty() { local_offset() } else { parse_offset(rest)? });
            Some(moment)
        }
        _ => rest.is_empty().then_some(moment),
    }
}

fn offset_text(minutes: i32) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let (hours, minutes) = (minutes.abs() / 60, minutes.abs() % 60);
    if minutes == 0 { format!("{sign}{hours:02}") } else { format!("{sign}{hours:02}:{minutes:02}") }
}

pub(crate) fn format(moment: &Moment, kind: Kind) -> String {
    let date = format!("{:04}-{:02}-{:02}", moment.date.year(), moment.date.month(), moment.date.day());
    let mut time = format!("{:02}:{:02}:{:02}", moment.hour, moment.minute, moment.second);
    if moment.micros != 0 {
        time.push_str(format!(".{:06}", moment.micros).trim_end_matches('0'));
    }
    match kind {
        Kind::Date => date,
        Kind::Time => time,
        Kind::DateTime => format!("{date} {time}"),
        Kind::DateTimeTz => format!("{date} {time}{}", offset_text(moment.offset.unwrap_or(0))),
    }
}

fn local_offset() -> i32 {
    chrono::Local::now().offset().local_minus_utc() / 60
}

/// Current date/time text for a column kind, as the "Set Value → Now" helper writes it.
pub(crate) fn now_text(kind: Kind) -> String {
    format(&now(kind, None), kind)
}

/// The current time; zones are kept at whole seconds like DataGrip's "Now".
fn now(kind: Kind, offset: Option<i32>) -> Moment {
    let offset = offset.unwrap_or_else(local_offset);
    let current = match kind {
        Kind::DateTimeTz => chrono::Utc::now().naive_utc() + chrono::Duration::minutes(offset.into()),
        _ => chrono::Local::now().naive_local(),
    };
    Moment {
        date: current.date(),
        hour: if kind == Kind::Date { 0 } else { current.hour() },
        minute: if kind == Kind::Date { 0 } else { current.minute() },
        second: if kind == Kind::Date { 0 } else { current.second() },
        micros: 0,
        offset: (kind == Kind::DateTimeTz).then_some(offset),
    }
}

pub(crate) struct DatePicker {
    pub target: Target,
    pub kind: Kind,
    pub text: String,
    pub moment: Moment,
    pub nullable: bool,
    pub null: bool,
    /// Month shown in the calendar, which can differ from the selected date.
    view: (i32, u32),
    dirty: bool,
}

pub(crate) enum Outcome {
    Open,
    Close,
    Apply(Value),
}

impl DatePicker {
    pub(crate) fn new(target: Target, current: &Value, kind: Kind, nullable: bool) -> Self {
        let text = match current {
            Value::Text(text) => text.clone(),
            _ => String::new(),
        };
        let parsed = parse(&text, kind);
        let moment = parsed.unwrap_or_else(|| now(kind, None));
        Self {
            target,
            kind,
            // Unrecognized values (e.g. 'infinity') stay visible until replaced.
            text: if parsed.is_some() || !text.is_empty() { text } else { format(&moment, kind) },
            moment,
            nullable,
            null: matches!(current, Value::Null),
            view: (moment.date.year(), moment.date.month()),
            dirty: false,
        }
    }

    fn set(&mut self, moment: Moment) {
        self.moment = moment;
        self.text = format(&moment, self.kind);
        self.view = (moment.date.year(), moment.date.month());
        self.null = false;
        self.dirty = true;
    }

    pub(crate) fn staged_value(&self) -> Result<Value, String> {
        if self.null {
            return Ok(Value::Null);
        }
        let text = self.text.trim();
        if text.is_empty() {
            return Err("Enter a value or choose NULL".into());
        }
        // Normalize recognized input; other literals are validated by the server.
        Ok(Value::Text(parse(text, self.kind).map_or_else(|| text.to_owned(), |m| format(&m, self.kind))))
    }

    fn calendar(&mut self, ui: &mut egui::Ui) {
        let (year, month) = self.view;
        ui.horizontal(|ui| {
            if crate::icons::button(ui, "chevron-left", egui::vec2(24.0, 24.0), false).on_hover_text("Previous month").clicked() {
                self.view = if month == 1 { (year - 1, 12) } else { (year, month - 1) };
            }
            let title = NaiveDate::from_ymd_opt(year, month, 1).map_or_else(String::new, |d| d.format("%B %Y").to_string());
            ui.add_sized([150.0, 24.0], egui::Label::new(RichText::new(title).strong()));
            if crate::icons::button(ui, "chevron-right", egui::vec2(24.0, 24.0), false).on_hover_text("Next month").clicked() {
                self.view = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
            }
            let mut shown_year = self.view.0;
            if ui.add(egui::DragValue::new(&mut shown_year).range(1..=9999)).on_hover_text("Year").changed() {
                self.view.0 = shown_year;
            }
        });
        let Some(first) = NaiveDate::from_ymd_opt(self.view.0, self.view.1, 1) else { return };
        let today = chrono::Local::now().date_naive();
        let lead = first.weekday().num_days_from_monday();
        let start = first - chrono::Duration::days(lead.into());
        let cell = egui::vec2(34.0, 26.0);
        egui::Grid::new("date-picker-days").spacing(egui::vec2(2.0, 2.0)).show(ui, |ui| {
            for name in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] {
                ui.add_sized(cell, egui::Label::new(RichText::new(name).small().weak()));
            }
            ui.end_row();
            for week in 0..6 {
                for weekday in 0..7 {
                    let day = start + chrono::Duration::days(week * 7 + weekday);
                    let in_month = day.month() == self.view.1;
                    let selected = !self.null && day == self.moment.date;
                    let mut text = RichText::new(day.day().to_string());
                    if !in_month {
                        text = text.weak();
                    }
                    if day == today {
                        text = text.underline();
                    }
                    let response = ui.add_sized(cell, egui::Button::selectable(selected, text));
                    if response.clicked() {
                        let mut moment = self.moment;
                        moment.date = day;
                        self.set(moment);
                    }
                }
                ui.end_row();
            }
        });
    }

    fn time_fields(&mut self, ui: &mut egui::Ui) {
        let mut moment = self.moment;
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Time");
            changed |= ui.add(egui::DragValue::new(&mut moment.hour).range(0..=23).custom_formatter(|v, _| format!("{v:02}"))).changed();
            ui.label(":");
            changed |= ui.add(egui::DragValue::new(&mut moment.minute).range(0..=59).custom_formatter(|v, _| format!("{v:02}"))).changed();
            ui.label(":");
            changed |= ui.add(egui::DragValue::new(&mut moment.second).range(0..=59).custom_formatter(|v, _| format!("{v:02}"))).changed();
            ui.label(".");
            changed |= ui.add(egui::DragValue::new(&mut moment.micros).range(0..=999_999).custom_formatter(|v, _| format!("{v:06}")))
                .on_hover_text("Microseconds; written only when non-zero").changed();
            if self.kind == Kind::DateTimeTz {
                let mut offset = moment.offset.unwrap_or(0);
                ui.label("UTC");
                if ui.add(egui::DragValue::new(&mut offset).range(-15 * 60..=15 * 60).speed(15.0)
                    .custom_formatter(|v, _| offset_text(v as i32))
                    .custom_parser(|text| parse_offset(text.trim()).map(f64::from)))
                    .on_hover_text("Offset from UTC. PostgreSQL stores the instant; the offset only affects how it is entered.")
                    .changed()
                {
                    moment.offset = Some(offset);
                    changed = true;
                }
            }
        });
        if changed {
            self.set(moment);
        }
    }

    pub(crate) fn show(&mut self, ctx: &egui::Context) -> Outcome {
        let mut outcome = Outcome::Open;
        let modal = egui::Modal::new(egui::Id::new("date-picker")).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.horizontal(|ui| {
                ui.strong(match self.kind {
                    Kind::Date => "Choose date",
                    Kind::Time => "Choose time",
                    _ => "Choose date and time",
                });
                ui.label(RichText::new(&self.target.column).monospace().weak());
            });
            ui.add_space(6.0);
            ui.add_enabled_ui(!self.null, |ui| {
                if self.kind.has_date() {
                    self.calendar(ui);
                    ui.add_space(4.0);
                }
                if self.kind.has_time() {
                    self.time_fields(ui);
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let label = match self.kind { Kind::Date => "Today", _ => "Now" };
                if ui.button(label).clicked() {
                    self.set(now(self.kind, self.moment.offset));
                }
                if self.nullable {
                    let mut null = self.null;
                    if ui.checkbox(&mut null, "NULL").changed() {
                        self.null = null;
                        self.dirty = true;
                    }
                }
            });
            ui.add_space(4.0);
            let response = ui.add_enabled(!self.null, egui::TextEdit::singleline(&mut self.text)
                .id(egui::Id::new("date-picker-text"))
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY));
            if response.changed() {
                self.dirty = true;
                if let Some(moment) = parse(&self.text, self.kind) {
                    self.moment = moment;
                    self.view = (moment.date.year(), moment.date.month());
                }
            }
            if !self.null && parse(&self.text, self.kind).is_none() {
                ui.label(RichText::new("Not a recognized value; the database will validate it on save.").small().color(ui.visuals().warn_fg_color));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let staged = self.staged_value();
                let apply = ui.add_enabled(staged.is_ok(), egui::Button::new("Apply · Enter"))
                    .on_hover_text("Stage this value. Changes are reviewed as SQL before saving.");
                let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
                if (apply.clicked() || enter) && let Ok(value) = staged {
                    outcome = if self.dirty { Outcome::Apply(value) } else { Outcome::Close };
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Close;
                }
            });
        });
        if modal.should_close() && matches!(outcome, Outcome::Open) {
            outcome = Outcome::Close;
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_types_map_to_picker_kinds() {
        for (data_type, expected) in [
            ("date", Some(Kind::Date)),
            ("DATE", Some(Kind::Date)),
            ("timestamp without time zone", Some(Kind::DateTime)),
            ("timestamp(3) without time zone", Some(Kind::DateTime)),
            ("timestamp with time zone", Some(Kind::DateTimeTz)),
            ("timestamptz", Some(Kind::DateTimeTz)),
            ("datetime(6)", Some(Kind::DateTime)),
            ("timestamp", Some(Kind::DateTime)),
            ("time without time zone", Some(Kind::Time)),
            ("time with time zone", None),
            ("text", None),
            ("date[]", None),
            ("interval", None),
        ] {
            assert_eq!(kind(data_type), expected, "{data_type}");
        }
    }

    #[test]
    fn driver_text_round_trips_exactly() {
        for (text, kind) in [
            ("2024-02-29", Kind::Date),
            ("2024-02-29 12:34:56", Kind::DateTime),
            ("2024-02-29 12:34:56.123456", Kind::DateTime),
            ("2024-02-29 12:34:56.5", Kind::DateTime),
            ("2024-02-29 12:34:56.123456+00", Kind::DateTimeTz),
            ("2024-02-29 12:34:56+05:30", Kind::DateTimeTz),
            ("2024-02-29 12:34:56-08", Kind::DateTimeTz),
            ("23:59:59.000001", Kind::Time),
        ] {
            let moment = parse(text, kind).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(format(&moment, kind), text);
        }
        // Accepted input forms are normalized.
        let moment = parse("2024-02-29T08:00Z", Kind::DateTimeTz).unwrap();
        assert_eq!(format(&moment, Kind::DateTimeTz), "2024-02-29 08:00:00+00");
        assert_eq!(format(&parse("2024-02-29", Kind::DateTime).unwrap(), Kind::DateTime), "2024-02-29 00:00:00");
    }

    #[test]
    fn invalid_and_special_values_are_not_misread() {
        for (text, kind) in [
            ("2023-02-29", Kind::Date),
            ("0000-00-00", Kind::Date),
            ("infinity", Kind::DateTime),
            ("2024-02-29 24:00:00", Kind::DateTime),
            ("2024-02-29 12:34:56.1234567", Kind::DateTime),
            ("2024-02-29 12:34:56+05", Kind::DateTime),
            ("-838:59:59", Kind::Time),
            ("2024-2-9", Kind::Date),
        ] {
            assert_eq!(parse(text, kind), None, "{text}");
        }
    }

    #[test]
    fn now_text_is_parseable_whole_seconds_for_every_kind() {
        for kind in [Kind::Date, Kind::DateTime, Kind::DateTimeTz, Kind::Time] {
            let text = now_text(kind);
            let moment = parse(&text, kind).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(moment.micros, 0, "{text}");
            assert_eq!(format(&moment, kind), text);
        }
        assert_eq!(now_text(Kind::Date).len(), 10);
        assert_eq!(now_text(Kind::DateTime).len(), 19);
    }

    fn target() -> Target {
        Target { table: "t".into(), row: vec![Value::Integer(1)], column: "at".into(), original: Value::Null }
    }

    #[test]
    fn picker_keeps_unknown_text_and_normalizes_recognized_input() {
        let picker = DatePicker::new(target(), &Value::Text("infinity".into()), Kind::DateTime, false);
        assert_eq!(picker.text, "infinity");
        assert_eq!(picker.staged_value().unwrap(), Value::Text("infinity".into()));
        let mut picker = DatePicker::new(target(), &Value::Text("2024-02-29 12:00:00".into()), Kind::DateTime, true);
        let mut moment = picker.moment;
        moment.date = NaiveDate::from_ymd_opt(2024, 3, 1).unwrap();
        picker.set(moment);
        assert_eq!(picker.staged_value().unwrap(), Value::Text("2024-03-01 12:00:00".into()));
        picker.text = "2024-03-02T09:30".into();
        assert_eq!(picker.staged_value().unwrap(), Value::Text("2024-03-02 09:30:00".into()));
        picker.null = true;
        assert_eq!(picker.staged_value().unwrap(), Value::Null);
        let empty = DatePicker::new(target(), &Value::Null, Kind::Date, true);
        assert!(empty.null && parse(&empty.text, Kind::Date).is_some(), "NULL opens on today");
    }
}
