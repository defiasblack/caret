# Milestone 0.7 manual smoke test

Run this checklist from a disposable project on Windows, macOS, and Linux
before a 0.7 release. Record the terminal emulator, OS version, commit, and
result for each platform.

1. Launch `caret` with a folder and with a file. Confirm folder launches show
   the tree, file launches focus the editor, and `:tree` / `Ctrl-B` toggle it.
2. Type several lines, navigate between them, undo and redo grouped edits, and
   confirm undo restores both text and selection state.
3. Test auto-indent with Enter, `o`, and `O`; smart Backspace; paired delimiters;
   comments; trailing-whitespace trimming; and final-newline policies.
4. Add cursors above and below, make a column selection, select repeated
   occurrences, and edit or delete with all cursors active.
5. Use `Ctrl-F` and `Ctrl-H` with case, whole-word, regex, selection scope,
   history, replace-one, and replace-all. Confirm `Ctrl-Shift-F` / `:grep`
   searches the project and supports excluded-match replacement previews.
6. Open a Git project with modified, added, deleted, and untracked files. Confirm
   the tree appears immediately, the header briefly shows `git…`, status badges
   update without blocking typing, collapsed folders summarize descendant
   changes, `:gitrefresh` queues another refresh, and a non-Git folder remains
   usable. Expand nested folders and verify `├─`, `└─`, and `│` guides terminate
   correctly.
7. Open `Ctrl-P` / `:files`, filter by a partial path, open a result, and verify
   recently opened files rank ahead of ordinary matches.
8. Open `:settings`, search for `undo`, inspect a row, and verify each row shows
   current value, default, validation, and live/next-launch scope. Change a
   setting through `:set` and confirm the value updates in the browser.
9. Open `:themes`; navigate with arrows, mouse hover, and the mouse wheel.
   Apply a theme, cancel a preview, and confirm the large theme catalog remains
   scrollable in a normal terminal window.
10. Open `:keybindings`, search for an action, rebind it with `:bind`, verify
   conflict warnings, then restore it with `:unbind` or `:bindreset`.
11. Open the dashboard, project search, context menus, terminal pane, LSP
    panels, and Help. Confirm every panel closes with Esc and that the F1 pages
    describe the current 0.7 workflows.
12. Quit with dirty tabs and verify save/discard/cancel behavior. Restart with
    session restoration enabled and confirm tabs, splits, cursors, scroll, and
    sidebar state return.
13. Run `caret doctor`, then run `cargo test --all-targets --all-features` and
    `cargo build --release` from the repository root.
14. Open `:manager` at wide, medium, and narrow terminal widths. Confirm it
    transitions from parent/current/preview to current/preview to current-only,
    remains keyboard-usable, and mouse click, double-click, right-click action
    menu, and wheel navigation agree with the rendered rows.
15. Exercise selection, range selection, select-all, invert, filtering,
    directory history, inline create/rename/go-to, sorting, hidden files, safe
    previews, Ctrl-Enter split opening, failed-item retry, pane-ratio settings,
    and opening a terminal at the selected directory.
16. In a disposable tree, copy, cut/move, duplicate, paste with rename/skip/
    overwrite conflicts, cancel a multi-item copy, trash, and permanently
    delete. Confirm exact-path/count prompts, dirty-buffer blocking,
    partial-failure summaries, no copy-into-descendant recursion, and open-tab
    path synchronization after moves and renames.

The CI matrix covers release builds, tests, formatting, Clippy, and diagnostics
on Ubuntu, Windows, and macOS. The interactive portions above still require a
real terminal on each platform because mouse, PTY, colors, Alt-key handling,
and terminal restoration cannot be fully verified in headless CI.
