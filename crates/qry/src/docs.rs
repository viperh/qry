//! The generated references under `docs/`: the key bindings, from the same
//! help model the in-app help draws, and the config file's keys, as a
//! table and as a JSON Schema.
//!
//! The tests in `docs_tests.rs` compare these with the files and fail on
//! drift; `QRY_BLESS=1` writes them instead.

use serde_json::{Map, Value, json};

use crate::{config::Config, help_model};

/// One key of the config file other than `keybindings`.
#[derive(Debug)]
pub struct ConfigKey {
    /// Dotted, as it nests: `history.max_entries`.
    pub path: &'static str,
    /// The JSON Schema type: `string`, `boolean`, `integer`, `array`.
    pub kind: &'static str,
    pub description: &'static str,
}

/// Every key the config reads besides the key bindings. A test checks this
/// against what `qry --print-config` prints, so a new key cannot be left
/// out.
pub const CONFIG_KEYS: &[ConfigKey] = &[
    ConfigKey {
        path: "theme",
        kind: "string",
        description: "The colour theme; `--theme` wins over it. Only `dark` exists for now.",
    },
    ConfigKey {
        path: "continue_on_error",
        kind: "boolean",
        description: "Run all (F8) keeps going after a statement fails, instead of stopping.",
    },
    ConfigKey {
        path: "max_rows",
        kind: "integer",
        description: "The most rows a query keeps; more are not read. 0 keeps every row.",
    },
    ConfigKey {
        path: "confirm_writes_on_prod",
        kind: "boolean",
        description: "The first write of a session on a connection labelled `prod` asks first.",
    },
    ConfigKey {
        path: "history.enabled",
        kind: "boolean",
        description: "Record the statements run, per connection.",
    },
    ConfigKey {
        path: "history.max_entries",
        kind: "integer",
        description: "The most history entries one connection keeps.",
    },
    ConfigKey {
        path: "history.ephemeral_to_disk",
        kind: "boolean",
        description: "Write the scratch database's history to disk too, instead of only \
                      keeping it for the run.",
    },
    ConfigKey {
        path: "history.redact",
        kind: "array",
        description: "Words whose string literal is stored as `'•••'` in the history, such \
                      as `PASSWORD`.",
    },
];

/// `docs/keybindings.md`: every group of the help, in its order, as a
/// `| Keys | Action |` table.
pub fn keybindings_markdown(config: &Config) -> String {
    let mut out = String::from(
        "# Key bindings\n\n\
         <!-- Generated from the built-in keymap by `crates/qry/src/docs.rs`; \
         run `QRY_BLESS=1 cargo test -p qry docs` after changing a binding. -->\n\n\
         These are the defaults, as `.config/config.json` sets them. A user config \
         can rebind or unbind any of them; see [config.md](config.md). The in-app \
         help (`?` or F1) lists the keys in effect.\n",
    );
    for group in help_model::build(&config.keymap) {
        out.push_str(&format!("\n## {}\n\n", group.title));
        out.push_str("| Keys | Action |\n| --- | --- |\n");
        for (keys, description) in &group.entries {
            out.push_str(&format!(
                "| {} | {} |\n",
                code(keys),
                description.replace('|', "\\|")
            ));
        }
        for note in &group.notes {
            out.push_str(&format!("\n{note}\n"));
        }
    }
    out
}

/// Each space-separated key as inline code, with `|` escaped for tables.
fn code(keys: &str) -> String {
    keys.split(' ')
        .map(|key| format!("`{}`", key.replace('|', "\\|")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The built-in value of every config key, as `qry --print-config` prints
/// it.
fn defaults(config: &Config) -> Map<String, Value> {
    let printed: Value = serde_json::from_str(&config.effective_json()).unwrap_or(Value::Null);
    let mut flat = Map::new();
    flatten("", &printed, &mut flat);
    flat
}

fn flatten(prefix: &str, value: &Value, out: &mut Map<String, Value>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if prefix.is_empty() && key == "keybindings" {
                    continue;
                }
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&path, value, out);
            }
        }
        value => {
            out.insert(prefix.to_string(), value.clone());
        }
    }
}

/// The keys `qry --print-config` prints, dotted, without the bindings.
pub fn printed_keys(config: &Config) -> Vec<String> {
    defaults(config).keys().cloned().collect()
}

/// The generated part of `docs/config.md`: one row per key.
pub fn config_markdown(config: &Config) -> String {
    let defaults = defaults(config);
    let mut out =
        String::from("| Key | Type | Default | Description |\n| --- | --- | --- | --- |\n");
    for key in CONFIG_KEYS {
        let default = defaults
            .get(key.path)
            .map_or_else(String::new, |v| format!("`{v}`"));
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            key.path, key.kind, default, key.description
        ));
    }
    out
}

