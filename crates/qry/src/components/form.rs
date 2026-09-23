//! Modal forms: a column of labelled fields drawn over the panes.
//!
//! [`Form`] holds everything the New Connection and Export modals share —
//! key handling, layout, scrolling and the popup frame. An implementor only
//! supplies its fields and what submitting them means.

use crossterm::event::KeyEvent;
use ratatui::{
    layout::Flex,
    prelude::*,
    widgets::{Block, Clear},
};

use crate::action::Action;
use crate::components::textinput::{TextInput, typed_char};
use crate::keymap::FormCommand;

/// Each field is a bordered box, with a blank row between boxes.
const FIELD_HEIGHT: u16 = 3;
const FIELD_GAP: u16 = 1;
const LABEL_WIDTH: u16 = 12;
pub const MODAL_WIDTH: u16 = 60;

pub enum Field {
    Text(TextInput),
    /// Cycled with ← / → (or Space) instead of typed.
    Choice {
        options: &'static [&'static str],
        selected: usize,
    },
}

impl Field {
    pub fn text() -> Self {
        Field::Text(TextInput::default())
    }

    pub fn masked() -> Self {
        Field::Text(TextInput { masked: true, ..TextInput::default() })
    }

    pub fn choice(options: &'static [&'static str], selected: usize) -> Self {
        Field::Choice { options, selected }
    }

    fn apply(&mut self, command: FormCommand) {
        match self {
            Field::Text(input) => match command {
                FormCommand::Left => input.move_left(),
                FormCommand::Right => input.move_right(),
                FormCommand::LineStart => input.move_home(),
                FormCommand::LineEnd => input.move_end(),
                FormCommand::DeleteBack => input.delete_back(),
                FormCommand::DeleteForward => input.delete_forward(),
                _ => {}
            },
            Field::Choice { options, selected } => match command {
                FormCommand::Right | FormCommand::NextChoice => {
                    *selected = (*selected + 1) % options.len();
                }
                FormCommand::Left => *selected = (*selected + options.len() - 1) % options.len(),
                _ => {}
            },
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        match self {
            Field::Text(input) => input.render(frame, area, focused),
            Field::Choice { options, selected } => {
                let option = options[*selected];
                let line = if focused {
                    Line::from(format!("‹ {option} ›")).cyan()
                } else {
                    Line::from(format!("  {option}"))
                };
                frame.render_widget(line, area);
            }
        }
    }
}

