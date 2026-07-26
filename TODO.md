# Caret Work Tracker

This checklist is the implementation companion to [`ROADMAP.md`](ROADMAP.md).
It was reconciled against the repository on 2026-07-25 at commit `ea6b7a5`.

- `[x]` means the behavior exists in the codebase and has supporting tests or
  documentation where practical.
- `[ ]` means the roadmap requirement is incomplete. Notes call out partial or
  legacy implementations so they are not mistaken for finished foundations.
- Work is ordered by release priority. New roadmap work should be added here
  when it becomes actionable, and checked off in the same change that completes
  it.

## Current priorities

1. Finish the 0.6 cross-platform release sign-off and remaining reliability
   boundaries.
2. Replace the synchronous project tree with the non-blocking explorer
   foundation.
3. Complete the polished project explorer on that foundation.
4. Build the dedicated file manager, safe filesystem-operation layer, and
   preview system.
5. Finish 0.7 platform, accessibility, failure-path, and performance testing.

---

## Milestone 0.6 — Trustworthy Foundation

### Completed foundations

- [x] Atomic same-directory document and configuration saves with durable
  temporary files, permission preservation, Windows replacement/write-through,
  and Unix rename boundaries.
- [x] UTF-8 BOM, LF/CRLF, final-newline, binary, unsupported-encoding, empty
  file, long-line, and read-only handling.
- [x] Content fingerprints protect against delayed or coarse filesystem
  metadata.
- [x] Periodic crash-recovery journal with discovery, compare, recover, and
  discard workflows in the platform application-data directory.
- [x] Session restoration for projects, tabs, active tab, cursor and scroll
  positions, split layout, and sidebar state; terminal processes are excluded.
- [x] External-change reload, overwrite, compare, and defer protection,
  including deleted and same-size replaced files.
- [x] Structured diagnostics and panic logs, terminal restoration,
  `caret doctor`, `:doctor`, `:copydiagnostics`, LSP stderr capture, terminal
  capability reporting, and clipboard capability reporting.
- [x] Headless SSH clipboard fallback through OSC 52.
- [x] Focused reliability modules for documents, platform replacement,
  recovery, sessions, diagnostics, LSP transport, persistence, and settings,
  documented in `docs/ARCHITECTURE.md`.
- [x] Automated recovery, interrupted/write-failure save, encoding, Unicode,
  external-change, session, configuration, platform-path, shell, and PTY
  regression coverage.
- [x] Windows, macOS, and Linux CI build/test/diagnostic matrix plus Ubuntu
  install and PTY startup smoke coverage.

### Remaining release work

- [ ] Complete and record the interactive forced-termination and core-workflow
  sign-off on Windows, macOS, and Linux using `docs/SMOKE_TEST_0.6.md`.
- [ ] Continue decomposing the oversized `App` coordinator and `ui.rs` into
  focused event/update, command-registry, workspace, input, layout, and widget
  boundaries without weakening the reliability modules already extracted.
- [ ] Add the remaining reliability cases from the roadmap: broken symlinks,
  permission-denied directories, paths disappearing during reads, and slow or
  network-mounted directories where practical.
- [ ] Add structured diagnostics for the future watcher, preview, and
  filesystem-operation services as those services are introduced.
- [ ] Complete a sustained dogfooding pass with no unresolved data-loss defect;
  any data-loss issue blocks the release.

---

## Milestone 0.7 — Excellent Everyday Editing and Project Navigation

### Everyday editing and discovery already delivered

- [x] Typing-first non-modal Caret and Conventional profiles; modal Normal mode
  is limited to the Vim profile.
- [x] Centered searchable command palette with descriptions, matching shortcuts,
  keyboard/mouse navigation, scrolling, and a clickable title-bar entry point.
- [x] Configurable startup view and launch-target-aware sidebar visibility.
- [x] Operation-based undo/redo with grouped typing, selection and multi-cursor
  restoration, clear boundaries, and configurable history depth.
- [x] Auto-indent, smart backspace, paired delimiters, comments, indent/outdent,
  move/duplicate/join/split/sort line operations, trailing-whitespace trimming,
  and final-newline policy.
- [x] Multi-cursor deletion, cursor-above/below, column selection, next/all
  occurrence selection, and multiple-selection restoration.
