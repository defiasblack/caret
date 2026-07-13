# Caret

![Caret terminal code editor](assets/caret-logo/caret-wordmark.svg)

**Caret** is a polished terminal text editor written in Rust. It combines a
friendly typing-first workflow with Vim-inspired navigation, a project folder
tree, searchable commands, syntax coloring, mouse-wheel scrolling, and
undo/redo.

## Highlights

- Wider, expandable project/folder tree with recursive controls
- Back/forward file and cursor-location history
- Open a file or an entire directory from the command line
- `Ctrl-E` switches between the editor and file tree
- `Ctrl-B` shows or hides the file tree
- Rope-based text storage for efficient editing of large files
- Normal and Insert modes
- Tree-sitter parsing and syntax coloring for Rust, Go, C#, YAML, JSON, TOML, Python, and shell files
- Incremental search with match highlighting
- Undo and redo
- Grouped undo for consecutive typing and deletion, split by navigation
- Line numbers and horizontal/vertical scrolling
- Context-sensitive hotkey strip
- Mouse-wheel scrolling
- Commands for save, save-as, open, quit, goto-line, themes, and settings
- Recent-project welcome dashboard, right-click context menus, and Git gutter markers
- Caret, Vim, and conventional keymap profiles
- Persistent LSP/background status with reduced-motion support
- Built-in persistent shell pane with live output and command history

## Build and install

```bash
cargo build --release
chmod +x install.sh
./install.sh
```

Open a project folder:

```bash
caret ~/Documents/my-project
```

Open a specific file:

```bash
caret src/main.rs
```

## Mouse support

Caret enables terminal mouse capture automatically.

- Click a file to open it in a tab.
- Click a folder to expand or collapse it.
- Click inside the editor to place the cursor and enter Insert mode.
- Click a tab in the tab bar to switch to it.
- Click `F1 Help` in the upper-right corner to open the help panel.
- Use the mouse wheel over the file tree or editor to scroll that pane.

Mouse support depends on the terminal forwarding mouse events. Windows Terminal,
GNOME Terminal, Konsole, and most modern terminal emulators support this.

See [Accessibility and terminal compatibility](docs/ACCESSIBILITY.md) for the
keyboard-only workflow, monochrome/reduced-motion options, and release matrix.

## Easy quit confirmation

Press `Ctrl-Q` normally. When there are unsaved tabs, Caret now displays:

```text
Unsaved changes — [S] Save all & quit   [D] Discard & quit   [Esc] Cancel
```

You no longer need to remember `:q!` just to discard changes and exit.

- `S` saves all named modified tabs and quits.
- `D` discards all unsaved changes and quits.
- `Esc` or `C` cancels and returns to the editor.
- If an untitled tab cannot be saved because it has no filename, Caret stays open
  and shows the save error instead of losing the tab.

## Navigation history

| Key | Action |
|---|---|
| `Alt-Left` | Go back to the previous file/cursor location |
| `Alt-Right` | Go forward to the next location |
| `:back` | Go back |
| `:forward` | Go forward |

History entries remember the active file, tab, cursor line and column, and scroll
position. Opening files, switching tabs, clicking elsewhere in the document, and
jumping to a line create navigation history entries.

## Tab keys

| Key | Action |
|---|---|
| `Alt-N` | Next Caret tab |
| `Alt-P` | Previous Caret tab |
| `Ctrl-PageDown` / `Ctrl-PageUp` | Next / previous tab |
| `Ctrl-T` | New tab |
| `Ctrl-W` | Close current tab |
| `Alt-1` through `Alt-9` | Select a tab directly |
| `:tn` / `:tp` | Next / previous tab by command |

`Alt-Left` and `Alt-Right` are reserved for navigation history. This also avoids
the common problem where terminals intercept `Ctrl-Tab`.

## Project tree keys

The sidebar now defaults to 40 columns and can be resized from 22 to 80 columns.

| Key | Action |
|---|---|
| `Ctrl-E` | Switch focus between editor and files |
| `Ctrl-B` | Show or hide the tree |
| Click `FILES` in the title bar | Show or hide the tree |
| `Up` / `Down` or `k` / `j` | Move selection |
| `Enter` | Open a file or toggle a directory |
| `Left` / `Right` or `h` / `l` | Collapse or expand |
| `Shift-Right` | Expand the selected folder and its immediate child folders |
| `Shift-Left` | Recursively collapse the selected folder |
| `*` | Expand all folders, with a 5,000-directory safety limit |
| `-` | Collapse all folders |
| `Backspace` | Collapse or jump to the parent folder |
| `Home` | Jump to the top of the project tree |
| `.` | Show or hide dotfiles |
| `r` | Refresh the tree |
| `Esc` | Return focus to the editor |

The folder tree uses wider indentation, connector guides, explicit `DIR`
markers, and trailing `/` characters so nested folder structure is easier to
read.

## Editor keys

### Normal mode

