use std::{collections::HashMap, env, path::PathBuf, sync::LazyLock};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use directories::ProjectDirs;
use serde::{
    Deserialize, Serialize,
    de::{Deserializer, IgnoredAny},
};
use strum::EnumIter;
use tracing::error;

use crate::{action::Action, keymap::PaneKeys};

/// The default config, baked into the binary at compile time. User config
/// files found in [`get_config_dir`] are layered on top of it.
const CONFIG: &str = include_str!("../../../.config/config.json");

/// Reverse-domain qualifier and organization used to locate the per-user
/// config and data directories. Change these when you rename the project.
const APP_QUALIFIER: &str = "dev";
const APP_ORGANIZATION: &str = "viperh";

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub(crate) keybindings: KeyBindings,
    /// Keys that act inside one pane; see [`crate::keymap`].
    #[serde(default)]
    pub(crate) panes: PaneKeys,
    /// The theme's name (`--theme` wins over it); see [`crate::theme`].
    #[serde(default)]
    pub(crate) theme: Option<String>,
    /// The template's old `styles` section. Themes replaced it, so it is
    /// only noticed, to warn that it is ignored.
    #[serde(default)]
    styles: Option<IgnoredAny>,
}

/// Upper-cased crate name, used as the prefix for the `*_DATA`, `*_CONFIG`
/// and `*_LOG_LEVEL` environment variables (see `.envrc`).
pub(crate) static PROJECT_NAME: LazyLock<String> =
    LazyLock::new(|| env!("CARGO_CRATE_NAME").to_uppercase().to_string());
pub(crate) static DATA_FOLDER: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    env::var(format!("{}_DATA", PROJECT_NAME.clone()))
        .ok()
        .map(PathBuf::from)
});
pub(crate) static CONFIG_FOLDER: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    env::var(format!("{}_CONFIG", PROJECT_NAME.clone()))
        .ok()
        .map(PathBuf::from)
});

/// The directory as a string for the `config` crate, which only stores UTF-8.
/// A non-UTF-8 path is an error rather than a silently mangled path.
fn utf8_dir<'a>(kind: &str, dir: &'a std::path::Path) -> Result<&'a str, config::ConfigError> {
    dir.to_str().ok_or_else(|| {
        config::ConfigError::Message(format!(
            "the {kind} directory {} is not valid UTF-8",
            dir.display()
        ))
    })
}

impl Config {
    pub(crate) fn new() -> color_eyre::Result<Self, config::ConfigError> {
        let default_config: Config =
            json5::from_str(CONFIG).map_err(|e| config::ConfigError::Foreign(Box::new(e)))?;
        let data_dir = get_data_dir();
        let config_dir = get_config_dir();
        let mut builder = config::Config::builder()
            .set_default("data_dir", utf8_dir("data", &data_dir)?)?
            .set_default("config_dir", utf8_dir("config", &config_dir)?)?;

        let config_files = [
            ("config.json5", config::FileFormat::Json5),
            ("config.json", config::FileFormat::Json),
            ("config.yaml", config::FileFormat::Yaml),
            ("config.toml", config::FileFormat::Toml),
            ("config.ini", config::FileFormat::Ini),
        ];
        let mut found_config = false;
        for (file, format) in &config_files {
            let source = config::File::from(config_dir.join(file))
                .format(*format)
                .required(false);
            builder = builder.add_source(source);
            if config_dir.join(file).exists() {
                found_config = true
            }
        }
        if !found_config {
            error!("No configuration file found. Application may not behave as expected");
        }

        let mut cfg: Self = builder.build()?.try_deserialize()?;

        for (mode, default_bindings) in default_config.keybindings.0.iter() {
            let user_bindings = cfg.keybindings.0.entry(*mode).or_default();
            for (key, cmd) in default_bindings.iter() {
                user_bindings
                    .entry(key.clone())
                    .or_insert_with(|| cmd.clone());
            }
        }
        cfg.panes.merge_defaults(&default_config.panes);
        if cfg.theme.is_none() {
            cfg.theme = default_config.theme;
        }

        Ok(cfg)
    }