- [x] Complete file find/replace with case, whole-word, regex, selection scope,
  history, match counts, replace-one, replace-all, keyboard, and mouse flows.
- [x] Gitignore-aware project search and reviewable project replacement with
  per-match exclusion, atomic rewrites, and dirty-tab protection.
- [x] Fuzzy file opener with matched-character highlighting and persisted recent
  files, plus a recent-project dashboard.
- [x] Three keymap profiles, user-defined bindings, conflict/interception
  warnings, reset, searchable binding browser, and platform-aware display.
- [x] Searchable settings catalog with current/default values, descriptions,
  validation, restart indicators, and immediate application where supported.
- [x] Tabs, split editing, navigation history, integrated PTY/ConPTY terminal,
  context menus, help, documentation, and 0.7 smoke-test instructions.

### Non-blocking filesystem foundation

- [x] Move Git-status collection to a background worker with refresh throttling,
  generation-based stale-result rejection, cached results, and UI-thread
  application.
- [x] Preserve project selection by path on refresh and protect recursive tree
  walks with depth/result limits and symlink-cycle rules.
- [ ] Replace the synchronous flattened `ProjectTree` authority with cached
  per-directory snapshots and a presentation-only flattened projection.
- [ ] Load directories lazily and perform directory scans outside rendering and
  keyboard handling.
- [ ] Add request/generation identifiers for directory scans, metadata, and
  previews, not only Git status.
- [ ] Add a debounced cross-platform filesystem watcher with targeted directory
  invalidation, overflow recovery, and stale-event rejection.
- [ ] Add progressive background metadata loading for visible and selected
  entries.
- [ ] Support cancellation or replacement of obsolete scan and preview work.
- [ ] Model explicit loading, empty, permission-denied, missing, and error
  states.

### Project explorer sidebar

- [x] Expand/collapse navigation, recursive controls, reveal-current-file,
  filter, hidden-file toggle, root `.gitignore` support, symlink markers,
  selection scrolling, mouse navigation, and adjustable width.
- [x] Correct last-child connector metadata, compact directory/file markers,
  trailing directory suffixes, full-row selection, and right-aligned basic Git
  badges.
- [x] Basic explorer context menus for open, create, rename, duplicate, Git
  stage/unstage, and editor actions.
- [x] Narrow-terminal layout and mouse hit-testing regression coverage for the
  current sidebar.
- [ ] Add a project header with discoverable new-file, new-folder, refresh, and
  collapse controls.
- [ ] Add configurable Unicode, Nerd Font, and ASCII icon modes.
- [ ] Add distinct visual roles for file types, directories, executables,
  symlinks, hidden entries, and ignored entries.
- [ ] Represent renamed and conflicted Git states distinctly instead of folding
  them into the basic modified state.
- [ ] Separate keyboard focus, selection, hover, and active-file styling.
- [ ] Highlight matched characters in explorer filtering and show breadcrumbs
  for the selected filesystem path.
- [ ] Replace command-prompt create/rename flows with true inline new-file,
  new-folder, and rename fields.
- [ ] Back explorer context menus and shortcuts with one shared command
  registry.
- [ ] Add optional size and modified-time columns for wide layouts.
- [ ] Add configurable sorting and directories-first behavior.
- [ ] Verify explorer behavior for Windows drive/UNC roots, macOS/Linux
  symlinks, Unicode filenames, SSH, tmux, monochrome, ASCII, mouse-disabled,
  and narrow-terminal sessions.

### Dedicated file manager workspace

- [ ] Add a themed `:manager` workspace with a configurable,
  terminal-conflict-checked shortcut.
- [ ] Implement parent, current, and preview/metadata panes with shared
  breadcrumbs, operation status, selection status, and contextual hints.
- [ ] Implement wide, medium, narrow, and very-narrow responsive layouts.
- [ ] Add keyboard and mouse directory navigation, parent traversal,
  back/forward history, filtering, sorting, and quick path entry.
- [ ] Add single, range, all, invert, and clear selection workflows.
- [ ] Open files in the current tab, a new tab, or a split, and open a terminal
  at the selected directory.
- [ ] Persist safe file-manager workspace state in sessions and reuse Caret's
  themes, commands, settings, and status conventions.

### Safe filesystem operations

The current project tree has synchronous, single-item create, rename, duplicate,
move, and permanent-delete commands. They are useful prototypes, but they do
not satisfy the roadmap's safe operation layer.

