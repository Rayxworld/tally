# Tally: Product Spec v0.1 (Draft)

Status: **Draft, implemented and tested on Linux. Not yet validated on Windows or macOS.**

## 1. Summary

Tally is a small offline command-line tool that sorts loose files in a folder (for example Downloads) into predictable folders by file type. It never overwrites or deletes a file, can preview every action, and can undo any sweep.

## 2. Principles

1. **Never lose data.** No overwrites, no deletes. Every move is recorded and reversible.
2. **Preview first.** Every action that changes files supports `--dry-run`.
3. **Zero setup.** Works with built-in defaults and no account or network access. A config file is optional.
4. **Small and fast.** Single native binary with minimal dependencies.

## 3. Non-goals (v0.1)

- Background watching, tray icon, GUI (planned v0.2/v0.3)
- Archive extraction (v0.3, opt-in only)
- Content-based or name-based grouping (needs a precise algorithm first)
- Recursing into sub-folders
- Cloud sync awareness, network drives

## 4. Commands

| Command | Behavior |
|---|---|
| `tally --sweep <DIR>` | Sort the **top-level files** of DIR. Sub-folders are never moved or entered. |
| `--dry-run` | With `--sweep` or `--undo`: print the plan, change nothing, write no history. |
| `--verbose` | Also list skipped files and the reason. |
| `tally --undo [N]` | Reverse the last N sweeps (default 1). |
| `tally --history` | List recorded sweeps, newest first. |
| `tally --init-config` | Write the default config file. Refuses to overwrite. |
| `tally --show-config` | Print the configuration in effect. |
| `--config-file <PATH>` | Use a specific config file. |

Exit codes: `0` success, `1` one or more files failed or a runtime error, `2` bad arguments.

## 5. Routing

1. A file is matched by its **lowercased final extension** against the config rules. First match wins; an extension may appear in only one rule (validated at load).
2. Files with no matching rule stay where they are.
3. Destinations use OS known folders (`{documents}`, `{pictures}`, `{videos}`, `{music}`, `{downloads}`, `{desktop}`, `{home}`) so redirected folders such as OneDrive work. `~` is also accepted.
4. Destination folders are created as needed and recorded so undo can remove them if they end up empty.

## 6. Safety rules

**Always skipped:**

- Folders (so `.git`, `node_modules`, `target`, project folders are never touched)
- Symbolic links
- Hidden files (names starting with `.`)
- Names in `skip_names` (default: `.env`, `config.local`, `desktop.ini`, `.DS_Store`, `Thumbs.db`)
- Extensions in `skip_extensions` (default: `.crdownload .part .partial .opdownload .download .tmp .lnk .url .ini`)
- Files with no extension, or names that are not valid UTF-8
- Files modified less than `min_age_seconds` ago (default 10), which protects in-progress downloads
- Files already inside their destination folder

## 7. Collisions

When the destination name is taken:

1. Compute SHA-256 of both files.
2. **Identical content:** move the new file to `<destination>/_Duplicates/` (numbered if needed). Nothing is deleted.
3. **Different content:** use the next free name: `Document (1).pdf`, `Document (2).pdf`. Compound extensions are respected (`archive (1).tar.gz`).
4. A folder with the same name is never touched; the next free file name is used.

Moves use a plain rename. If that fails (for example across drives), Tally copies, verifies by hash, and only then removes the original. On any failure the original is left in place.

## 8. History and undo

- Each sweep that moves at least one file appends one JSON line to `transactions.jsonl` in the Tally home folder.
- Home folder: `TALLY_HOME` if set, otherwise the OS config folder plus `tally/`.
- Undo walks operations in reverse and moves each file back. It **never overwrites**: if something now occupies the original path, that file is left, the problem is reported, and the transaction is kept so undo can be retried.
- Files that are no longer at their destination are reported and skipped.
- Folders Tally created are removed only if empty.

Transaction format:

```json
{
  "transaction_id": "tx_20261022_131800_042",
  "timestamp": "2026-10-22T13:18:00Z",
  "target": "/home/user/Downloads",
  "operations": [
    {"source_path": "/home/user/Downloads/a.pdf",
     "destination_path": "/home/user/Documents/a.pdf",
     "action": "moved"}
  ],
  "created_dirs": []
}
```

## 9. Configuration (`tally.config.json`)

All fields optional; missing fields use defaults. Unknown fields are rejected so typos are caught.

```json
{
  "version": 1,
  "min_age_seconds": 10,
  "duplicates_folder": "_Duplicates",
  "skip_extensions": [".crdownload", ".part", ".tmp", ".lnk"],
  "skip_names": [".env", "desktop.ini"],
  "rules": [
    { "name": "Documents", "destination": "{documents}", "extensions": [".pdf", ".docx"] },
    { "name": "Installers", "destination": "{documents}/Installers", "extensions": [".exe", ".msi"] }
  ]
}
```

Note: if you set `rules`, `skip_extensions` or `skip_names`, your list **replaces** the default list rather than adding to it. Run `tally --init-config` to start from the full defaults.

## 10. Success criteria

- Sweeping 1,000 files completes in under 1 second on an SSD (excluding hashing on collisions). *Not yet benchmarked.*
- After any sweep, `--undo` restores every file to its original path when nothing else has changed.
- No test or run can cause a file's content to be lost.

## 11. Roadmap

- **v0.2:** `--watch` using the `notify` crate with `notify-debouncer-full`, ignoring partial-download files and waiting for stable file size; `--stats`; start-on-login.
- **v0.3:** tray/menu-bar app (separate binary from the CLI on Windows), opt-in archive extraction with zip-slip and zip-bomb protection, precisely specified name grouping.

## 12. Open questions

- Final product name (the name "Tally" is shared with well-known accounting software; check trademarks).
- Whether to ship the default rules that send `.json` and `.yml` to Data, or leave config-like files alone.
- Windows testing, including OneDrive-redirected folders and locked files.
