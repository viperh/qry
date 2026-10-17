# Key bindings

<!-- Generated from the built-in keymap by `crates/qry/src/docs.rs`; run `QRY_BLESS=1 cargo test -p qry docs` after changing a binding. -->

These are the defaults, as `.config/config.json` sets them. A user config can rebind or unbind any of them; see [config.md](config.md). The in-app help (`?` or F1) lists the keys in effect.

## Global

| Keys | Action |
| --- | --- |
| `ctrl-q` | quit |
| `ctrl-z` | suspend to the shell |
| `F1` | show / hide the help |
| `ctrl-p` | command palette |
| `Tab` | next pane |
| `shift-Tab` | previous pane |
| `ctrl-n` | new connection |
| `ctrl-o` | import connections |
| `ctrl-/` | show / hide the key hints |
| `ctrl-↑` | grow the editor |
| `ctrl-↓` | shrink the editor |
| `F11` | maximise the pane / restore |
| `ctrl-l` | show the messages |
| `Esc` | dismiss the message |

## Tree

| Keys | Action |
| --- | --- |
| `?` | show / hide the help |
| `:` | command palette |
| `n` | new connection |
| `e` | edit this connection |
| `D` `Del` | forget this connection |
| `c` | duplicate this connection |
| `k` `↑` | up |
| `j` `↓` | down |
| `h` `←` | left |
| `l` `→` | right |
| `PgUp` | up a page |
| `PgDn` | down a page |
| `g` `g` `Home` | to the top |
| `G` `End` | to the bottom |
| `Space` | expand / collapse |
| `Enter` | connect / switch to its session |
| `shift-Enter` | open another session on it |
| `r` | refresh the node |
| `R` | refresh the whole connection |
| `/` | filter the tree |
| `.` | show / hide system objects |
| `s` | insert SELECT * FROM the table |
| `y` | copy the qualified name |
| `d` | describe the table |
| `x` | disconnect its session |
| `J` | show / hide the jobs |
| `Esc` | clear the filter |

## Editor · Editing

| Keys | Action |
| --- | --- |
| `↑` | up |
| `↓` | down |
| `←` | left |
| `→` | right |
| `PgUp` | up a page |
| `PgDn` | down a page |
| `ctrl-Home` | to the top |
| `ctrl-End` | to the bottom |
| `Home` | start of the line |
| `End` | end of the line |
| `Enter` | new line |
| `Tab` | indent |
| `ctrl-←` | previous word |
| `ctrl-→` | next word |
| `shift-←` | select left |
| `shift-→` | select right |
| `shift-↑` | select up |
| `shift-↓` | select down |
| `ctrl-shift-←` | select the previous word |
| `ctrl-shift-→` | select the next word |
| `shift-Home` | select to the start of the line |
| `shift-End` | select to the end of the line |
| `ctrl-a` | select all |
| `Backspace` | delete the character before |
| `Del` | delete the character after |
| `ctrl-w` | delete the word before |
| `alt-d` | delete the word after |
| `ctrl-k` | delete to the end of the line |
| `ctrl-z` | undo |
| `ctrl-y` `ctrl-shift-z` | redo |
| `ctrl-x` | cut |
| `ctrl-v` | paste |

## Editor · Running

| Keys | Action |
| --- | --- |
| `F5` `ctrl-Enter` | run the statement |
| `F8` | run everything |
| `ctrl-e` | run the selection |
| `ctrl-c` | cancel the query (copy when none runs) |

## Editor · Other

| Keys | Action |
| --- | --- |
| `F6` | next pane |
| `ctrl-r` | query history |
| `alt-↑` | previous statement from the history |
| `alt-↓` | next statement from the history |
| `ctrl-s` | save the statement as a snippet |
| `shift-Tab` | dedent |

## Results · Navigation

| Keys | Action |
| --- | --- |
| `k` `↑` | up |
| `j` `↓` | down |
| `h` `←` | left |
| `l` `→` | right |
| `PgUp` `ctrl-u` | up a page |
| `PgDn` `ctrl-d` | down a page |
| `g` `g` | to the top |
| `G` | to the bottom |
| `0` `Home` | start of the line |
| `$` `End` | end of the line |
| `[` | previous result |
| `]` | next result |

## Results · Copy & search

| Keys | Action |
| --- | --- |
| `ctrl-e` | export the result to a file |
| `/` | search the results |
| `y` | copy the cell |
| `Y` | copy the row |
| `n` | next match |
| `N` | previous match |
| `ctrl-y` | copy as… |
| `Enter` | inspect the value |
| `i` | show / hide the inspector |

## Results · Other

| Keys | Action |
| --- | --- |
| `?` | show / hide the help |
| `:` | command palette |
| `ctrl-c` | cancel the query (copy when none runs) |
| `d` | describe the table |
| `s` | sort by the column |
| `S` | add the column to the sort |
| `ctrl-g` | go to a row |
| `<` | narrow the column |
| `>` | widen the column |
| `=` | reset the column widths |
| `F` | freeze the first column |
| `L` | load the next rows |
| `A` | fetch every row |
| `J` | show / hide the jobs |

## Inspector

| Keys | Action |
| --- | --- |
| `?` | show / hide the help |
| `:` | command palette |
| `Esc` | go to the results |
| `k` `↑` | up |
| `j` `↓` | down |
| `h` `←` | left |
| `l` `→` | right |
| `g` `g` | to the top |
| `G` | to the bottom |
| `y` | copy the cell |
| `Enter` | inspect the value |
| `i` | show / hide the inspector |

In the value popup: y copies the whole value, w wraps, f formats JSON, h/l pan, q closes

Describing a table (d in the tree or the results): j/k scroll, Esc goes back

## Sessions

| Keys | Action |
| --- | --- |
| `ctrl-1–9` | switch to session N |
| `ctrl-PgDn` | next session |
| `ctrl-PgUp` | previous session |
| `ctrl-w` | close this session |

shift-Enter in the tree opens another session on a connection

## Dialogs

| Keys | Action |
| --- | --- |
| `ctrl-q` | quit |
| `F1` | show / hide the help |
| `↑` | up |
| `↓` | down |
| `←` | left |
| `→` | right |
| `PgUp` | up a page |
| `PgDn` | down a page |
| `Home` | start of the line |
| `End` | end of the line |
| `Esc` | cancel / close |
| `Enter` | submit |
| `ctrl-t` | test the connection |
| `Tab` | next field |
| `shift-Tab` | previous field |
| `Space` | next choice |
| `Backspace` | delete the character before |
| `Del` | delete the character after |

## Command palette

| Keys | Action |
| --- | --- |
| `ctrl-q` | quit |
| `F1` | show / hide the help |
| `ctrl-p` | command palette |
| `ctrl-Enter` | insert and run |
| `ctrl-d` | delete the entry |
| `↑` `ctrl-k` | up |
| `↓` `ctrl-j` | down |
| `←` | left |
| `→` | right |
| `PgUp` | up a page |
| `PgDn` | down a page |
| `Home` | start of the line |
| `End` | end of the line |
| `Esc` | cancel / close |
| `Enter` | submit |
| `Tab` | next field |
| `Backspace` | delete the character before |
| `Del` | delete the character after |

## Jobs

| Keys | Action |
| --- | --- |
| `?` | show / hide the help |
| `:` | command palette |
| `c` | cancel the query (copy when none runs) |
| `k` `↑` | up |
| `j` `↓` | down |
| `h` `←` | left |
| `l` `→` | right |
| `g` `g` | to the top |
| `G` | to the bottom |
| `Enter` | open |
| `J` `Esc` | show / hide the jobs |