- [ ] Move all filesystem mutations into a dedicated non-blocking command layer
  below the explorer and file-manager UI.
- [ ] Implement new file/folder, rename, duplicate, copy, cut/move, paste,
  platform trash, explicit permanent delete, and multi-item operations.
- [ ] Add bulk rename after the core operations are stable.
- [ ] Add progress, safe cancellation, retry, and completed/cancelled/partial/
  failed result summaries.
- [ ] Add overwrite, skip, rename, apply-to-all, and cancel conflict handling.
- [ ] Add cross-device move fallback and safe operation undo where trustworthy.
- [ ] Reject copies/moves into the source or its descendants and preserve
  permissions and timestamps where practical.
- [ ] Handle locked, read-only, unavailable, case-only, and partially copied
  paths explicitly.
- [ ] Require exact-path or item-count confirmation for destructive actions and
  explicit confirmation before deleting/replacing an open dirty file.
- [ ] Synchronize path changes with open/dirty tabs, recent files, sessions,
  recovery records, navigation history, Git state, and active LSP documents.
- [ ] Refresh explorer snapshots only after confirmed operation results.

### Preview system

- [ ] Create a provider-based, background preview API with size/time limits,
  cancellation, generation IDs, and explicit unsupported/error states.
- [ ] Add syntax-colored text/source and Markdown-source previews.
- [ ] Add directory summaries and readable JSON, TOML, and YAML previews.
- [ ] Add binary metadata plus a bounded hexadecimal header.
- [ ] Add symlink target/status and basic image metadata previews.
- [ ] Guarantee that previews never execute untrusted files or block editor
  input.
- [ ] Defer terminal images, PDF/archive previews, media metadata, and
  extension-provided previewers until the initial provider API is stable.

### UI, settings, and test completion

- [ ] Introduce reusable top bar, tab bar, explorer, file-manager, file-list,
  preview, task-strip, editor, terminal, status, prompt, menu, and confirmation
  widgets incrementally; do not rewrite the editor core for the renderer.
- [ ] Use one calculated layout for both drawing and mouse hit testing across
  every workspace.
- [ ] Add semantic theme roles for panels, focus, selections, file states, Git
  states, metadata, operation states, warnings, and destructive confirmations,
  with derived defaults for existing and plugin themes.
- [ ] Extend settings for icon mode, explorer sorting/directories-first,
  hidden/ignored policy, manager pane ratios, preview limits, confirmation
  policy, and trash behavior.
- [ ] Add failure/performance tests for large/deep directories, broken links,
  permissions, disappearing paths, event bursts, stale results, watcher
  overflow, large Git states, operation failures/conflicts, cross-device moves,
  locked files, trash behavior, and path synchronization.
- [ ] Complete and record the 0.7 interactive smoke matrix on Windows, macOS,
  Linux, SSH, tmux, monochrome/ASCII, narrow terminals, and mouse-disabled
  sessions using `docs/SMOKE_TEST_0.7.md`.

---

## Milestone 0.8 — Zero-Configuration Coding

### Existing foundations

- [x] File-type detection and syntax coloring for Rust, Go, C#, Python, Bash,
  YAML, JSON, TOML, and Markdown, plus syntax folding, symbols, and
  breadcrumbs.
- [x] Manual C#/Rust LSP startup with framed transport, stderr logging,
  workspace roots, full-document synchronization, diagnostics, completion,
  hover, definition, references, rename, code actions, formatting,
  format-on-save, snippets, and workspace edits.

### Remaining work

- [ ] Store one persistent full-document tree-sitter tree per open document and
  update it incrementally instead of creating parsers during line rendering and
  feature requests.
- [ ] Move highlighting to queries and add indentation queries plus bracket/
  syntax-node matching.
- [ ] Create a data-driven language registry for extensions, comments, grammar,
  queries, server, formatter, and platform-specific executable guidance.
- [ ] Deliver complete Rust, C#, Python, Go, and JavaScript/TypeScript language
  experiences; JavaScript/TypeScript parsing and LSP support are not present.
- [ ] Detect and start available language servers automatically with visible
  startup/indexing state, restart/stop, per-project disable, per-language
  configuration, actionable missing-server guidance, and platform-aware
  executable discovery.
- [ ] Add signature help, selection formatting, document/workspace symbols, and
  robust diagnostics/problems workflows.
