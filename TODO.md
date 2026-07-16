# Caret Roadmap

## Milestone 0.6 — Trustworthy Foundation (in progress)

- [x] Atomic same-directory document and configuration saves with durable temporary files and permission preservation
- [x] UTF-8 BOM, LF/CRLF, final-newline, binary, and unsupported-encoding handling
- [x] Periodic crash-recovery journal in the platform application-data directory
- [x] Recovery commands: `:recover` and `:discardrecovery`
- [x] Structured panic log, terminal restoration, `caret doctor`, and safe in-editor diagnostic-copy commands
- [x] Headless SSH clipboard fallback through OSC 52 terminal clipboard transfer
- [x] Session restoration for projects, tabs, active tab, cursor/scroll, split layout, and sidebar state (terminals are intentionally excluded)
- [x] External-change reload, overwrite, and compare choices
- [x] Recovery discovery with filename/timestamp and recover/compare/discard commands
- [x] Windows, macOS, and Linux CI build/test/diagnostic smoke matrix
- [x] Ubuntu install and pseudo-terminal startup smoke test
- [ ] Run the documented forced-termination smoke tests on Windows, macOS, and Linux before release (`docs/SMOKE_TEST_0.6.md`)


## Milestone 0.7 — Excellent Everyday Editing (in progress)

- [x] Configurable startup view (`startup = session | folder | empty | dashboard`); the default now opens the current folder's file tree instead of always showing the welcome dashboard, which is reachable on demand via `:welcome`.
- [x] Sidebar visibility follows the launch target: `caret <folder>` opens with the file tree shown, `caret <file>` opens with it hidden (`:tree` toggles).
- [x] Operation-based undo/redo replacing whole-document snapshots: grouped typing, selection and multi-cursor restoration, clear boundaries, configurable depth (`:set undolimit=N`)
- [x] Editing fundamentals: auto-indent on Enter and `o`/`O` (`:set autoindent`), smart backspace, paired-delimiter/comment/line operations (pre-existing) plus `:trim`, `:splitline`, `:set trimonsave`, and `:set finalnewline=preserve|always|strip`
- [x] Multi-cursor improvements: backspace/delete at every cursor, add cursor above/below (Ctrl-Alt-Up/Down), column selection (Alt-Shift-Arrows), select all occurrences (Ctrl-Shift-L / `:selectoccurrences`)
- [x] Complete find-and-replace panel (Ctrl-F / Ctrl-H): case, whole-word, and regex toggles; replace one (Alt-Enter) and all (Alt-A); search within selection; search history (Up/Down); live match counter; shared compiled search engine (`src/search.rs`)
- [x] Project-wide search (Ctrl-Shift-F / `:grep`): gitignore-aware walker, results panel with keyboard and mouse navigation, and replacement with preview — per-match exclusion (Del), double-Alt-A confirmation, atomic file rewrites, dirty open tabs skipped
- [x] Fuzzy file opener (Ctrl-P / `:files`) with in-house scorer, matched-character highlighting, and recently opened files (persisted, ranked first)
- [x] File tree: filtering (`/` or `f`), root `.gitignore` support, symlink markers with cycle-safe expansion, delete confirmation showing the full path
- [x] User-editable key bindings (`:bind`, `:unbind`, `:bindreset`, `[custom_keys]` in config) with conflict detection, terminal-interception warnings, macOS modifier-symbol display, and a searchable `:keybindings` browser including per-profile fixed keys
- [x] Case-sensitivity filesystem tests: recased paths reuse the tab on Windows and open as distinct files on Linux
- [ ] Searchable settings interface: browse/search all settings with descriptions, defaults, validation, and restart-required indicators (today `:set` covers a subset with basic validation)
- [ ] Refresh the in-app help pages (F1) and README/docs for the new 0.7 features
- [ ] Run the milestone 0.7 exit-criteria smoke test on Windows, macOS, and Linux

## Known bugs

- C# `:def` returns no definition in Caret even though the same `csharp-ls`
  request succeeds against the loaded solution in an isolated protocol test.
