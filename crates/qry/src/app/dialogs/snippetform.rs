use super::{
    Submission,
    form::{Field, Form},
};
use crate::{action::Action, theme::Accent};

const LABELS: [&str; 2] = ["Name", "Scope"];
/// The scope choice: the session's connection, or every one.
pub(crate) const SCOPES: &[&str] = &["this connection", "global"];

/// What saving the form does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SnippetMode {
    /// Save this SQL. `replacing` names the snippet the editor was editing
    /// (its name and scope), which the save may overwrite.
    Save {
        sql: String,
        replacing: Option<(String, Option<String>)>,
    },
    /// Rename the snippet with this name and scope, or move it.
    Rename {
        name: String,
        connection: Option<String>,
    },
}

/// What the snippet dialog asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnippetSave {
    pub(crate) name: String,
    /// `None` for a global snippet, else the connection's id.
    pub(crate) connection: Option<String>,
    pub(crate) mode: SnippetMode,
}

/// Save snippet (ctrl-s in the editor) and Rename snippet: a name, unique
/// in its scope, and whether it is this connection's or global.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SnippetForm {
    fields: [Field; 2],
    focus: usize,
    error: Option<String>,
    /// The connection "this connection" means.
    connection: String,
    mode: SnippetMode,
}

impl SnippetForm {
    /// The form for `mode`, filled with `name` and the scope of
    /// `connection` (`None` is global). `here` is the session's connection.
    pub(crate) fn new(mode: SnippetMode, name: &str, connection: Option<&str>, here: &str) -> Self {
        let mut input = Field::text_with_placeholder("active users");
        if let Field::Text(text) = &mut input {
            for c in name.chars() {
                text.insert(c);
            }
        }
        // Another connection's snippet stays its own when renamed.
        let here = connection.unwrap_or(here).to_string();
        Self {
            fields: [
                input,
                Field::choice(SCOPES, usize::from(connection.is_none())),
            ],
            focus: 0,
            error: None,
            connection: here,
            mode,
        }
    }
}

impl Form for SnippetForm {
    fn title(&self) -> &'static str {
        match self.mode {
            SnippetMode::Save { .. } => "Save Snippet",
            SnippetMode::Rename { .. } => "Rename Snippet",
        }
    }

    fn footer(&self) -> &'static [(Action, &'static str)] {
        &[
            (Action::Submit, "save"),
            (Action::NextField, "next"),
            (Action::Cancel, "cancel"),
        ]
    }

    fn accent(&self) -> Accent {
        Accent::Act
    }

    fn width(&self) -> u16 {
        56
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
        let name = self.text(0).trim();
        if name.is_empty() {
            return Err("A snippet needs a name".into());
        }
        let connection = (self.selected(1) == 0).then(|| self.connection.clone());
        Ok(Submission::SaveSnippet(SnippetSave {
            name: name.to_string(),
            connection,
            mode: self.mode.clone(),
        }))
    }
}
