# Caret plugins

Caret plugins are TOML manifests that can add commands, languages, themes, and
save hooks without recompiling the editor. Put manifests directly in the
directory shown by `:plugindir`, then run `:pluginreload`.

Only install plugins you trust. Plugin commands are local programs and inherit
your user permissions.

## Manifest

```toml
name = "Example tools"
version = "0.1.0"

[[commands]]
name = "uppercase"
description = "Uppercase the selection"
program = "powershell.exe"
args = ["-NoProfile", "-File", "uppercase.ps1"]
timeout_ms = 15000

[[languages]]
name = "Caret Notes"
extensions = ["note"]
line_comment = "#"

[[themes]]
name = "midnight"
base = "nord"

[themes.colors]
background = "#080b12"
keyword = "#ff79c6"

[hooks]
on_save = []
```

Relative command paths are resolved from the manifest directory. Programs on
`PATH` can be named directly. Commands time out after 15 seconds by default;
set `timeout_ms` per command when needed. Each command receives one JSON object on standard
input:

```json
{
  "project": "C:\\code\\project",
  "file": "C:\\code\\project\\main.cs",
  "language": "C#",
  "text": "entire buffer",
  "selection": "selected text or null",
  "cursor_line": 4,
  "cursor_column": 12,
  "arguments": ["values", "from", "the", "command"],
  "event": "command"
}
```

The command must write either no output or one JSON response to standard
output. Supported response properties are:

- `message`: show a status message.
- `replace_document`: replace the active buffer.
- `replace_selection`: replace the selection.
- `insert_text`: replace the selection, if any, or insert at the cursor.
- `open`: open a project-relative or absolute file path.

For example:

```json
{"replace_selection":"HELLO","message":"Uppercased selection"}
```

Run commands with `:plugin name [arguments]`. Loaded plugin commands also
appear in the filtered command palette. `:plugins` lists loaded plugins and
commands.

## Languages

A language entry supplies a display name and line-comment prefix for matching
file extensions. It participates in the status bar and `:comment` command.

## Themes

A plugin theme starts with one built-in `base` (`oxide`, `nord`, `dracula`,
`solarized`, or `mono`) and overrides any named colors. Hex RGB values are
supported. Apply it with `:theme name`; Caret remembers it across launches.

## Save hooks

Put command names in `hooks.on_save`. Caret runs them after saving the active
file, applies their response, and saves any resulting buffer edits once more.
The event passed to the plugin is `on_save`.
