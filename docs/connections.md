# Connections

## Where they are kept

Saved connections live in `connections.json` in qry's data directory
(`$QRY_DATA`, else the platform's data directory; `qry --version` prints
it). The file is written whole, through a temporary file, every time a
connection is added, edited, duplicated or deleted. The scratch connection
that every run starts with (an in-memory SQLite database) is never saved.

Each record is a JSON object:

```json
{
  "id": "4f1c2a9e",
  "name": "prod-db",
  "driver": "postgres",
  "host": "db.example.com",
  "port": 5432,
  "user": "app",
  "database": "app",
  "schema": "public",
  "tls": "prefer",
  "secret": { "mode": "keyring" },
  "label": "prod"
}
```

`driver` is one of `postgres`, `mysql`, `mariadb`, `sqlite` and `oracle`.
Fields a driver does not use are left out: SQLite has `path` (and `mode`,
such as `ro`), Oracle has `service` and `wallet`, and any driver can be
given a whole `dsn` instead of its fields. `label` (`dev`, `staging` or
`prod`) colours the connection in the tree and the tab bar; `prod` also asks
before the first write of each session (config `confirm_writes_on_prod`).
Keys qry does not know are kept when the file is written again, so an older
qry does not drop what a newer one added.

## Passwords

A password is never written to `connections.json`, the history, the logs or
an export. `secret` says where it comes from:

| `secret` | Where the password is |
| --- | --- |
| `{ "mode": "keyring" }` | The OS keychain (Keychain, Credential Manager, Secret Service), under the connection's id. Typed once in the connection dialog. |
| `{ "mode": "env", "var": "PGPASSWORD_PROD" }` | An environment variable, read at each connect. |
| `{ "mode": "prompt" }` | Asked for at each connect, and kept only for that connection. |
| `{ "mode": "none" }` | No password: SQLite, trust or socket authentication. |

When the keychain cannot be reached (a headless Linux box with no Secret
Service, say), qry says so once and the connection falls back to asking each
time. Deleting a connection offers to forget its keychain entry and its
query history.

## Adding and testing

`ctrl-n` opens the connection dialog. Its fields follow the driver; `ctrl-t`
(or the Test button) connects once with what is typed and reports the
server's version, without saving anything. `e` on a connection in the tree
edits it, `c` duplicates it (the copy asks for its password) and `D` (or
Delete) deletes it.

## Importing

`ctrl-o` imports connections from:

- **qry**: a backup made with *Export connections* (palette). Any
  password-like field in a hand-edited backup is dropped.
- **DBeaver**: `data-sources.json` from a workspace's `.dbeaver` directory.
- **Oracle SQL Developer**: the connections JSON it exports.

The format is detected from the file. A preview lists every connection as
new (`+`), already here (`=`, same target) or skipped (`!`, with why), and
only the chosen ones are saved. Imported connections never carry a
password: SQL Developer and DBeaver keep theirs encrypted or in their own
stores, so each imported connection asks on its first connect.

## Sessions

Opening a connection starts a *session*: its own database connection,
editor, results and history, shown as a tab in the top bar. A connection can
have several sessions (shift-Enter in the tree) for, say, two transactions at once.
`ctrl-1` … `ctrl-9` switch between them and `ctrl-w` closes one; closing a
session with a running statement or an open transaction asks first.
