use qry_core::copy::CopyFormat;

use super::{
    Submission,
    form::{Field, Form},
};
use crate::{action::Action, theme::Accent};

const LABELS: [&str; 2] = ["Copy", "As"];
const SCOPE: usize = 0;
const FORMAT: usize = 1;
const SCOPES: [&str; 4] = ["the cell", "the row", "the column", "the whole result"];
const FORMATS: [&str; 5] = ["CSV", "TSV", "JSON", "Markdown", "SQL INSERT"];

/// The results' copy menu: what to copy, and as what.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CopyForm {
    fields: [Field; 2],
    focus: usize,
    error: Option<String>,
}

impl Default for CopyForm {
    fn default() -> Self {
        Self {
            fields: [Field::choice(&SCOPES, 1), Field::choice(&FORMATS, 1)],
            focus: SCOPE,
            error: None,
        }
    }
}

impl Form for CopyForm {
    fn title(&self) -> &'static str {
        "Copy"
    }

    fn footer(&self) -> &'static [(Action, &'static str)] {
        &[
            (Action::Submit, "copy"),
            (Action::Cancel, "cancel"),
            (Action::NextField, "next"),
        ]
    }

    fn accent(&self) -> Accent {
        Accent::Act
    }

    fn width(&self) -> u16 {
        50
    }

    fn labels(&self) -> &'static [&'static str] {
        &LABELS
    }

    fn fields(&self) -> &[Field] {
        &self.fields
    }

    fn fields_mut(&mut self) -> &mut [Field] {
        &mut self.fields
    }

    fn focus(&self) -> usize {
        self.focus
    }

    fn set_focus(&mut self, focus: usize) {
        self.focus = focus;
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    fn submit(&self) -> Result<Submission, String> {
        Ok(Submission::Copy {
            scope: self.selected(SCOPE),
            format: CopyFormat::ALL[self.selected(FORMAT).min(CopyFormat::ALL.len() - 1)],
        })
    }
}
