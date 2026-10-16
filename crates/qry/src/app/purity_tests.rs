//! The reducer and the views do no I/O: a scan of their source, like
//! sverb's `t02_reducer_modules_import_no_io`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::testing::sources;

#[test]
fn reducer_modules_import_no_io() {
    // Built from pieces, so this file does not trip its own scan.
    let forbidden = [
        ["tok", "io::"].concat(),
        ["std::", "fs"].concat(),
        ["std::", "net"].concat(),
        ["std::", "env"].concat(),
        ["Instant::", "now"].concat(),
        ["System", "Time"].concat(),
        ["key", "ring"].concat(),
        ["crate::", "runtime"].concat(),
        ["db::", "spawn"].concat(),
        // The history store's types may be named, not its files.
        ["File", "Store"].concat(),
        ["history_store::", "perform"].concat(),
        ["save_", "snippets"].concat(),
        ["load_", "snippets"].concat(),
    ];
    // The keychain and the stash are I/O; only the `Password` type may be
    // named from here.
    let secrets = ["secrets", "::"].concat();
    let password = [secrets.as_str(), "Password"].concat();

    let files = sources(&["app", "views"]);
    assert!(files.len() >= 20, "scanned only {} files", files.len());
    for (path, text) in &files {
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let at = format!("{}:{}", path.display(), n + 1);
            for word in &forbidden {
                assert!(
                    !code.contains(word.as_str()),
                    "{at}: reducer and view code must not use `{word}`: {line}"
                );
            }
            assert!(
                !code.replace(&password, "").contains(&secrets),
                "{at}: only `{password}` may come from the secrets module: {line}"
            );
        }
    }
}