    /// Problems with the config that do not stop qry from starting, one line
    /// each, for the runtime to report.
    pub(crate) fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.styles.is_some() {
            warnings.push(
                "the config's \"styles\" section is ignored; colours come from the theme \
                 (\"theme\": \"dark\")"
                    .to_string(),
            );
        }
        warnings
    }

    /// Only the built-in config, ignoring any user files. Keeps tests
    /// independent of the machine they run on.
    #[cfg(any(test, feature = "test-util"))]
    #[allow(
        clippy::expect_used,
        reason = "the embedded config is a constant that every test parses"
    )]
    pub fn embedded() -> Self {
        json5::from_str(CONFIG).expect("the embedded config is valid")
    }
}

pub(crate) fn get_data_dir() -> PathBuf {
    if let Some(s) = DATA_FOLDER.clone() {
        s
    } else if let Some(proj_dirs) = project_directory() {
        proj_dirs.data_local_dir().to_path_buf()
    } else {
        PathBuf::from(".").join(".data")
    }
}

pub(crate) fn get_config_dir() -> PathBuf {
    if let Some(s) = CONFIG_FOLDER.clone() {
        s
    } else if let Some(proj_dirs) = project_directory() {
        proj_dirs.config_local_dir().to_path_buf()
    } else {
        PathBuf::from(".").join(".config")
    }
}

fn project_directory() -> Option<ProjectDirs> {
    ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, env!("CARGO_PKG_NAME"))
}

/// The sections of the `keybindings` config: one for the main screen and one
/// per modal. Which one applies follows from what is open (see
/// [`State::key_section`](crate::app::state::State::key_section)); the
/// names are the ones the config has always used.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, EnumIter)]
pub enum KeySection {
    #[default]
    Home,
    AddConnModal,
    ExpoModal,
    PasswordModal,
    ImportModal,
    ConnExportModal,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct KeyBindings(pub HashMap<KeySection, HashMap<Vec<KeyEvent>, Action>>);

impl<'de> Deserialize<'de> for KeyBindings {
    fn deserialize<D>(deserializer: D) -> color_eyre::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let parsed_map = HashMap::<KeySection, HashMap<String, Action>>::deserialize(deserializer)?;

        let keybindings = parsed_map
            .into_iter()
            .map(|(mode, inner_map)| {
                let converted_inner_map = inner_map
                    .into_iter()
                    .map(|(key_str, cmd)| {
                        parse_key_sequence(&key_str)
                            .map(|keys| (keys, cmd))
                            .map_err(|e| {
                                serde::de::Error::custom(format!(
                                    "invalid key sequence `{key_str}`: {e}"
                                ))
                            })
                    })
                    .collect::<Result<_, D::Error>>()?;
                Ok((mode, converted_inner_map))
            })
            .collect::<Result<_, D::Error>>()?;

        Ok(KeyBindings(keybindings))
    }
}

pub(crate) fn parse_key_event(raw: &str) -> color_eyre::Result<KeyEvent, String> {
    let raw_lower = raw.to_ascii_lowercase();
    let (remaining, modifiers) = extract_modifiers(&raw_lower);
    parse_key_code_with_modifiers(remaining, modifiers)
}

fn extract_modifiers(raw: &str) -> (&str, KeyModifiers) {
    let mut modifiers = KeyModifiers::empty();
    let mut current = raw;

    loop {
        match current {
            rest if rest.starts_with("ctrl-") => {
                modifiers.insert(KeyModifiers::CONTROL);
                current = &rest[5..];
            }
            rest if rest.starts_with("alt-") => {
                modifiers.insert(KeyModifiers::ALT);
                current = &rest[4..];
            }
            rest if rest.starts_with("shift-") => {
                modifiers.insert(KeyModifiers::SHIFT);
                current = &rest[6..];
            }
            _ => break, // break out of the loop if no known prefix is detected
        };
    }

    (current, modifiers)
}