pub trait Form {
    fn title(&self) -> &'static str;
    /// Shown in the popup's bottom border while there is no error.
    fn hint(&self) -> &'static str;
    /// One label per field, in the same order as [`Form::fields`].
    fn labels(&self) -> &'static [&'static str];
    fn fields(&self) -> &[Field];
    fn fields_mut(&mut self) -> &mut [Field];
    fn focus(&self) -> usize;
    fn set_focus(&mut self, focus: usize);
    fn error(&self) -> Option<&str>;
    fn set_error(&mut self, error: Option<String>);
    /// What to send when the form is accepted, or why it was refused.
    fn submit(&self) -> Result<Action, String>;

    /// Called after a key changed the form, so a form whose fields depend on
    /// each other can add or remove one.
    fn after_change(&mut self) {}

    fn text(&self, field: usize) -> &str {
        match &self.fields()[field] {
            Field::Text(input) => &input.value,
            Field::Choice { .. } => "",
        }
    }

    fn selected(&self, field: usize) -> usize {
        match &self.fields()[field] {
            Field::Choice { selected, .. } => *selected,
            Field::Text(_) => 0,
        }
    }

    /// `command` is what the config binds `key` to. In a text field, plain
    /// characters are typed even if bound, so Space types a space there.
    fn handle_key(&mut self, key: KeyEvent, command: Option<FormCommand>) {
        let focus = self.focus();
        let count = self.labels().len();

        if let Some(c) = typed_char(key)
            && matches!(self.fields()[focus], Field::Text(_))
        {
            self.set_error(None);
            if let Field::Text(input) = &mut self.fields_mut()[focus] {
                input.insert(c);
            }
            self.after_change();
            return;
        }

        match command {
            Some(FormCommand::NextField) => self.set_focus((focus + 1) % count),
            Some(FormCommand::PrevField) => self.set_focus((focus + count - 1) % count),
            // Submitting is the owner's job: it has the channel to send on.
            Some(FormCommand::Submit) => {}
            Some(command) => {
                self.set_error(None);
                self.fields_mut()[focus].apply(command);
                self.after_change();
            }
            None => {}
        }
    }

    /// Height of the popup with every field shown.
    fn height(&self) -> u16 {
        let fields = self.labels().len() as u16;
        fields * (FIELD_HEIGHT + FIELD_GAP) - FIELD_GAP + 4
    }

    /// Draws the popup centred on `area`, shrinking to fit a short terminal.
    fn render(&self, frame: &mut Frame, area: Rect) {
        let height = self.height().min(area.height);
        let [popup] = Layout::horizontal([Constraint::Length(MODAL_WIDTH)])
            .flex(Flex::Center)
            .areas(area);
        let [popup] = Layout::vertical([Constraint::Length(height)])
            .flex(Flex::Center)
            .areas(popup);

        frame.render_widget(Clear, popup);
        let footer = match self.error() {
            Some(error) => Line::from(format!(" {error} ")).red(),
            None => Line::from(format!(" {} ", self.hint())).dark_gray(),
        };
        let block = Block::bordered().title(self.title()).title_bottom(footer);
        let inner = block.inner(popup).inner(Margin::new(2, 1));
        frame.render_widget(block, popup);
        self.render_fields(frame, inner);
    }

    /// Draws as many fields as fit, scrolled so the focused one is visible.
    fn render_fields(&self, frame: &mut Frame, area: Rect) {
        let labels = self.labels();
        let fits = usize::from((area.height + FIELD_GAP) / (FIELD_HEIGHT + FIELD_GAP))
            .clamp(1, labels.len());
        let first = (self.focus() + 1).saturating_sub(fits);
        let rows = Layout::vertical(vec![Constraint::Length(FIELD_HEIGHT); fits])
            .spacing(FIELD_GAP)
            .split(area);

        for (row, i) in rows.iter().zip(first..) {
            if row.height < FIELD_HEIGHT {
                continue;
            }
            let focused = i == self.focus();
            let [label_area, box_area] =
                Layout::horizontal([Constraint::Length(LABEL_WIDTH), Constraint::Fill(1)])
                    .areas(*row);

            // The label sits on the box's middle row, level with the text.
            let label_area = Rect { y: label_area.y + 1, height: 1, ..label_area };
            let label = if focused {
                Line::from(format!("› {}", labels[i])).cyan().bold()
            } else {
                Line::from(format!("  {}", labels[i]))
            };
            frame.render_widget(label, label_area);

            let block = Block::bordered().border_style(if focused {
                Style::new().cyan()
            } else {
                Style::new().dark_gray()
            });
            let input_area = block.inner(box_area).inner(Margin::new(1, 0));
            frame.render_widget(block, box_area);

            self.fields()[i].render(frame, input_area, focused);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::Config;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    pub fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Sends `key` to `form` the way `Home` does: with its configured command.
    pub fn press(form: &mut impl Form, key: KeyEvent) {
        form.handle_key(key, Config::embedded().panes.form.get(key));
    }

    pub fn type_str(form: &mut impl Form, text: &str) {
        text.chars().for_each(|c| press(form, key(KeyCode::Char(c))));
    }

    pub fn render(form: &impl Form, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| form.render(frame, frame.area())).unwrap();
        terminal.backend().buffer().clone()
    }

    pub fn lines(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    /// A two-field form, so the trait can be tested without either real one.
    struct TestForm {
        fields: [Field; 2],
        focus: usize,
        error: Option<String>,
    }

    impl Default for TestForm {
        fn default() -> Self {
            Self {
                fields: [Field::text(), Field::choice(&["red", "green"], 0)],
                focus: 0,
                error: None,
            }
        }
    }

    impl Form for TestForm {
        fn title(&self) -> &'static str { "Test" }
        fn hint(&self) -> &'static str { "Enter ok" }
        fn labels(&self) -> &'static [&'static str] { &["Name", "Colour"] }
        fn fields(&self) -> &[Field] { &self.fields }
        fn fields_mut(&mut self) -> &mut [Field] { &mut self.fields }
        fn focus(&self) -> usize { self.focus }
        fn set_focus(&mut self, focus: usize) { self.focus = focus }
        fn error(&self) -> Option<&str> { self.error.as_deref() }
        fn set_error(&mut self, error: Option<String>) { self.error = error }
        fn submit(&self) -> Result<Action, String> {
            if self.text(0).is_empty() {
                return Err("Name is required".into());
            }
            Ok(Action::Info(self.text(0).to_string()))
        }
    }

    #[test]
    fn typing_and_editing_go_to_the_focused_text_field() {
        let mut form = TestForm::default();
        type_str(&mut form, "hélo");
        press(&mut form, key(KeyCode::Left));
        press(&mut form, key(KeyCode::Char('l')));
        assert_eq!(form.text(0), "héllo");

        press(&mut form, key(KeyCode::Home));
        press(&mut form, key(KeyCode::Delete));
        press(&mut form, key(KeyCode::End));
        press(&mut form, key(KeyCode::Backspace));
        assert_eq!(form.text(0), "éll");

        // Space is bound to NextChoice, but types a space in a text field.
        press(&mut form, key(KeyCode::Char(' ')));
        assert_eq!(form.text(0), "éll ");
        // Shortcuts are not typed.
        press(&mut form, KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(form.text(0), "éll ");
    }

    #[test]
    fn tab_cycles_fields_and_choices_ignore_typing() {
        let mut form = TestForm::default();
        press(&mut form, key(KeyCode::Tab));
        assert_eq!(form.focus(), 1);

        press(&mut form, key(KeyCode::Char('x')));
        assert_eq!(form.selected(1), 0);
        press(&mut form, key(KeyCode::Right));
        assert_eq!(form.selected(1), 1);
        press(&mut form, key(KeyCode::Char(' ')));
        assert_eq!(form.selected(1), 0);
        press(&mut form, key(KeyCode::Left));
        assert_eq!(form.selected(1), 1);

        press(&mut form, key(KeyCode::Tab));
        assert_eq!(form.focus(), 0);
        press(&mut form, key(KeyCode::BackTab));
        assert_eq!(form.focus(), 1);
    }

    #[test]
    fn editing_clears_the_error() {
        let mut form = TestForm::default();
        form.set_error(Some("Name is required".into()));
        press(&mut form, key(KeyCode::Char('a')));
        assert_eq!(form.error(), None);
    }

    #[test]
    fn the_popup_shows_labels_the_hint_and_errors() {
        let mut form = TestForm::default();
        let shown = lines(&render(&form, 80, 24)).join("\n");
        assert!(shown.contains("Test"), "{shown}");
        assert!(shown.contains("› Name"), "{shown}");
        assert!(shown.contains("Colour"), "{shown}");
        assert!(shown.contains("Enter ok"), "{shown}");

        form.set_error(Some("Name is required".into()));
        let shown = lines(&render(&form, 80, 24)).join("\n");
        assert!(shown.contains("Name is required"), "{shown}");
        assert!(!shown.contains("Enter ok"), "{shown}");
    }

    #[test]
    fn the_popup_is_sized_from_the_field_count() {
        let form = TestForm::default();
        // 2 fields: 3 rows each, 1 row between, 1 row padding and a border.
        assert_eq!(form.height(), 11);

        let buffer = render(&form, 80, 24);
        let top = (0..24).find(|&y| buffer[(10, y)].symbol() == "┌").unwrap();
        assert_eq!(buffer[(10 + MODAL_WIDTH - 1, top + 11 - 1)].symbol(), "┘");
    }

    #[test]
    fn a_short_terminal_scrolls_to_the_focused_field() {
        let mut form = TestForm::default();
        form.set_focus(1);
        let shown = lines(&render(&form, 80, 8)).join("\n");
        assert!(shown.contains("› Colour"), "{shown}");
        assert!(!shown.contains("Name"), "{shown}");
    }
}