- [ ] Negotiate UTF-8/UTF-16/UTF-32 position encodings and replace
  full-document `didChange` updates with incremental synchronization.
- [ ] Add request cancellation, graceful restart, multiple simultaneous
  servers, progress/configuration handling, and cross-platform integration
  tests for all five official languages.
- [ ] Keep explorer rename/move operations synchronized with LSP workspaces and
  open documents.

---

## Milestone 0.9 — Installation, Packaging, and Onboarding

### Existing foundations

- [x] CI formatting, Clippy, tests, release builds, and diagnostic smoke runs on
  Windows, macOS, and Linux.
- [x] Basic Unix installation script and source-build documentation.

### Remaining work

- [ ] Produce installable Windows x64 artifacts, PATH installer, Winget and
  Scoop packages, checksums, and signing where feasible.
- [ ] Produce Apple Silicon and practical Intel/universal macOS artifacts,
  Homebrew formula, checksums, uninstall instructions, signing, and
  notarization.
- [ ] Produce Linux x86_64 and practical ARM64 binaries, tarball, `.deb`, RPM,
  checksums, uninstall instructions, and AUR/Nix guidance.
- [ ] Add tagged-release automation for packaging, checksums, SBOM, release
  notes, artifact upload, and installation smoke tests.
- [ ] Build a skippable first-launch flow covering keymap choice, themes/icons,
  editor, explorer, file manager, fuzzy opening, command palette, terminal,
  help, platform modifiers, current-folder opening, and a short tutorial.
- [ ] Expand searchable help with platform-aware key display, clickable
  commands, explorer/manager safety workflows, examples, troubleshooting, LSP,
  terminal, settings, clipboard, and SSH guidance.
- [ ] Test installation, upgrade, PATH integration, and removal on all three
  platforms without requiring a Rust toolchain.

---

## Milestone 0.10 — Controlled Extensibility

### Existing prototype

- [x] Out-of-process TOML plugin prototype for commands, document/selection
  edits, file opening, notifications, language comment rules, themes, and
  after-save hooks.
- [x] Per-command timeouts, stderr failure messages, JSON request/response
  transport, reload/list commands, documentation, and a tested sample plugin.

### Remaining work

- [ ] Rename and formalize the system as versioned extensions with explicit
  protocol compatibility and capabilities.
- [ ] Add stable before-save and file-open hooks plus safe preview providers.
- [ ] Make extension edits one document undo operation in every response path.
- [ ] Add cancellable execution, structured output/error logging, reliable
  process cleanup, and Windows/macOS/Linux launch tests.
- [ ] Display source, executable, requested capabilities, project/user scope,
  and trust status before execution.
- [ ] Enforce preview size/time/capability limits and ensure extension failures
  cannot freeze or crash Caret.
- [ ] Keep an online extension marketplace out of scope until the protocol is
  stable.

---

## Milestone 1.0 Beta — Hardening and Feature Freeze

- [ ] Freeze major feature work after milestones 0.6–0.10 meet their exit
  criteria; accept only reliability, performance, accessibility,
  compatibility, documentation, and API-stabilization work.
- [ ] Add repeatable benchmarks for startup, editing, undo, rendering,
  scanning, navigation, watchers, previews, Git, file operations, search,
  parsing, LSP, terminal behavior, CPU, and memory.
- [ ] Add and verify large-file mode, near-zero idle CPU, responsive
  100,000-line editing, and stable large-project navigation.
- [ ] Complete the roadmap's Windows, macOS, Linux, SSH, tmux, narrow-terminal,
  clipboard-disabled, monochrome, ASCII, permission, broken-link, shell,
  network-path, and long-session compatibility matrix.
- [ ] Dogfood Caret for sustained real development and track crashes, failed
  saves, undo defects, rendering/input issues, stale filesystem work, partial
  operations, resource growth, and workflows that still require another tool.

---

## Known release-blocking or tracked defects

- [ ] Fix C# `:def`: Caret returns no definition even though the same
  `csharp-ls` request succeeds against the loaded solution in an isolated
  protocol test.
- [ ] Treat every newly discovered data-loss defect as release-blocking and add
  a regression test with the fix.

Features explicitly listed under “Features That Should Wait Until After 1.0” in
`ROADMAP.md` are intentionally omitted from this actionable checklist.
