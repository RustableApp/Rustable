//! Bounded similarity request, validated again on the database worker.
#[derive(Clone, Debug)]
pub(crate) enum Input {
    Element(Vec<u8>),
    Values(Vec<f32>),
}
#[derive(Clone, Debug)]
pub(crate) struct Search {
    pub key: Vec<u8>,
    pub input: Input,
    pub filter: String,
    pub count: usize,
    pub effort: usize,
    pub filter_effort: usize,
}
impl Search {
    pub fn element(key: Vec<u8>, element: Vec<u8>) -> Self {
        Self {
            key,
            input: Input::Element(element),
            filter: String::new(),
            count: 20,
            effort: 100,
            filter_effort: 1000,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.key.len() > 4096 || self.filter.len() > 4096 {
            return Err("Key and filter must each fit within 4 KiB.".into());
        }
        if !(1..=100).contains(&self.count)
            || !(1..=1000).contains(&self.effort)
            || !(1..=10000).contains(&self.filter_effort)
        {
            return Err(
                "Use 1–100 results, 1–1,000 search effort and 1–10,000 filter attempts.".into(),
            );
        }
        match &self.input {
            Input::Element(value) if value.len() <= 65536 => Ok(()),
            Input::Values(values) if !values.is_empty() && values.len() <= 4096
                && values.iter().all(|v| v.is_finite())
                && values.iter().any(|v| *v != 0.0) => Ok(()),
            _ => Err("Use an element name up to 64 KiB or 1–4,096 finite vector components with at least one nonzero value.".into()),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn command(&self) -> Result<redis::Cmd, String> {
        self.validate()?;
        let mut command = redis::cmd("VSIM");
        command.arg(&self.key);
        match &self.input {
            Input::Element(element) => {
                command.arg("ELE").arg(element);
            }
            Input::Values(values) => {
                command.arg("VALUES").arg(values.len()).arg(values);
            }
        }
        command
            .arg("WITHSCORES")
            .arg("COUNT")
            .arg(self.count)
            .arg("EF")
            .arg(self.effort);
        if !self.filter.trim().is_empty() {
            command
                .arg("FILTER")
                .arg(&self.filter)
                .arg("FILTER-EF")
                .arg(self.filter_effort);
        }
        // Direct VSIM retains Redis background execution; never wrap in Lua or NOTHREAD.
        Ok(command)
    }
}
/// Parse only on explicit submission, never on a rendering/typing hot path.
pub(crate) fn parse_values(text: &str) -> Result<Vec<f32>, String> {
    if text.len() > 65536 {
        return Err("Vector input exceeds 64 KiB.".into());
    }
    let text = text.trim();
    let text = if text.starts_with('[') && text.ends_with(']') {
        &text[1..text.len() - 1]
    } else {
        text
    };
    let mut values = Vec::new();
    for token in text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|v| !v.is_empty())
    {
        if values.len() == 4096 {
            return Err("Vector exceeds 4,096 components.".into());
        }
        let value = token
            .parse::<f32>()
            .map_err(|_| "Enter numeric components separated by commas or spaces.".to_string())?;
        if !value.is_finite() {
            return Err("Vector components must be finite 32-bit numbers.".into());
        }
        values.push(value);
    }
    if values.is_empty() || values.iter().all(|v| *v == 0.0) {
        return Err("Enter a nonzero vector.".into());
    }
    Ok(values)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vector_input_bounds_and_finite_nonzero_components() {
        assert_eq!(parse_values("[1, -0.5, 2e-3]").unwrap(), [1.0, -0.5, 0.002]);
        for text in ["", "[]", "0, -0", "NaN", "inf", "1e99", "[1, nope]", "[1,2"] {
            assert!(parse_values(text).is_err(), "{text}");
        }
        assert!(parse_values(&"1 ".repeat(4097)).is_err());
        assert!(parse_values(&" ".repeat(65537)).is_err());
    }
    #[test]
    fn worker_revalidates_settings_and_binary_element() {
        let mut search = Search::element(b"key".to_vec(), vec![0, 255]);
        assert!(search.validate().is_ok());
        for count in [0, 101, usize::MAX] {
            search.count = count;
            assert!(search.validate().is_err());
        }
        search.count = 100;
        search.input = Input::Values(vec![1.0, f32::NAN]);
        assert!(search.validate().is_err());
        search.input = Input::Values(vec![1.0; 4097]);
        assert!(search.validate().is_err());
        search.input = Input::Values(vec![1.0]);
        search.filter = "x".repeat(4097);
        assert!(search.validate().is_err());
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn command_preserves_filter_as_one_argument_and_background_execution() {
        let mut search = Search::element(b"key".to_vec(), vec![0, 255]);
        search.input = Input::Values(vec![1.0, 0.5]);
        search.filter = ".category == \"camera\"".into();
        search.count = 7;
        let actual = search.command().unwrap().get_packed_command();
        let expected = redis::cmd("VSIM")
            .arg("key")
            .arg("VALUES")
            .arg(2)
            .arg(1.0f32)
            .arg(0.5f32)
            .arg("WITHSCORES")
            .arg("COUNT")
            .arg(7)
            .arg("EF")
            .arg(100)
            .arg("FILTER")
            .arg(&search.filter)
            .arg("FILTER-EF")
            .arg(1000)
            .get_packed_command();
        assert_eq!(actual, expected);
    }
}

pub(crate) struct Draft {
    search: Search,
    supplied: bool,
    values: String,
    error: Option<String>,
    #[cfg(test)]
    benchmark_focus: bool,
}
impl Draft {
    pub fn new(key: Vec<u8>, element: Vec<u8>) -> Self {
        Self {
            search: Search::element(key, element),
            supplied: false,
            values: String::new(),
            error: None,
            #[cfg(test)]
            benchmark_focus: false,
        }
    }
    #[cfg(all(feature = "qa", not(target_arch = "wasm32")))]
    pub fn preview_supplied(&mut self) -> Search {
        self.supplied = true;
        self.values = "[1, 0.25, 0.1]".into();
        self.search.filter = ".category == \"Electronics\"".into();
        self.search.count = 10;
        let mut search = self.search.clone();
        search.input = Input::Values(parse_values(&self.values).unwrap());
        search
    }
    pub fn matches(&self, key: &[u8], element: &[u8]) -> bool {
        self.search.key == key
            && matches!(&self.search.input, Input::Element(value) if value == element)
    }
    pub fn show(&mut self, ui: &mut eframe::egui::Ui, enabled: bool) -> Option<Search> {
        use eframe::egui;
        let mut submit = false;
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.supplied, false, "Selected element");
                ui.selectable_value(&mut self.supplied, true, "Supplied vector");
            });
            if self.supplied {
                let response = ui.add(
                    egui::TextEdit::multiline(&mut self.values)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .char_limit(65536)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("1, 0.5, -0.2 …"),
                );
                #[cfg(test)]
                if self.benchmark_focus {
                    response.request_focus();
                    self.benchmark_focus = false;
                }
                #[cfg(not(test))]
                let _ = response;
                ui.weak("Up to 4,096 components · match the stored vector dimensions");
            }
            ui.label("Attribute filter");
            ui.add(
                egui::TextEdit::singleline(&mut self.search.filter)
                    .desired_width(f32::INFINITY)
                    .char_limit(4096)
                    .hint_text("Optional · .category == \"camera\""),
            );
            ui.horizontal_wrapped(|ui| {
                ui.label("Results");
                ui.add(egui::DragValue::new(&mut self.search.count).range(1..=100));
                ui.label("Search effort");
                ui.add(egui::DragValue::new(&mut self.search.effort).range(1..=1000));
                ui.label("Filter attempts");
                ui.add_enabled(
                    !self.search.filter.trim().is_empty(),
                    egui::DragValue::new(&mut self.search.filter_effort).range(1..=10000),
                );
            });
            ui.horizontal(|ui| {
                ui.weak("Higher effort can improve recall at a cost in server work.");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    submit = ui
                        .add(crate::redis_workspace::action_button("search", "Search"))
                        .clicked();
                });
            });
        });
        let result = if submit {
            let mut search = self.search.clone();
            let parsed = if self.supplied {
                parse_values(&self.values).map(|values| search.input = Input::Values(values))
            } else {
                Ok(())
            };
            match parsed.and_then(|()| search.validate()) {
                Ok(()) => {
                    self.error = None;
                    Some(search)
                }
                Err(error) => {
                    self.error = Some(error);
                    None
                }
            }
        } else {
            None
        };
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.separator();
        result
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use eframe::egui;
    fn frame(
        ctx: &egui::Context,
        draft: &mut Draft,
        enabled: bool,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<Search>) {
        let mut action = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 760.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = draft.show(ui, enabled);
                });
            },
        );
        (output, action)
    }
    fn click(ctx: &egui::Context, draft: &mut Draft, enabled: bool, label: &str) -> Option<Search> {
        fn find(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
                _ => None,
            }
        }
        let (output, _) = frame(ctx, draft, enabled, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, label))
            .expect("visible control");
        let mut result = None;
        for pressed in [true, false] {
            let (_, action) = frame(
                ctx,
                draft,
                enabled,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            if action.is_some() {
                result = action;
            }
        }
        result
    }
    #[test]
    fn vector_form_submits_explicitly_and_retains_draft_after_validation_and_server_error() {
        let ctx = egui::Context::default();
        let mut draft = Draft::new(b"key".to_vec(), vec![0, 255]);
        assert!(click(&ctx, &mut draft, true, "Supplied vector").is_none());
        assert!(draft.supplied);
        draft.values = "NaN".into();
        draft.search.filter = ".price < 100".into();
        assert!(click(&ctx, &mut draft, true, "Search").is_none());
        assert!(draft.error.is_some());
        assert_eq!(draft.values, "NaN");
        draft.values = "[1,0.5]".into();
        assert!(click(&ctx, &mut draft, false, "Search").is_none());
        let request = click(&ctx, &mut draft, true, "Search").unwrap();
        assert!(matches!(request.input,Input::Values(v) if v==[1.0,0.5]));
        assert_eq!(request.filter, ".price < 100");
        // Server errors are displayed by Workspace; neither response nor rendering consumes the draft.
        for _ in 0..3 {
            assert!(frame(&ctx, &mut draft, true, vec![]).1.is_none());
        }
        assert_eq!(draft.values, "[1,0.5]");
        assert_eq!(draft.search.filter, ".price < 100");
        assert!(click(&ctx, &mut draft, true, "Selected element").is_none());
        let request = click(&ctx, &mut draft, true, "Search").unwrap();
        assert!(matches!(request.input,Input::Element(v) if v==[0,255]));
    }
}

#[cfg(test)]
pub(crate) const BENCHMARK_HAS_CONTROLS: bool = true;
#[cfg(test)]
impl Draft {
    pub(crate) fn prepare_benchmark(&mut self, supplied: bool) {
        self.supplied = supplied;
        self.benchmark_focus = supplied;
        self.values = std::iter::repeat_n("0.25", 4096)
            .collect::<Vec<_>>()
            .join(", ");
    }
}
