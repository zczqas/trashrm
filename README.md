# trashrm

A tiny, dependency-free `rm` alternative for Unix. It moves files to the
freedesktop.org Trash (`~/.local/share/Trash`) instead of deleting them, so
`del -r testing` is never a one-way mistake.

The installed binary is called `del`, so it never shadows the real `rm`.

## Install

```sh
cargo install trashrm
```

From a checkout:

```sh
git clone https://github.com/zczqas/trashrm
cd trashrm
cargo install --path .
```

## Usage

```sh
del file.txt              # move a file to the trash
del -r testing            # move a directory
del -rf build/ old.log    # -f ignores missing files
del -v notes.md           # print what happened
del --list                # show trashed items (oldest first)
del --undo                # restore the most recently trashed item
```

Opt in to the safer behaviour in your interactive shell only:

```sh
alias rm='del'            # scripts and cron still get the real /bin/rm
```

Need a real delete? Use `command rm -rf thing` or `\rm -rf thing`.

## Options

| Option | Meaning |
| --- | --- |
| `-r`, `-R`, `-d` | Allow directories (all three are treated as recursive) |
| `-f` | Ignore missing files; `del -f` with no operands succeeds |
| `-v` | Print a `trashed '...' -> '...'` line for each item |
| `-i`, `-I` | Accepted for `rm` muscle memory, but ignored |
| `--recursive`, `--force`, `--verbose` | Long forms of the above |
| `--list` | Print deletion date and original path for every trashed item |
| `--undo` | Restore the most recently trashed item |
| `--` | Stop option parsing; treat the rest as file names |
| `--help`, `--version` | Print usage or version and exit |

## Trash location

- **Linux:** `$XDG_DATA_HOME/Trash` if `XDG_DATA_HOME` is set, otherwise
  `~/.local/share/Trash`. Items are stored in `files/` with a matching
  `info/<name>.trashinfo` record, following the
  [freedesktop Trash spec](https://specifications.freedesktop.org/trash-spec/latest/).
  This means your file manager's Trash and "Restore" work normally.
- **macOS:** `~/.Trash`. No `.trashinfo` files are written, so `--list` and
  `--undo` are not supported there.

Only the home trash is used, not per-drive `.Trash-$uid` folders.

## Behaviour notes

- Original paths in `.trashinfo` files are percent-encoded, and deletion dates
  are written in UTC.
- Name collisions are resolved by appending `.2`, `.3`, and so on
  (`report.pdf` becomes `report.pdf.2` if `report.pdf` is already trashed).
- Symlinks are trashed as links; their targets are never followed.
- Moving across filesystems falls back to copy + delete.
- `--list` prints entries oldest first. `--undo` restores the newest entry and
  refuses to overwrite a file that already exists at the original path.
- Deleting a directory without `-r` fails with `Is a directory (use -r)`.
- Paths with no file name (`.`, `..`, `/`) are refused.
- Exit codes: `0` on success, `1` if any item could not be trashed, `2` for
  usage errors such as an unknown option.

## Development

```sh
cargo build --release     # binary at target/release/del
cargo test                # unit tests for date/path encoding
```

## License

MIT
