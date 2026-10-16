use std::{env, path::PathBuf, sync::LazyLock};

use color_eyre::eyre::{WrapErr, eyre};
use directories::ProjectDirs;
use serde::{Deserialize, de::IgnoredAny};
use serde_json::{Map, Value};
use strum::IntoEnumIterator;
use tracing::error;

use crate::{
    action::Action,
    config_migrate::{self as migrate, Migrated},
    keymap::{KeyContext, Keymap, chord_to_string, display_order, parse_key_sequence},
};

/// The default config, baked into the binary at compile time. User config
/// files found in [`get_config_dir`] are layered on top of it.
const CONFIG: &str = include_str!("../../../.config/config.json");

/// What errors and warnings call the built-in config.
const EMBEDDED: &str = "the built-in config";

/// The user config files looked for in [`get_config_dir`], in the order they
/// are layered: a later one wins.
const CONFIG_FILES: [(&str, config::FileFormat); 5] = [
    ("config.json5", config::FileFormat::Json5),
    // JSON5 reads every JSON file too, and lets `config.json` carry comments
    // like the built-in one does: pointing `QRY_CONFIG` at the repo's
    // `.config/` (as `.envrc` does) layers that very file.
    ("config.json", config::FileFormat::Json5),
    ("config.yaml", config::FileFormat::Yaml),
    ("config.toml", config::FileFormat::Toml),
    ("config.ini", config::FileFormat::Ini),
];

/// Reverse-domain qualifier and organization used to locate the per-user
/// config and data directories. Change these when you rename the project.
const APP_QUALIFIER: &str = "dev";
const APP_ORGANIZATION: &str = "viperh";

/// The config in effect: the built-in one with the user's files over it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    /// Every key binding; see [`crate::keymap`].
    pub(crate) keymap: Keymap,
    /// The theme's name (`--theme` wins over it); see [`crate::theme`].
    pub(crate) theme: Option<String>,
    /// Run all keeps going after a statement fails, instead of stopping.
    pub(crate) continue_on_error: bool,
    /// What the query history records, and how much of it.
    pub(crate) history: HistoryConfig,
    /// Problems that did not stop the config from loading; see
    /// [`Config::warnings`].
    warnings: Vec<String>,
}

/// One config file as written, before its bindings are read.
#[derive(Debug, Default, Deserialize)]
struct RawFile {
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    continue_on_error: Option<bool>,
    #[serde(default)]
    history: Option<RawHistory>,
    /// `{ context: { chord: action or null } }`.
    #[serde(default)]
    keybindings: Option<Map<String, Value>>,
    /// The old format's per-pane keys; see [`crate::config_migrate`].
    #[serde(default)]
    panes: Option<Map<String, Value>>,
    /// The template's old `styles` section. Themes replaced it, so it is
    /// only noticed, to warn that it is ignored.
    #[serde(default)]
    styles: Option<IgnoredAny>,
}

/// The query history's settings; see [`crate::history_store`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryConfig {
    /// Record statements at all.
    pub(crate) enabled: bool,
    /// The most entries one connection keeps.
    pub(crate) max_entries: usize,
    /// Write the scratch database's history to disk too, instead of only
    /// keeping it for the run.
    pub(crate) ephemeral_to_disk: bool,
    /// The words whose string literal is stored as `'•••'`.
    pub(crate) redact: Vec<String>,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: qry_core::history::DEFAULT_CAP,
            ephemeral_to_disk: false,
            redact: qry_core::history::DEFAULT_REDACT
                .iter()
                .map(|w| (*w).to_string())
                .collect(),
        }
    }
}

/// The `history` section as written: each key is optional.
#[derive(Debug, Default, Deserialize)]
struct RawHistory {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    max_entries: Option<usize>,
    #[serde(default)]
    ephemeral_to_disk: Option<bool>,
    #[serde(default)]
    redact: Option<Vec<String>>,
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

impl Config {
    /// The built-in config with every user file in [`get_config_dir`] over
    /// it. A file that cannot be read, a key that cannot be parsed or an
    /// unknown action or context is an error naming the file.
    pub(crate) fn new() -> color_eyre::Result<Self> {
        let config_dir = get_config_dir();
        let mut files = Vec::new();
        for (file, format) in CONFIG_FILES {
            let path = config_dir.join(file);
            if !path.exists() {
                continue;
            }
            let name = path.display().to_string();
            let raw: RawFile = config::Config::builder()
                .add_source(config::File::from(path).format(format))
                .build()
                .and_then(config::Config::try_deserialize)
                .wrap_err_with(|| format!("cannot read {name}"))?;
            files.push((name, raw));
        }
        if files.is_empty() {
            error!("No configuration file found. Application may not behave as expected");
        }
        Self::layered(files)
    }

