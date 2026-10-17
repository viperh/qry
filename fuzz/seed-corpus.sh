#!/usr/bin/env bash
# Fills fuzz/corpus/<target>/ with seeds: the shared test fixtures, every
# key binding of the default config, and hand-made edge cases. Idempotent:
# every seed has a fixed name (a fixture's own, or a hash of its content), so
# running it again rewrites the same files and leaves what the fuzzers added
# alone. Needs jq for the key bindings.
#
#   fuzz/seed-corpus.sh            # from anywhere in the repository
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixtures="$root/tests/fixtures"
corpus="$root/fuzz/corpus"

targets=(
  import_qry import_dbeaver import_sqldeveloper import_auto connections_parse
  keymap_parse palette_score sql_lex sql_split grid copy_render
)
for target in "${targets[@]}"; do
  mkdir -p "$corpus/$target"
done

# copy <file> <target>...: the file under its own name.
copy() {
  local file="$1"
  shift
  for target in "$@"; do
    cp "$file" "$corpus/$target/$(basename "$file")"
  done
}

# seed <target> <bytes>: printf-style bytes, named by their hash.
seed() {
  local target="$1" bytes="$2" name
  name="$(printf '%b' "$bytes" | sha1sum | cut -c1-16)"
  printf '%b' "$bytes" >"$corpus/$target/seed-$name"
}

# seed_text <target> <text>: the text as it is, named by its hash.
seed_text() {
  local target="$1" text="$2" name
  name="$(printf '%s' "$text" | sha1sum | cut -c1-16)"
  printf '%s' "$text" >"$corpus/$target/seed-$name"
}

# --- fixtures -------------------------------------------------------------
for file in "$fixtures"/dbeaver/*; do copy "$file" import_dbeaver import_auto; done
for file in "$fixtures"/sqldeveloper/*; do copy "$file" import_sqldeveloper import_auto; done
for file in "$fixtures"/qry-backup/*; do copy "$file" import_qry import_auto connections_parse; done
for file in "$fixtures"/sql/*; do copy "$file" sql_lex sql_split; done
# Inputs that once crashed a target are seeds for it too.
for dir in "$fixtures"/fuzz-regressions/*/; do
  target="$(basename "$dir")"
  [ -d "$corpus/$target" ] || continue
  for file in "$dir"*; do [ -f "$file" ] && copy "$file" "$target"; done
done

# --- key bindings from the default config ---------------------------------
# config.json is JSON5 with whole-line comments; dropping those leaves JSON.
sed -E 's#^[[:space:]]*//.*$##' "$root/.config/config.json" |
  jq -r '.keybindings[] | keys[]' | sort -u |
  while IFS= read -r binding; do
    seed_text keymap_parse "$binding"
  done

# --- hand-made seeds ------------------------------------------------------
# SQL: an unterminated string, a nested block comment, dollar quotes, `$$`
# alone, Oracle q-quotes, a PL/SQL block, MySQL DELIMITER.
for sql in \
  "select 'never closed" \
  "/* outer /* inner */ still a comment */ select 1;" \
  "create function f() returns int as \$body\$ select 1; \$body\$ language sql;" \
  "\$\$" \
  "select q'[it's; fine]', q'{x}', q'<y>' from dual;" \
  "begin\n  null;\nend;\n/\nselect 1 from dual;" \
  "DELIMITER //\ncreate procedure p() begin select 1; end//\nDELIMITER ;\nselect 2;" \
  "select \"quoted \"\"ident\"\"\", \`tick\`, [bracket] from t; -- trailing" \
  "select E'\\\\'' ; select 1e10, .5, 0x1F, x'00ff';"; do
  seed sql_lex "$sql"
  seed sql_split "$sql"
done

# JSON: deeply nested arrays, and each format's skeleton.
nested="$(printf '%.0s[' {1..200})$(printf '%.0s]' {1..200})"
for target in import_qry import_dbeaver import_sqldeveloper import_auto connections_parse; do
  seed "$target" "$nested"
  seed "$target" '{"connections": {"x": {"provider": "postgresql", "configuration": {"host": "h", "port": "99999"}}}}'
  seed "$target" '{"connections": [{"name": "n", "info": {"hostname": "h", "port": 1521, "sid": "XE"}}]}'
  seed "$target" '{"version": 1, "connections": [{"id": "a", "name": "\\u00e9", "driver": "sqlite", "secret": {"mode": "none"}}]}'
done

# Key bindings: odd ones.
for keys in '<ctrl-shift-alt-f12>' '<<>>' '<>' '<lt><gt>' 'ctrl--' 'alt-' '<g><g>' 'shift-Tab' 'f13' '←' 'ctrl-é'; do
  seed keymap_parse "$keys"
done

# Palette: query, NUL, text.
for pair in 'exp\0Export results' 'zz\0Export' '\0' 'İ\0i̇stanbul' 'a b\0a·b_c-d/e'; do
  seed palette_score "$pair"
done

# Grid and copy: column count, a kind per column, two choice bytes, then
# names and cells split by 0xFF (0xFE alone is NULL). See
# qry_core::fuzz::result_from_bytes.
for target in grid copy_render; do
  seed "$target" '\x00\x00\x00\x01n\xff3\xffNaN\xff1\xff\xfe\xff10\xff9'
  seed "$target" '\x01\x03\x00\x12\x05a\xffb\xffa2\xffx\xffa10\xff\xfe\xff\xfe\xffy'
  seed "$target" '\x03\x00\x01\x03\x0a\x07\x02select\xfft\xff"q"\xff`b`\xff1\xff\n\xff|\xff\t'
done

# Every target has at least one seed.
for target in "${targets[@]}"; do
  if [ -z "$(ls -A "$corpus/$target")" ]; then
    echo "seed-corpus.sh: no seeds for $target" >&2
    exit 1
  fi
done
echo "seeded ${#targets[@]} targets under $corpus"
