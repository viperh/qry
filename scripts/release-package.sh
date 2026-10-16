#!/usr/bin/env bash
# Packs one release archive, reproducibly.
#
#   scripts/release-package.sh <version> <label> <binary> <out-dir> [assets-dir]
#
# Writes <out-dir>/qry-<version>-<label>.tar.gz (or .zip for a windows-*
# label) and a .sha256 next to it, in `sha256sum` format so the release job
# can concatenate them into SHA256SUMS. The archive holds one directory,
# qry-<version>-<label>/, with the binary, LICENSE, README.md, CHANGELOG.md,
# man/qry.1 and completions/*. [assets-dir] is where `qry generate` wrote
# qry.1 and the completion scripts (the `qry-assets` artifact); without it
# the man page and completions are generated with <binary> itself, which
# only works when the binary runs on this machine.
#
# Two runs with the same inputs and SOURCE_DATE_EPOCH give byte-identical
# archives: GNU tar sorts entries, zeroes owners and pins every mtime, and
# gzip -n leaves out the name and time. SOURCE_DATE_EPOCH defaults to the
# last commit's time. On macOS install GNU tar (`brew install gnu-tar`);
# the script uses `gtar` when it exists.
#
# Labels: linux-x86_64, linux-aarch64, macos-universal, windows-x86_64.

set -euo pipefail

usage() {
  echo "usage: $0 <version> <label> <binary> <out-dir> [assets-dir]" >&2
  exit 2
}

[[ $# -eq 4 || $# -eq 5 ]] || usage
version=$1
label=$2
binary=$3
out=$4
assets=${5:-}

[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.+-]+)?$ ]] \
  || { echo "error: '$version' is not a version like 1.2.3" >&2; exit 2; }
case $label in
  linux-x86_64 | linux-aarch64 | macos-universal | windows-x86_64) ;;
  *) echo "error: unknown label '$label'" >&2; exit 2 ;;
esac
[[ -f $binary ]] || { echo "error: no binary at '$binary'" >&2; exit 2; }

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
: "${SOURCE_DATE_EPOCH:=$(git -C "$root" log -1 --format=%ct)}"
export SOURCE_DATE_EPOCH
# Files keep the same order and permissions whatever the caller's umask
# and locale.
export LC_ALL=C TZ=UTC
umask 022

if command -v gtar >/dev/null; then
  tar=gtar
else
  tar=tar
fi
"$tar" --version 2>/dev/null | grep -q 'GNU tar' \
  || { echo "error: GNU tar is required (on macOS: brew install gnu-tar)" >&2; exit 2; }

if command -v sha256sum >/dev/null; then
  sha256() { sha256sum "$@"; }
else
  sha256() { shasum -a 256 "$@"; }
fi

stem="qry-$version-$label"
exe=qry
[[ $label == windows-* ]] && exe=qry.exe

mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
dir="$work/$stem"
mkdir -p "$dir/man" "$dir/completions"

install -m 0755 "$binary" "$dir/$exe"
for f in LICENSE README.md CHANGELOG.md; do
  install -m 0644 "$root/$f" "$dir/$f"
done

if [[ -z $assets ]]; then
  assets="$work/assets"
  mkdir -p "$assets"
  "$binary" generate man >"$assets/qry.1"
  "$binary" generate completions bash >"$assets/qry.bash"
  "$binary" generate completions zsh >"$assets/_qry"
  "$binary" generate completions fish >"$assets/qry.fish"
  "$binary" generate completions powershell >"$assets/_qry.ps1"
fi
install -m 0644 "$assets/qry.1" "$dir/man/qry.1"
for f in qry.bash _qry qry.fish _qry.ps1; do
  [[ -f $assets/$f ]] || { echo "error: $assets/$f is missing" >&2; exit 1; }
  install -m 0644 "$assets/$f" "$dir/completions/$f"
done

if [[ $label == windows-* ]]; then
  archive="$stem.zip"
  rm -f "$out/$archive"
  # zip has no --mtime, so every entry gets SOURCE_DATE_EPOCH on disk
  # first (GNU touch: Git Bash on Windows, or Linux).
  find "$dir" -exec touch -d "@$SOURCE_DATE_EPOCH" {} +
  # -X leaves out extra attributes (uid/gid, extended timestamps); the
  # sorted file list fixes the entry order.
  # Zipped next to the directory and moved: a native Windows zip.exe never
  # sees an MSYS path.
  (cd "$work" && find "$stem" -print | sort | zip -X -q -9 "$archive" -@)
  mv "$work/$archive" "$out/$archive"
else
  archive="$stem.tar.gz"
  "$tar" -C "$work" --sort=name --owner=0 --group=0 --numeric-owner \
    --mtime="@$SOURCE_DATE_EPOCH" --format=gnu -cf - "$stem" \
    | gzip -n -9 >"$out/$archive"
fi

(cd "$out" && sha256 "$archive" >"$archive.sha256")
echo "$out/$archive"
