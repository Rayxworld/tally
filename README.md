# Tally

Safe, undoable folder organizer. Sorts loose files by type, never overwrites or deletes anything, and can undo.

## Build

1. Install Rust from https://rustup.rs (needs Rust 1.75 or newer).
2. In this folder run:

```
cargo build --release
```

The program is created at `target/release/tally` (`target\release\tally.exe` on Windows).

## Test

```
cargo test
```

## Use

Always preview first:

```
tally --sweep ~/Downloads --dry-run
```

If the plan looks right, run it for real:

```
tally --sweep ~/Downloads
```

Changed your mind:

```
tally --undo          # reverse the last sweep
tally --undo 3        # reverse the last 3 sweeps
tally --undo --dry-run
tally --history
```

Windows PowerShell example: `.\target\release\tally.exe --sweep $HOME\Downloads --dry-run`

## Configure (optional)

```
tally --init-config     # writes the default config and prints where it is
tally --show-config
```

Edit the file, then run a `--dry-run` to check your changes. See SPEC.md section 9 for every option.

Config and history live in your OS config folder under `tally/`. Set the `TALLY_HOME` environment variable to change that.

## Safety summary

- Only top-level files in the folder you name are touched. Sub-folders are never entered or moved.
- Hidden files, shortcuts, partial downloads, `.env`, and files changed in the last 10 seconds are skipped.
- Name collisions get `(1)`, `(2)` suffixes. Identical copies go to `_Duplicates/`, not the bin.
- Undo never overwrites; if a spot is taken it tells you and keeps the record so you can retry.

## Project layout

```
src/main.rs     command-line parsing and wiring
src/config.rs   config file, defaults, folder tokens
src/sweep.rs    routing, collision handling, moving
src/undo.rs     reversing sweeps
src/ledger.rs   history file (JSON Lines)
src/util.rs     hashing, safe move, names, timestamps
src/tests.rs    end-to-end tests
```