    /// The built-in config with `files` over it, in order.
    fn layered(files: Vec<(String, RawFile)>) -> color_eyre::Result<Self> {
        let embedded: RawFile = json5::from_str(CONFIG).wrap_err("the built-in config")?;
        let mut config = Self::default();
        config.apply(EMBEDDED, embedded)?;
        for (name, file) in files {
            config.apply(&name, file)?;
        }
        config.check_prefixes();
        Ok(config)
    }

    /// Layers one file over what is loaded so far: its theme, and each of
    /// its bindings over the same chord's (`null` unbinds it).
    fn apply(&mut self, name: &str, file: RawFile) -> color_eyre::Result<()> {
        if file.styles.is_some() {
            self.warnings.push(
                "the config's \"styles\" section is ignored; colours come from the theme \
                 (\"theme\": \"dark\")"
                    .to_string(),
            );
        }
        if file.theme.is_some() {
            self.theme = file.theme;
        }
        if let Some(keep_going) = file.continue_on_error {
            self.continue_on_error = keep_going;
        }
        if let Some(history) = file.history {
            let h = &mut self.history;
            h.enabled = history.enabled.unwrap_or(h.enabled);
            h.max_entries = history.max_entries.unwrap_or(h.max_entries).max(1);
            h.ephemeral_to_disk = history.ephemeral_to_disk.unwrap_or(h.ephemeral_to_disk);
            if let Some(redact) = history.redact {
                h.redact = redact;
            }
        }

        let mut old = Migrated::default();
        let mut old_format = file.panes.is_some();
        for (section, bindings) in file.keybindings.unwrap_or_default() {
            if let Some(context) = context_named(&section) {
                self.read_section(name, context, &bindings)?;
            } else if let Some(context) = migrate::old_context(&section) {
                old_format = true;
                migrate::mode_section(&section, context, &bindings, &mut old);
            } else {
                let contexts: Vec<_> = KeyContext::iter().map(|c| c.to_string()).collect();
                return Err(eyre!(
                    "{name}: \"keybindings\" has an unknown context \"{section}\"; \
                     the contexts are: {}",
                    contexts.join(", ")
                ));
            }
        }
        if let Some(panes) = &file.panes {
            migrate::panes(panes, &mut old);
        }
        if old_format {
            for (context, chord, action) in &old.bindings {
                self.keymap.bind(*context, chord, *action);
            }
            let mut warning = format!(
                "{name}: config uses the old keybinding format; it was converted, run qry \
                 --print-config to see the new one"
            );
            if !old.skipped.is_empty() {
                warning.push_str(&format!(
                    "; these keys could not be converted and were skipped: {}",
                    old.skipped.join(", ")
                ));
            }
            self.warnings.push(warning);
        }
        Ok(())
    }

    /// Reads one context's `{ chord: action or null }`.
    fn read_section(
        &mut self,
        name: &str,
        context: KeyContext,
        bindings: &Value,
    ) -> color_eyre::Result<()> {
        let Value::Object(bindings) = bindings else {
            return Err(eyre!(
                "{name}: keybindings.{context} must map keys to actions, not {bindings}"
            ));
        };
        for (raw, action) in bindings {
            let chord = parse_key_sequence(raw).map_err(|e| {
                eyre!("{name}: keybindings.{context}: cannot read the key \"{raw}\": {e}")
            })?;
            if action.is_null() {
                self.keymap.unbind(context, &chord);
                continue;
            }
            let action = serde_json::from_value::<Action>(action.clone()).map_err(|e| {
                eyre!(
                    "{name}: keybindings.{context}: \"{raw}\" is bound to {action}, which \
                     is not an action ({e})"
                )
            })?;
            self.keymap.bind(context, &chord, action);
        }
        Ok(())
    }