| Key | Action |
|---|---|
| `i` | Insert before cursor |
| `a` | Insert after cursor |
| `o` / `O` | Open line below / above |
| Arrow keys or `h j k l` | Move |
| `w` / `b` | Next / previous word |
| `0` / `$` | Start / end of line |
| `gg` / `G` | Top / bottom of file |
| `x` | Delete character |
| `dd` | Delete line |
| `D` | Duplicate current line |
| `Alt-Up` / `Alt-Down` | Move current line up / down |
| `Ctrl-J` | Join current line with the next line |
| `Tab` / `Shift-Tab` | Indent / outdent the current line or selection |
| `Ctrl-/` | Toggle comments using the active file's language |
| `yy` | Yank line |
| `p` | Paste yanked line below |
| `q` + register | Start recording a macro (press `q` in Normal mode to stop) |
| `@` + register | Replay a recorded macro |
| `u` | Undo |
| `Ctrl-r` | Redo |
| `/` | Search |
| `n` / `N` | Next / previous result |
| `%` | Jump to the matching bracket |
| `:` | Command prompt |
| `F1` | Help |

### Insert mode

| Key | Action |
|---|---|
| `Esc` | Return to Normal mode |
| `Ctrl-s` | Save |
| `Ctrl-q` | Quit |
| `Tab` | Insert spaces using current tab width |
| `F7` | Duplicate current line and remain in Insert mode |
| `(`, `[`, `{`, `'`, `"` | Insert a matching pair; typing its closer skips over it |
| `Backspace` | Remove an empty matching pair together |
| `Ctrl` + Left / Right | Move to previous / next word |
| `Ctrl` + `Shift` + Left / Right | Select previous / next word |
| Double-click | Select the clicked word |
| `Shift` + arrows / Home / End | Select text with the keyboard |
| Drag with left mouse button | Select text with the mouse |
| `Ctrl-C` / `Ctrl-X` / `Ctrl-V` | Copy / cut / paste selected text |
| `Ctrl-D` | Select the next occurrence of the current word or selection |
| `Delete` / `Backspace` | Delete selected text |

## Commands

| Command | Action |
|---|---|
| `:w` | Save |
| `:w file.txt` | Save as, relative to project root |
| `:q` / `:q!` | Quit / force quit |
| `:wq` or `:x` | Save and quit |
| `:e path` | Open a file or folder |
| `:e! path` | Open and discard unsaved changes |
| `:cd path` | Change project root |
| `:tree` | Toggle project tree |
| `:refresh` | Refresh project tree |
| `:pwd` | Display project root |
| `:back` / `:forward` | Navigate location history |
| `:expandall` / `:collapseall` | Expand or collapse the entire tree |
| `:treewidth 44` | Set sidebar width directly |
| `:reveal` | Reveal the active file in the project tree |
| `:new [file]` | Create a new buffer |
| `:duplicate`, `:moveup`, `:movedown`, `:join` | Line operations |
| `:sort` | Sort current line or selected lines |
| `:indent` / `:outdent` | Indent or outdent the current line or selection |
| `:comment` | Toggle comments using the active file's language |
| `:terminal` | Open or focus the integrated shell pane |
| `:terminalclose` | Close the integrated shell pane |
| `:42` | Jump to line 42 |
| `:set treewidth=40` | Change sidebar width |
| `:set number` / `:set nonumber` | Toggle line numbers |
| `:set tabstop=4` | Change tab width |
| `:theme oxide` / `:theme mono` | Change theme |
| `:doctor` / `:copydiagnostics` | View or copy the support diagnostic report |
| `:recover N` / `:recovercompare N` / `:discardrecovery` | Restore, compare, or discard crash-recovery snapshots |

## Coding intelligence

Run `:lsp` in a saved C# or Rust file, then use the keyboard or matching
commands:

| Key / command | Action |
|---|---|
| `Ctrl-Space` / `:complete` | Filter and insert completions |
| `:hover` | Show type and documentation |
| `F12` / `:definition` | Go to definition |
| `Shift-F12` / `:references` | Browse references |
| `F2` / `:symbolrename name` | Rename a symbol across its workspace |
| `Ctrl-.` / `:actions` | Browse and apply code actions |
| `:diagnostics` | Browse errors and warnings |
| `:format` | Format the document |

Result panels support arrows, Page Up/Down, mouse hover/click, Enter, and Esc.

## Integrated terminal

Press `Ctrl-Backtick` or run `:terminal` to open a persistent shell below the
editor. The PTY shell inherits the project folder and remains usable in SSH
sessions without a desktop GUI.

| Key | Action |
|---|---|
| `Ctrl-Backtick` | Switch between editor and terminal |
| `Ctrl-Shift-Backtick` | Close the terminal pane |
| `Up` / `Down` | Browse command history |
| `PageUp` / `PageDown` | Scroll terminal output |
| `Ctrl-L` | Clear terminal output |
| `Ctrl-C` | Clear the current terminal input |

The pane is intended for shell commands, builds, tests, Git, and scripts.
The pane uses a real PTY/ConPTY, so interactive shells, terminal colors, and
full-screen terminal programs work inside Caret. Set `CARET_SHELL` to override
the default shell.

When no desktop clipboard is available—for example in a headless SSH session—
Caret copies through the OSC 52 terminal clipboard protocol. The terminal
emulator must permit OSC 52 for copied text to reach the local clipboard.

## Plugins

Run `:plugindir` to find the plugin directory, add a TOML manifest, and run
`:pluginreload`. Plugins can contribute commands, language comment rules,
themes, and save hooks. See [Caret plugins](docs/PLUGINS.md) and the
[sample plugin](examples/plugins/sample.toml).

## Reliability verification

The release smoke checklist for atomic saves, recovery, external changes, and
session restoration is in [Milestone 0.6 smoke tests](docs/SMOKE_TEST_0.6.md).
