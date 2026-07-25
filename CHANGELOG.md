# Changelog

All notable changes to Caret will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project intends to follow [Semantic Versioning](https://semver.org/) as the public release process matures.

## Unreleased

### Added

- Complete find-and-replace panel (Ctrl-F to find, Ctrl-H or :replace to
  replace): case-sensitivity, whole-word, and regex toggles, replace
  one/all, search within a multi-line selection, recallable search
  history, and a live match counter
- Project-wide search and replace (Ctrl-Shift-F, :projectsearch, :grep):
  gitignore-aware, binary-safe, with a reviewable preview where single
  matches can be excluded before a confirmed, atomic apply; files with
  unsaved changes are skipped and reported
- Fuzzy file opener (Ctrl-P, :files) over every non-ignored project
  file, with matched-character highlighting and recently opened files
  ranked first; the last 30 opened files persist across sessions
- File tree filtering (press / or f in the tree), root .gitignore
  support in the tree (toggled with hidden files via .), symlink
  markers with cycle-safe expansion, and delete confirmations that name
  the exact path
- User-editable key bindings: :bind / :unbind / :bindreset with
  conflict detection, startup validation of the config's [custom_keys]
  table, warnings for chords terminals commonly intercept, macOS
  modifier symbols in key displays, and a searchable :keybindings
  browser that also lists the active profile's fixed keys
- Editing fundamentals: auto-indent on Enter and o/O (:set autoindent),
  smart backspace through leading spaces, multi-cursor backspace and
  delete, add cursor above/below (Ctrl-Alt-Up/Down), column selection
  (Alt-Shift-Arrows), select all occurrences (Ctrl-Shift-L), :trim,
  :splitline, :set trimonsave, and :set finalnewline=preserve|always|strip
- Community health files and contribution guidance
- Cross-platform continuous integration
- Automated dependency update configuration

### Changed

- Undo and redo are operation-based instead of whole-document
  snapshots: typing runs coalesce into single steps, selections and
  multi-cursors are restored on undo, empty steps are dropped, and the
  history depth is configurable (:set undolimit=N, default 1000)
- Editor settings (line numbers, tab width, indent and save cleanups,
  undo depth) now apply to every open tab and propagate to new tabs
- Settings are never written by the test suite, protecting the user's
  real configuration file during development
- **Breaking:** the Caret profile is no longer modal. Previously Esc left
  Insert mode and Normal mode ran Vim's bare-letter commands, so typing an
  ordinary word could run motions and then hit `o`, `i`, or `a` -- which
  opened a line, switched to Insert, and silently typed the rest of the
  word into the document. Esc now clears the selection and stays in Insert,
  and printable keys always insert text. Normal mode is a Vim-profile
  concept only; run :keymap vim for the previous behaviour
- Folds and macros are rebindable actions reachable without Normal mode:
  F9 toggles a fold, Shift-F9 folds all, Ctrl-F9 unfolds all, F4 starts
  and stops macro recording, Shift-F4 replays. The Vim profile keeps
  zc/zo/za/zM/zR and q/@ as well
- The keybinding browser and help no longer advertise Vim commands
  (dd, yy, hjkl) to the non-modal profiles, and describe Esc and Tab
  according to the active profile
- The command palette is a centred modal: a search field at the top, the
  matching commands listed with a plain-English description, and the chord
  that runs the same thing right-aligned beside each one. It filters on
  descriptions as well as names, so "quit" finds :q, with name matches
  always ranked first. Replaces the unlabelled 30-column strip of command
  names that used to sit above the status bar
- A clickable [Command] control in the title bar, beside [F1 Help],
  opens the command palette -- previously the `:` prompt was only
  reachable from Normal mode, which the non-modal profiles never enter.
  The hotkey row for those profiles now shows Ctrl-Shift-P Command
  instead of an Esc that no longer returns to Normal mode
- Title-bar hit zones are derived from the same table that draws them,
  and are dropped on terminals too narrow to render the controls where
  the offsets claim

## 0.5.0

### Added

- Expandable project tree with recursive controls
- File and cursor-location navigation history
- Tabs, syntax highlighting, search, undo, redo, and selections
- Caret, Vim, and conventional keymap profiles
- Language-server workflows for Rust and C#
- Persistent integrated PTY/ConPTY terminal
- Plugin manifests for commands, language rules, themes, and save hooks