    /// One warning for every binding that starts a longer one in the same
    /// context: it can only run once the sequence times out.
    fn check_prefixes(&mut self) {
        let conflicts = self.keymap.prefix_conflicts();
        if conflicts.is_empty() {
            return;
        }
        let listed: Vec<String> = conflicts
            .iter()
            .map(|(context, short, long)| {
                format!(
                    "{context} `{}` and `{}`",
                    chord_to_string(short),
                    chord_to_string(long)
                )
            })
            .collect();
        self.warnings.push(format!(
            "these keys start a longer binding, so they only run after waiting a second: {}",
            listed.join("; ")
        ));
    }

    /// Problems with the config that do not stop qry from starting, one line
    /// each, for the runtime to report.
    pub(crate) fn warnings(&self) -> Vec<String> {
        self.warnings.clone()
    }

    /// The keymap in effect, written in the config's own format: what
    /// `qry --print-config` prints. Contexts come in their usual order, and
    /// within one the bindings follow the actions' order, an action's chords
    /// plain and short first. Read back as a user file, it gives the same
    /// keymap.
    pub(crate) fn effective_json(&self) -> String {
        let quote = |s: &str| Value::String(s.to_string()).to_string();
        let mut out = String::from("{\n");
        if let Some(theme) = &self.theme {
            out.push_str(&format!("  \"theme\": {},\n", quote(theme)));
        }
        out.push_str(&format!(
            "  \"continue_on_error\": {},\n",
            self.continue_on_error
        ));
        let h = &self.history;
        let redact: Vec<String> = h.redact.iter().map(|w| quote(w)).collect();
        out.push_str(&format!(
            "  \"history\": {{\n    \"enabled\": {},\n    \"max_entries\": {},\n    \
             \"ephemeral_to_disk\": {},\n    \"redact\": [{}]\n  }},\n",
            h.enabled,
            h.max_entries,
            h.ephemeral_to_disk,
            redact.join(", ")
        ));
        out.push_str("  \"keybindings\": {");
        for (i, context) in KeyContext::iter().enumerate() {
            let mut bindings: Vec<_> = self
                .keymap
                .bindings(context)
                .map(|(chord, action)| (action.rank(), display_order(chord), *action))
                .collect();
            bindings.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
            out.push_str(if i == 0 { "\n" } else { ",\n" });
            out.push_str(&format!("    {}: {{", quote(&context.to_string())));
            for (j, (_, (_, _, chord), action)) in bindings.iter().enumerate() {
                out.push_str(if j == 0 { "\n" } else { ",\n" });
                let action = serde_json::to_string(action).unwrap_or_default();
                out.push_str(&format!("      {}: {action}", quote(chord)));
            }
            out.push_str(if bindings.is_empty() { "}" } else { "\n    }" });
        }
        out.push_str("\n  }\n}\n");
        out
    }

    /// Only the built-in config, ignoring any user files. Keeps tests
    /// independent of the machine they run on.
    #[cfg(any(test, feature = "test-util"))]
    #[allow(
        clippy::expect_used,
        reason = "the embedded config is a constant that every test parses"
    )]
    pub fn embedded() -> Self {
        Self::layered(Vec::new()).expect("the embedded config is valid")
    }

    /// The built-in config with one user file over it, written as JSON and
    /// read the way [`Config::new`] reads `config.json`. `path` only names it
    /// in errors and warnings.
    #[cfg(any(test, feature = "test-util"))]
    pub fn with_user_file(path: &str, json: &str) -> color_eyre::Result<Self> {
        let raw: RawFile = config::Config::builder()
            .add_source(config::File::from_str(json, config::FileFormat::Json))
            .build()
            .and_then(config::Config::try_deserialize)
            .wrap_err_with(|| format!("cannot read {path}"))?;
        Self::layered(vec![(path.to_string(), raw)])
    }
}

/// The context a `keybindings` section names, if it is one.
fn context_named(section: &str) -> Option<KeyContext> {
    KeyContext::iter().find(|context| context.to_string() == section)
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

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
