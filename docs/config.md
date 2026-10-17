# Configuration

qry reads its built-in defaults (`.config/config.json` in the repository,
baked into the binary) and layers a user file over them. The user file is
`config.json5`, `config.json`, `config.yaml`, `config.toml` or `config.ini`
in the config directory: `$QRY_CONFIG` if set, else the platform's config
directory (`~/.config/qry/` on Linux). `qry --version` prints the
directories in use, and `qry --print-config` prints the config in effect.

A user file only needs the keys it changes:

```json5
{
  "max_rows": 5000,
  "history": { "redact": ["PASSWORD", "SECRET", "TOKEN"] },
  "keybindings": {
    // Run the statement with alt-r as well as ctrl-Enter.
    "Editor": { "<alt-r>": "RunStatement" },
    // Unbind a key with null.
    "Results": { "<F>": null }
  }
}
```

## Keys

<!-- BEGIN GENERATED -->
| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `theme` | string | `"dark"` | The colour theme; `--theme` wins over it. Only `dark` exists for now. |
| `continue_on_error` | boolean | `false` | Run all (F8) keeps going after a statement fails, instead of stopping. |
| `max_rows` | integer | `1000` | The most rows a query keeps; more are not read. 0 keeps every row. |
| `confirm_writes_on_prod` | boolean | `true` | The first write of a session on a connection labelled `prod` asks first. |
| `history.enabled` | boolean | `true` | Record the statements run, per connection. |
| `history.max_entries` | integer | `1000` | The most history entries one connection keeps. |
| `history.ephemeral_to_disk` | boolean | `false` | Write the scratch database's history to disk too, instead of only keeping it for the run. |
| `history.redact` | array | `["PASSWORD","IDENTIFIED BY","SECRET"]` | Words whose string literal is stored as `'•••'` in the history, such as `PASSWORD`. |
<!-- END GENERATED -->

## Key bindings

`keybindings` maps, per context, a key or key sequence to an action. The
contexts are `Global`, `Tree`, `Editor`, `Results`, `Inspector`, `Jobs`,
`Dialog`, `Help` and `Palette`; a key in `Global` works wherever the focused
pane does not bind it. Keys are written in angle brackets, modifiers first:
`<ctrl-enter>`, `<shift-tab>`, `<alt-up>`, `<F5>`; a capital letter is the
shifted key, `<G>`. A sequence is several keys in a row, `<g><g>`. The
actions and their defaults are listed in [keybindings.md](keybindings.md).

The old per-pane format (`"panes": {...}`) is still read and turned into
contexts, with a warning.

## Environment

| Variable | What it does |
| --- | --- |
| `QRY_CONFIG` | The directory the user config is read from. |
| `QRY_DATA` | The directory connections, history and logs are kept in. |
| `QRY_LOG_LEVEL` | The log file's level, such as `debug` (the default is `info`). |
| `NO_COLOR` | Draw without colour; see [theme.md](theme.md). |

The schema of the file is in [config.schema.json](config.schema.json), for
editors that validate JSON against one.