/// `docs/config.schema.json`: the config file's shape, with the built-in
/// defaults.
pub fn config_schema(config: &Config) -> String {
    let defaults = defaults(config);
    let mut root = Map::new();
    for key in CONFIG_KEYS {
        let mut schema = json!({
            "type": key.kind,
            "description": key.description.replace('`', ""),
        });
        if let Some(default) = defaults.get(key.path) {
            schema["default"] = default.clone();
        }
        if key.kind == "array" {
            schema["items"] = json!({ "type": "string" });
        }
        if key.kind == "integer" {
            schema["minimum"] = json!(0);
        }
        match key.path.split_once('.') {
            None => {
                root.insert(key.path.to_string(), schema);
            }
            Some((parent, child)) => {
                let entry = root.entry(parent.to_string()).or_insert_with(
                    || json!({ "type": "object", "additionalProperties": false, "properties": {} }),
                );
                entry["properties"][child] = schema;
            }
        }
    }
    root.insert(
        "keybindings".into(),
        json!({
            "type": "object",
            "description": "Per context, a key (such as <ctrl-enter>) mapped to an action \
                            name, or to null to unbind it. See keybindings.md.",
            "additionalProperties": {
                "type": "object",
                "additionalProperties": { "type": ["string", "null"] }
            }
        }),
    );
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "qry configuration",
        "type": "object",
        "properties": Value::Object(root),
    });
    let mut text = serde_json::to_string_pretty(&schema).unwrap_or_default();
    text.push('\n');
    text
}

/// `file` with the text between its generated markers replaced by
/// `generated`; the hand-written text around them is kept.
pub fn replace_generated(file: &str, generated: &str) -> Option<String> {
    const BEGIN: &str = "<!-- BEGIN GENERATED -->\n";
    const END: &str = "<!-- END GENERATED -->";
    let start = file.find(BEGIN)? + BEGIN.len();
    let end = start + file[start..].find(END)?;
    Some(format!("{}{generated}{}", &file[..start], &file[end..]))
}

#[cfg(test)]
#[path = "docs_tests.rs"]
mod docs_tests;

/// A drawn screen as an SVG of monospaced text, each cell in its colours:
/// the README's screenshot, made from the real views so it never drifts
/// far from them.
pub fn screenshot_svg(
    buffer: &ratatui::buffer::Buffer,
    background: ratatui::style::Color,
) -> String {
    use ratatui::style::{Color, Modifier};
    const W: f32 = 8.4;
    const H: f32 = 17.0;
    // A 24-bit colour shows as `#RRGGBB`; anything else (the terminal's
    // default) takes the fallback.
    let hex = |color: Color, fallback: &str| {
        let shown = color.to_string();
        if shown.starts_with('#') {
            shown.to_lowercase()
        } else {
            fallback.to_string()
        }
    };
    let area = buffer.area;
    let bg = hex(background, "#000000");
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" \
         font-family=\"'JetBrains Mono','DejaVu Sans Mono',Menlo,Consolas,monospace\" \
         font-size=\"14\">\n<rect width=\"100%\" height=\"100%\" fill=\"{bg}\"/>\n",
        w = f32::from(area.width) * W,
        h = f32::from(area.height) * H,
    );
    for y in 0..area.height {
        // Runs of one background, then runs of one style of text.
        let mut x = 0;
        while x < area.width {
            let cell = &buffer[(x, y)];
            let start = x;
            while x < area.width && buffer[(x, y)].bg == cell.bg {
                x += 1;
            }
            let fill = hex(cell.bg, &bg);
            if fill != bg {
                out.push_str(&format!(
                    "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{H}\" fill=\"{fill}\"/>\n",
                    f32::from(start) * W,
                    f32::from(y) * H,
                    f32::from(x - start) * W,
                ));
            }
        }
        let mut x = 0;
        while x < area.width {
            let cell = &buffer[(x, y)];
            if cell.symbol().trim().is_empty() {
                x += 1;
                continue;
            }
            let start = x;
            let mut text = String::new();
            while x < area.width {
                let next = &buffer[(x, y)];
                if next.fg != cell.fg || next.modifier != cell.modifier {
                    break;
                }
                text.push_str(next.symbol());
                x += 1;
            }
            let text = text.trim_end();
            let escaped = text
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let mut attrs = format!("fill=\"{}\"", hex(cell.fg, "#d3d9e0"));
            if cell.modifier.contains(Modifier::BOLD) {
                attrs.push_str(" font-weight=\"bold\"");
            }
            if cell.modifier.contains(Modifier::ITALIC) {
                attrs.push_str(" font-style=\"italic\"");
            }
            if cell.modifier.contains(Modifier::UNDERLINED) {
                attrs.push_str(" text-decoration=\"underline\"");
            }
            out.push_str(&format!(
                "<text x=\"{:.1}\" y=\"{:.1}\" {attrs} xml:space=\"preserve\" \
                 textLength=\"{:.1}\">{escaped}</text>\n",
                f32::from(start) * W,
                f32::from(y) * H + 13.0,
                text.chars().count() as f32 * W,
            ));
        }
    }
    out.push_str("</svg>\n");
    out
}
