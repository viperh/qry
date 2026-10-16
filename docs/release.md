# Releasing qry

A release is a pushed tag `vX.Y.Z`. `.github/workflows/cd.yml` then builds
every target, checks the archives, publishes a GitHub release with notes
from `CHANGELOG.md`, and updates the package channels.

## Checklist

1. **Conventional commits throughout.** `feat`, `fix`, `perf`, `sec`,
   `refactor` and `docs` commits make the changelog; `build`, `ci`, `chore`
   and `test` stay out. Add each entry to `## [Unreleased]` in
   `CHANGELOG.md` as the change lands, under Added, Changed, Deprecated,
   Removed, Fixed or Security.
2. **Bump the version** in `Cargo.toml`: `workspace.package.version`, and the
   `version` of the `qry-core` path dependency in `[workspace.dependencies]`
   if it carries one.
3. **Date the changelog.** Rename `## [Unreleased]` to
   `## [X.Y.Z] - YYYY-MM-DD`, open a new empty `## [Unreleased]` above it,
   and update the compare links at the bottom of the file.
4. **Run `cargo test --workspace`**, so `Cargo.lock` and any snapshot that
   shows the version (the help overlay's title, T204) update. Review them
   with `cargo insta review`. Commit as `chore: release X.Y.Z`.
5. **Dry-run `cd.yml`.** Actions → CD → Run workflow on `master`. A manual
   run builds every target and uploads the `release-dry-run` artifact but
   publishes nothing. Download it and smoke-test each archive:
   - Linux: `tar -xzf qry-X.Y.Z-linux-x86_64.tar.gz && ./qry-X.Y.Z-linux-x86_64/qry --version`
   - macOS: the same with `macos-universal`, on Apple silicon and on Intel
     (or `arch -x86_64`).
   - Windows: unzip `qry-X.Y.Z-windows-x86_64.zip` and run `qry.exe --version`.
   - Check `sha256sum -c SHA256SUMS` and read `notes.md`.
6. **Push the tag:** `git tag -s vX.Y.Z -m "qry X.Y.Z" && git push origin vX.Y.Z`.
   The `meta` job fails at once if the tag does not match
   `workspace.package.version`, and the `release` job fails if
   `CHANGELOG.md` has no `## [X.Y.Z]` section.
7. **Verify each channel** once the `channels` job is done:
   - `yay -S qry-bin` (and `yay -S qry`, which builds from source)
   - `brew install viperh/tap/qry`
   - `scoop bucket add viperh https://github.com/viperh/scoop-bucket && scoop install qry`
   - `nix run github:viperh/qry/vX.Y.Z -- --version`

## What a release contains

| Archive | Built on | Binary |
|---|---|---|
| `qry-X.Y.Z-linux-x86_64.tar.gz` | ubuntu-latest, `x86_64-unknown-linux-musl` | static (musl, `+crt-static`) |
| `qry-X.Y.Z-linux-aarch64.tar.gz` | ubuntu-latest, `aarch64-unknown-linux-musl` via `cross` | static (musl, `+crt-static`) |
| `qry-X.Y.Z-macos-universal.tar.gz` | macos-14, `lipo` of x86_64 and arm64 | universal; signed and notarized when the Apple secrets are set |
| `qry-X.Y.Z-windows-x86_64.zip` | windows-latest, `x86_64-pc-windows-msvc` | with the icon and version info from `build.rs` |

Each archive holds one directory, `qry-X.Y.Z-<label>/`, with the binary,
`LICENSE`, `README.md`, `CHANGELOG.md`, `man/qry.1` and `completions/`
(`qry.bash`, `_qry` for zsh, `qry.fish`, `_qry.ps1`). Each has a `.sha256`
next to it, and `SHA256SUMS` lists them all (plus the optional CycloneDX
SBOMs). The release job checks `SHA256SUMS` before publishing.

The man page and completions come from the binary itself:

```sh
qry generate man > qry.1
qry generate completions bash|zsh|fish|powershell|elvish
```

### Reproducibility

- **Archives.** `scripts/release-package.sh` packs with GNU tar
  (`--sort=name --owner=0 --group=0 --numeric-owner --mtime=@$SOURCE_DATE_EPOCH`)
  and `gzip -n`, or `zip -X` with every mtime set to `SOURCE_DATE_EPOCH`.
  `SOURCE_DATE_EPOCH` is the tagged commit's time. Packing the same binary
  twice gives the same bytes. The `packaging` CI job checks it.
- **Binaries.** The Linux builds remap the checkout and cargo home
  (`--remap-path-prefix`) and strip symbols. The `linux` job builds the
  x86_64 binary a second time in another worktree at another path and
  `cmp`s the two.

## The keychain on musl (decision)

**Decision: (a), a pure-Rust Secret Service backend.** The static musl
builds keep full keychain support. Nothing had to change: it is what
`keyring` 4.2 already does by default.

What `keyring = "4.2.0"` (default features) pulls in, from
`cargo tree -e features -p qry --target x86_64-unknown-linux-musl`:

- `keyring/default` → `v1` → `zbus-secret-service-keyring-store` and
  `windows-native-keyring-store` (the Windows one only builds on Windows).
- `zbus-secret-service-keyring-store` → `secret-service` 5.2 with
  `crypto-rust`, on `zbus` 5: D-Bus implemented in Rust, and the session
  encryption in Rust too. No `libdbus-sys`, no `openssl-sys`, and no
  keyutils.

So no C library is linked for the keychain, and nothing stops a
`+crt-static` musl build. The `linux` job in `cd.yml` proves it on every
release: it fails unless `file` reports the binary as `statically linked`
or `static-pie linked`. To check by hand:

```sh
cargo tree -e features -p qry --target x86_64-unknown-linux-musl -i secret-service
cargo tree -p qry --target x86_64-unknown-linux-musl | grep -E 'dbus-sys|openssl-sys|keyutils'  # nothing
RUSTFLAGS="-C target-feature=+crt-static -C strip=symbols" \
  cargo build --locked --release -p qry --target x86_64-unknown-linux-musl   # needs musl-tools
file target/x86_64-unknown-linux-musl/release/qry   # static-pie linked
ldd target/x86_64-unknown-linux-musl/release/qry    # statically linked / not a dynamic executable
```

At run time the binary talks to whatever Secret Service owns
`org.freedesktop.secrets` on the session bus (GNOME Keyring, KWallet 6,
KeePassXC). With no session bus or no Secret Service, storing fails, and qry
takes its existing path for that (`crates/qry/src/db.rs`): the connection
stays open, the worker answers `SecretNotStored`, the saved record switches
to prompting, and an error toast says "… is saved, but the keychain would
not take its password: you will be asked each time".

The options not taken:

- (b) keyutils on musl only: passwords would not survive a reboot or
  logout. Not needed, since (a) links statically.
- (c) glibc builds on the oldest Ubuntu runner: still tied to a minimum
  glibc, which is what the musl builds avoid.

If a future `keyring` changes its defaults, check the tree again: a
`libdbus-sys` (the `sync-secret-service` / `dbus-secret-service` stores)
would break the static build.

### Manual verification (once per release that changes the keychain code)

| Platform | Steps | Expected |
|---|---|---|
| Linux desktop (Omarchy/Hyprland, gnome-keyring or KeePassXC) | Release binary: add a connection, connect, type the password with "remember" on, quit, start again, connect | Connects without asking; `secret-tool search service qry` lists the entry |
| Headless Linux (no session bus, e.g. `env -u DBUS_SESSION_BUS_ADDRESS`) | The same | The connection opens; a toast says the keychain would not take the password, and the next connect prompts |
| macOS | The same | The password is in Keychain Access under `qry` |
| Windows | The same | The password is in Credential Manager under `qry` |

## Package channels

The committed manifests in `packaging/` carry the placeholder version
`0.0.0` and all-zero checksums; only CI fills them in:

```
packaging/aur/qry/          PKGBUILD, .SRCINFO   builds from the tag's source tarball
packaging/aur/qry-bin/      PKGBUILD, .SRCINFO   the static musl archives
packaging/homebrew/qry.rb   macOS universal; Linux x86_64 and arm64
packaging/scoop/qry.json    Windows x86_64, with checkver and autoupdate
```

`scripts/update-packaging.py <version> SHA256SUMS --source-sha256 <hash> --out out`
writes the filled-in copies. `--self-test` checks it against a synthetic
`SHA256SUMS`; the `packaging` CI job runs that, lints each manifest, and
checks `.SRCINFO` against its PKGBUILD. After editing a PKGBUILD, run
`scripts/update-packaging.py --write-srcinfo`.

The `channels` job publishes each channel only when its secret is set, and
otherwise logs `::notice::skipping <channel>`:

| Channel | Secret | Target |
|---|---|---|
| AUR `qry`, `qry-bin` | `AUR_SSH_PRIVATE_KEY` | `ssh://aur@aur.archlinux.org/<pkg>.git` |
| Homebrew | `HOMEBREW_TAP_TOKEN` (contents: write on the tap) | `viperh/homebrew-tap`, `Formula/qry.rb` |
| Scoop | `SCOOP_BUCKET_TOKEN` (contents: write on the bucket) | `viperh/scoop-bucket`, `bucket/qry.json` |

macOS signing is optional too. It runs when `APPLE_CERTIFICATE_P12` is set,
and also needs `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
`APPLE_NOTARY_KEY` (the `.p8` contents), `APPLE_NOTARY_KEY_ID` and
`APPLE_NOTARY_ISSUER`. Without it the job logs
`::notice::unsigned macOS build`.

Nix users build from the flake: `nix run github:viperh/qry`. It needs no
release step.

## Out of scope

- crates.io publishing. The `packaging` CI job runs
  `cargo package -p qry-core --no-verify` so the crate stays publishable,
  but publishing is manual, if ever.
- Container images: qry is a desktop TUI.