fn parse_key_code_with_modifiers(
    raw: &str,
    mut modifiers: KeyModifiers,
) -> color_eyre::Result<KeyEvent, String> {
    let c = match raw {
        "esc" => KeyCode::Esc,
        "enter" => KeyCode::Enter,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "backtab" => {
            modifiers.insert(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "f1" => KeyCode::F(1),
        "f2" => KeyCode::F(2),
        "f3" => KeyCode::F(3),
        "f4" => KeyCode::F(4),
        "f5" => KeyCode::F(5),
        "f6" => KeyCode::F(6),
        "f7" => KeyCode::F(7),
        "f8" => KeyCode::F(8),
        "f9" => KeyCode::F(9),
        "f10" => KeyCode::F(10),
        "f11" => KeyCode::F(11),
        "f12" => KeyCode::F(12),
        "space" => KeyCode::Char(' '),
        "hyphen" => KeyCode::Char('-'),
        "minus" => KeyCode::Char('-'),
        "tab" => KeyCode::Tab,
        c if c.len() == 1 => {
            #[allow(clippy::unwrap_used, reason = "the guard checked there is one char")]
            let mut c = c.chars().next().unwrap();
            if modifiers.contains(KeyModifiers::SHIFT) {
                c = c.to_ascii_uppercase();
            }
            KeyCode::Char(c)
        }
        _ => return Err(format!("Unable to parse {raw}")),
    };
    Ok(KeyEvent::new(c, modifiers))
}

#[cfg(test)]
pub(crate) fn key_event_to_string(key_event: &KeyEvent) -> String {
    let char;
    let key_code = match key_event.code {
        KeyCode::Backspace => "backspace",
        KeyCode::Enter => "enter",
        KeyCode::Left => "left",
        KeyCode::Right => "right",
        KeyCode::Up => "up",
        KeyCode::Down => "down",
        KeyCode::Home => "home",
        KeyCode::End => "end",
        KeyCode::PageUp => "pageup",
        KeyCode::PageDown => "pagedown",
        KeyCode::Tab => "tab",
        KeyCode::BackTab => "backtab",
        KeyCode::Delete => "delete",
        KeyCode::Insert => "insert",
        KeyCode::F(c) => {
            char = format!("f({c})");
            &char
        }
        KeyCode::Char(' ') => "space",
        KeyCode::Char(c) => {
            char = c.to_string();
            &char
        }
        KeyCode::Esc => "esc",
        KeyCode::Null => "",
        KeyCode::CapsLock => "",
        KeyCode::Menu => "",
        KeyCode::ScrollLock => "",
        KeyCode::Media(_) => "",
        KeyCode::NumLock => "",
        KeyCode::PrintScreen => "",
        KeyCode::Pause => "",
        KeyCode::KeypadBegin => "",
        KeyCode::Modifier(_) => "",
    };

    let mut modifiers = Vec::with_capacity(3);

    if key_event.modifiers.intersects(KeyModifiers::CONTROL) {
        modifiers.push("ctrl");
    }

    if key_event.modifiers.intersects(KeyModifiers::SHIFT) {
        modifiers.push("shift");
    }

    if key_event.modifiers.intersects(KeyModifiers::ALT) {
        modifiers.push("alt");
    }

    let mut key = modifiers.join("-");

    if !key.is_empty() {
        key.push('-');
    }
    key.push_str(key_code);

    key
}

pub(crate) fn parse_key_sequence(raw: &str) -> color_eyre::Result<Vec<KeyEvent>, String> {
    if raw.chars().filter(|c| *c == '>').count() != raw.chars().filter(|c| *c == '<').count() {
        return Err(format!("Unable to parse `{raw}`"));
    }
    let raw = if !raw.contains("><") {
        let raw = raw.strip_prefix('<').unwrap_or(raw);
        raw.strip_prefix('>').unwrap_or(raw)
    } else {
        raw
    };
    let sequences = raw
        .split("><")
        .map(|seq| {
            if let Some(s) = seq.strip_prefix('<') {
                s
            } else if let Some(s) = seq.strip_suffix('>') {
                s
            } else {
                seq
            }
        })
        .collect::<Vec<_>>();

    sequences.into_iter().map(parse_key_event).collect()
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
