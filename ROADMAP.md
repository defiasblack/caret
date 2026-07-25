# Caret Roadmap

This roadmap describes the project's current direction. It is not a promise of specific features or delivery dates. Priorities may change based on user feedback, maintenance needs, reliability work, and contributor availability.

## Near-term priorities

- Improve reliability across Windows, Linux, macOS, SSH, and tmux sessions
- Complete the non-blocking filesystem foundation needed for a fast project explorer
- Redesign the project sidebar into a polished, responsive, editor-first explorer
- Add a dedicated full-screen file manager without making normal editing feel heavy
- Expand automated tests for editing, navigation, tabs, filesystem behavior, file operations, and terminal integration
- Improve error messages and recovery when files, shells, language servers, extensions, or background operations fail
- Make installation and upgrades easier with documented release artifacts
- Continue accessibility work for keyboard-only, monochrome, reduced-motion, ASCII-icon, and mouse-disabled use
- Keep documentation, built-in help, settings, and command discovery synchronized

---

## Product Vision

**Caret is the friendly terminal code editor for people who want IDE conveniences without IDE weight, complicated configuration, or a steep learning curve.**

Caret should be an editor someone can install, open, and understand without watching hours of tutorials or assembling a plugin configuration.

It should feel:

- Easier to learn than Helix
- More project-oriented than Micro
- Less complicated than Neovim
- Familiar to Windows, macOS, and Linux users
- Fast and dependable over SSH
- Useful immediately with sensible defaults
- Capable of serious project and filesystem work without leaving the editor

Caret's file experience should combine:

- The responsive, asynchronous navigation principles of Yazi
- The visual polish, panel hierarchy, and discoverability of Superfile
- Caret's own editor-first workflow, themes, command palette, tabs, terminal, and coding tools

Caret should learn from those projects without embedding, cloning, or tightly coupling itself to either one.

---

## Competitive Position

Caret does **not** need to beat Neovim, Emacs, Helix, Yazi, Superfile, or Micro in every category.

Caret needs to become the best choice for this user:

> “I want to edit code in the terminal, browse and organize my project, preview files, run commands, get completions and errors, and use familiar keyboard shortcuts without configuring an entire development environment.”

### Where Caret Should Win

| Category | Caret's Intended Advantage |
|---|---|
| Installation | One straightforward installation with no configuration required |
| First launch | Friendly dashboard, clear commands, visible shortcuts |
| Windows | Excellent Windows Terminal, PowerShell, Command Prompt, and ConPTY support |
| macOS | Excellent Terminal, iTerm2, Apple Silicon, Intel, zsh, and Homebrew support |
| Linux | Excellent support across major distributions, shells, terminals, SSH, and tmux |
| Project editing | Polished project explorer, fuzzy opening, project search, previews, and integrated terminal |
| File management | Safe multi-selection, copy, move, rename, trash, conflict handling, and progress reporting |
| Coding support | Automatic LSP setup with useful failure explanations |
| Input | Mouse, conventional shortcuts, Caret navigation, and optional Vim-inspired navigation |
| Discoverability | Command palette, contextual hints, searchable help, settings, and visible panel controls |
| Configuration | Good defaults with optional, understandable customization |
| SSH usage | Complete keyboard workflow and dependable terminal rendering |
| Safety | Atomic saves, recovery, conflict handling, safe file operations, and session restoration |

---

# Strategic Rules

## 1. Stability Before Feature Count

Caret already has enough visible features to demonstrate the product. New subsystems must not come at the cost of data safety, predictable input, or dependable rendering.

## 2. Zero Configuration Must Be the Default

A configuration file may improve Caret, but it should never be required to make Caret useful.

## 3. Every Important Action Must Be Discoverable

Important actions should have at least two of these entry points:

- Keyboard shortcut
- Command-palette entry
- Typed command
- Visible or clickable interface control
- Context menu where appropriate

## 4. Windows, macOS, and Linux Are First-Class Platforms

No platform should be treated as a secondary build target.

A feature is not complete until it has been checked for:

- Windows Terminal and PowerShell
- Command Prompt and ConPTY where relevant
- macOS Terminal and iTerm2
- Common Linux terminals
- SSH sessions
- tmux where relevant
- Clipboard-disabled and mouse-disabled environments
- Platform-specific paths, permissions, symlinks, trash behavior, and shell behavior

## 5. Caret Mode Is the Primary Experience

Caret can retain Conventional and Vim-inspired profiles, but the product should not become trapped trying to reproduce all of Vim.

## 6. No Feature Is Complete Without Tests and Documentation

A merged feature is not finished until it has:

- Automated tests where practical
- Failure-path tests for reliability-sensitive behavior
- Error handling
- Help text
- Keyboard access
- Documentation
- Release notes
- Platform verification

## 7. Filesystem Work Must Never Block Editing

Directory scans, Git status, previews, metadata reads, filesystem watching, copies, moves, deletion, and extension work must not block keyboard input or editor rendering.

Stale background results must be rejected through request or generation identifiers.

## 8. The File Manager Must Remain Editor-First

Caret should provide a powerful file manager, but it must not turn into a terminal file manager with an editor bolted on.

The lightweight project explorer remains the normal coding interface. The full file manager is an intentional workspace view for larger filesystem tasks.

## 9. The Interface Must Degrade Gracefully

Every major workspace must have useful wide, medium, narrow, monochrome, ASCII, keyboard-only, and mouse-enabled layouts.

Nerd Fonts may improve presentation but must never be required.

---

# Platform Support Policy

## Windows

Officially support:

- Windows 11
- Windows Server where practical
- Windows Terminal
- PowerShell
- Command Prompt
- OpenSSH sessions
- ConPTY
- Standard Windows clipboard behavior
- Drive-letter paths
- UNC paths
- Junctions and symlinks where available
- CRLF files
- Windows trash and file-locking behavior

## macOS

Officially support:

- Current and previous major macOS releases
- Apple Silicon
- Intel Macs while practical
- macOS Terminal
- iTerm2
- WezTerm where practical
- zsh
- bash
- fish where practical
- Homebrew installation
- Standard macOS clipboard behavior
- macOS trash, permissions, and symlinks
- Code signing and notarization when feasible

## Linux

Officially support:

- Ubuntu LTS
- Debian stable
- Fedora
- Arch Linux through community or package-manager support
- Common glibc-based distributions
- GNOME Terminal
- Konsole
- Alacritty
- Kitty
- WezTerm
- xterm-compatible terminals
- bash
- zsh
- fish
- tmux
- OpenSSH sessions
- Headless systems with no desktop clipboard
- Freedesktop trash behavior where available

## Platform Parity Rule

No release should advertise a feature as complete unless:

- Core behavior works on Windows, macOS, and Linux
- Platform-specific failures produce useful messages
- CI builds all supported platforms
- Manual smoke tests cover the release's major workflows
- Documentation includes platform-specific notes where needed

---

# Milestone 0.6 — Trustworthy Foundation

## Objective

Make Caret safe enough that users can trust it with important files.

This is the highest-priority milestone. Major user-facing subsystems must build on these reliability boundaries rather than bypassing them.

## Data Safety

Implement and verify:

### Atomic File Saving

- Write changes to a temporary file in the same directory
- Flush and synchronize the temporary file
- Preserve file permissions where possible
- Replace the original only after the temporary file is complete
- Never truncate the original before replacement is ready
- Handle Windows replacement and locking semantics correctly
- Handle macOS and Linux permissions correctly

### Crash-Recovery Journal

- Periodically record unsaved buffer changes
- Detect recovery data on startup
- Show filename, timestamp, and available actions
- Allow users to recover, compare, or discard the recovery version
- Store recovery data in the correct platform-specific application directory

### Session Restoration

Restore:

- Open project
- Tabs
- Active tab
- Cursor positions
- Scroll positions
- Split layout
- Sidebar state
- Workspace view where safe

Do not silently restore terminal processes or unfinished file operations.

### External-Change Conflict Handling

- Clearly distinguish reload, overwrite, compare, and later
- Never overwrite a changed disk file without confirmation
- Handle deleted or renamed files gracefully
- Handle network-mounted files and delayed timestamps carefully

### File-Format Handling

- Detect and preserve LF versus CRLF
- Correctly handle UTF-8 BOM files
- Detect binary files before displaying them as text
- Show a useful error for unsupported encodings
- Preserve final-newline state
- Avoid changing line endings unless requested

## Architecture

Break central application logic into focused systems:

```text
src/
  app/
    state.rs
    events.rs
    update.rs
    workspace.rs
  commands/
    registry.rs
    editor.rs
    explorer.rs
    project.rs
    workspace.rs
  document/
    buffer.rs
    history.rs
    persistence.rs
    recovery.rs
  explorer/
    model.rs
    entry.rs
    tree.rs
    browser.rs
    worker.rs
    watcher.rs
    preview.rs
    operations.rs
    clipboard.rs
    sort.rs
    git.rs
  input/
    keymap.rs
    mouse.rs
  workspace/
    project.rs
    session.rs
  services/
    lsp.rs
    git.rs
    terminal.rs
  platform/
    windows.rs
    macos.rs
    linux.rs
  ui/
    layout.rs
    style.rs
    editor.rs
    overlays.rs
    widgets/
      project_explorer.rs
      file_manager.rs
      file_list.rs
      preview.rs
      task_strip.rs
```

The exact names may change, but reliability-sensitive responsibilities must not remain concentrated in one application object or renderer.

## Diagnostics and Support

Add and maintain:

- Structured log file
- `caret doctor`
- Version, terminal, operating system, shell, and configuration report
- LSP process output and errors
- Background filesystem-operation logs
- Watcher and preview failure information
- Terminal capability report
- Clipboard capability report
- Safe “copy diagnostic report” command
- Panic recovery that restores the terminal before exiting

## Test Requirements

Add automated coverage for:

- Interrupted saves
- Disk-full or write-failure simulation
- CRLF and LF preservation
- Unicode cursor movement
- Combining characters and double-width characters
- External file changes
- Recovery after forced termination
- Invalid settings files
- Extremely long lines
- Empty files
- Read-only files
- Files deleted while open
- Windows, macOS, and Linux paths
- Shell detection
- Platform-specific application directories
- Symlink loops and broken links
- Permission-denied directories
- Network-mounted or slow directories where practical

## Exit Criteria

Version 0.6 is complete only when:

- Killing Caret during a save cannot destroy the original file
- An unsaved document can be restored after forced termination
- External disk changes cannot be overwritten accidentally
- Windows, macOS, and Linux CI pass
- Current editor features still work after architecture changes
- Caret can be used for sustained editing without known data-loss defects
- Core workflows have been manually smoke-tested on all three platforms

---

# Milestone 0.7 — Excellent Everyday Editing and Project Navigation

## Objective

Make Caret substantially more comfortable than a basic terminal editor for normal daily work, project navigation, and safe filesystem operations.

Version 0.7 should deliver two complementary file experiences:

1. A lightweight, polished **Project Explorer** used during normal editing
2. A dedicated, responsive **File Manager** used for previews, multi-selection, and substantial filesystem work

## Explorer Design Principles

- Use Yazi as a reference for asynchronous state, parent/current/preview navigation, task handling, and cancellation
- Use Superfile as a reference for panel hierarchy, focused borders, headers, footers, metadata, and presentation polish
- Implement the experience natively in Caret rather than launching or embedding another application
- Keep editor tabs, active documents, LSP state, recent files, recovery data, and filesystem operations synchronized
- Preserve the existing `Ctrl-E`, `Ctrl-B`, fuzzy-file, command-palette, mouse, and terminal workflows

## Non-Blocking Filesystem Foundation

Replace the synchronous flattened tree as the authoritative filesystem model with cached directory snapshots and render projections.

Implement:

- Cached snapshots for loaded directories
- Lazy loading when a directory expands or becomes visible
- A flattened tree projection used only for presentation and navigation
- Background directory scans
- Background Git-status collection
- Background metadata and preview generation
- Request identifiers and generation identifiers to discard stale results
- A debounced cross-platform filesystem watcher
- Targeted invalidation of changed directories instead of full project rescans
- Path-based selection restoration after refreshes
- Progressive metadata loading for visible and selected entries
- Cancellation or replacement of obsolete preview and scan work
- Explicit loading, empty, permission-denied, missing, and error states

The main editor loop may coordinate background events, but filesystem I/O must remain outside rendering and keyboard handling.

## Project Explorer Sidebar

Redesign the existing project sidebar while retaining its quick editor workflow.

Implement:

- Clear project header with new-file, new-folder, refresh, and collapse controls
- Cleaner indentation and correct last-child connector rendering
- Unicode, Nerd Font, and ASCII icon modes
- File-type, directory, executable, symlink, hidden, and ignored visual states
- Right-aligned Git badges for modified, added, deleted, renamed, conflicted, and untracked paths
- Distinct selection, keyboard focus, hover, and active-file states
- Full-row selection highlighting
- Matched-character highlighting while filtering
- Breadcrumbs for the selected path
- Inline rename, new-file, and new-folder fields
- Context menus backed by the same command registry as keyboard actions
- Optional size and modified-time columns at wider widths
- Responsive layouts that simplify rather than break at narrow widths
- Configurable directories-first behavior and sorting
- Preserve reveal-current-file, hidden-file toggle, `.gitignore`, symlink, and recursive expansion behavior

Avoid redundant labels such as a literal `DIR` marker when iconography, color, expansion state, and suffix already communicate the type.

## Full File Manager Workspace

Add a dedicated workspace view opened through a command such as `:manager` and a configurable shortcut.

The default wide layout should provide:

- Parent directory pane
- Current directory pane
- Preview or metadata pane
- Path and breadcrumb header
- Operation and selection status
- Context-sensitive action hints

Responsive behavior:

- Wide terminals: parent, current, and preview panes
- Medium terminals: current and preview panes
- Narrow terminals: current pane with preview as a toggle
- Very narrow terminals: names and essential status only

Navigation should support:

- Arrow keys and Caret bindings
- Optional `h`, `j`, `k`, and `l` navigation where compatible with the active keymap
- Enter to open a file or enter a directory
- Backspace or Left to return to the parent
- Space to toggle selection
- Select-all, invert-selection, range-selection, and clear-selection actions
- Mouse click, double-click, context menu, and wheel scrolling
- Directory history and forward/back navigation
- Sorting, filtering, and quick path entry
- Open in current tab, new tab, or split
- Open a terminal at the selected directory

The file manager must use Caret's existing themes, commands, settings, session model, and status conventions.

## Safe File Operations

Create a dedicated filesystem-command layer below the UI.

Implement:

- New file
- New folder
- Rename
- Duplicate
- Copy
- Cut and move
- Paste
- Trash
- Explicit permanent delete
- Multi-item operations
- Bulk rename after the core operations are stable
- Progress reporting
- Cancellation where safe
- Retry and failure summaries
- Conflict resolution: overwrite, skip, rename, apply to all, and cancel
- Cross-device move fallback
- File-operation undo where practical and trustworthy

Safety requirements:

- Never permanently delete through an ambiguous shortcut
- Show the exact path or item count in destructive confirmations
- Prefer platform trash when available
- Detect attempts to copy or move a directory into itself or a descendant
- Preserve permissions and timestamps where practical
- Handle locked, read-only, unavailable, or partially copied files explicitly
- Keep completed, cancelled, partially completed, and failed states distinct
- Update project snapshots only after confirmed results

When a path changes, Caret must update or warn about:

- Open tabs
- Dirty buffers
- Recent files
- Session state
- Recovery references
- Active LSP documents
- Git status
- Navigation history

Deleting or replacing an open dirty file requires explicit confirmation.

## Preview System

Introduce a provider-based preview API.

Initial previews:

- Text and source files using Caret's syntax and line rendering
- Directories with child counts and summary information
- JSON, TOML, and YAML with readable structured formatting where safe
- Markdown as source with optional styled preview later
- Binary files with type, size, timestamps, and a short hexadecimal header
- Symlink destination and status
- Basic image metadata before inline image rendering is attempted

Later previews may include:

- Terminal image protocols
- PDF previews
- Archives
- Audio and video metadata
- Extension-provided previewers

Preview requirements:

- Size and time limits
- Cancellation
- Generation identifiers
- Clear unsupported and error states
- No blocking of editor input
- No execution of untrusted files merely to preview them

## UI Rendering and Visual System

Incrementally migrate the UI toward reusable layout and widget boundaries, using Ratatui where it reduces manual layout complexity and improves testability.

Do not rewrite the application state or editor core merely to change the rendering library.

Create reusable widgets for:

- Top bar
- Tab bar
- Project explorer
- File manager
- File list
- Preview
- Task strip
- Editor
- Terminal
- Status bar
- Prompts, menus, and confirmations

Use one calculated layout for both rendering and mouse hit testing.

Add semantic theme roles for:

- Panel background and alternate background
- Focused and unfocused panel borders
- Selection foreground and background
- Active file
- Directory, regular file, symlink, executable, hidden, and ignored entries
- Git modified, added, deleted, renamed, conflicted, and untracked states
- Metadata keys and values
- Running, successful, cancelled, and failed operations
- Warnings and destructive confirmations

Existing themes must remain compatible through sensible derived defaults. Plugin themes may override the new roles.

## File Navigation and Search

Implement or strengthen:

- Fuzzy file opener
- Recently opened files
- Recently opened projects
- File-tree filtering
- Reveal current file
- `.gitignore` support
- Hidden-file toggle
- Symlink handling
- Correct handling of Windows drive roots and UNC paths
- Correct handling of macOS and Linux symlinks
- Correct handling of case-sensitive and case-insensitive filesystems
- Project-wide search
- Project-wide replacement with preview

Suggested default:

```text
Ctrl-P — Open file by name
Ctrl-E — Switch between editor and project explorer
Ctrl-B — Show or hide the project explorer
```

The full file-manager shortcut should be selected only after checking terminal conflicts across supported platforms.

## Find and Replace

Implement and stabilize:

- Find in current file
- Replace one
- Replace all
- Case-sensitivity toggle
- Whole-word toggle
- Regular-expression mode
- Search history
- Search within selection
- Project-wide search
- Project-wide replacement with reviewable preview
- Keyboard and mouse navigation

## Editing Fundamentals

Strengthen:

- Multi-cursor editing
- Multiple selections
- Column selection
- Select all occurrences
- Indent and outdent selections
- Auto-indentation
- Smart backspace
- Paired delimiters
- Toggle comments
- Move and duplicate lines
- Join and split lines
- Sort selected lines
- Trim trailing whitespace
- Configurable final-newline behavior

## Undo and Redo

Use operation-oriented history that supports:

- Efficient large-document undo
- Grouped typing operations
- Selection restoration
- Multi-cursor restoration
- Clear undo boundaries
- Configurable history limits
- Optional persistent undo later

Filesystem operation undo must remain separate from document undo and only be offered when it can be implemented safely.

## Keymaps

Support three official profiles:

| Profile | Purpose |
|---|---|
| Caret | Recommended balanced workflow |
| Conventional | Familiar shortcuts similar to graphical editors |
| Vim-inspired | Modal navigation without claiming complete Vim compatibility |

Add:

- User-defined bindings
- Key-conflict detection
- Searchable keybinding browser
- Reset-to-default option
- Per-profile help
- Context-sensitive explorer and file-manager hints
- Clear warning when a terminal intercepts a shortcut
- Platform-aware defaults for macOS modifier keys
- Documentation for Ctrl, Alt, Option, and Command differences

Do not attempt complete Vim operator, register, and text-object compatibility before 1.0.

## Settings Experience

Create a real settings interface for:

- Theme
- Icon mode
- Explorer width
- Explorer sorting
- Directories-first behavior
- Hidden and ignored files
- File-manager pane ratios
- Preview enablement and limits
- Confirmation policy
- Trash behavior
- Search settings
- Keymaps
- Existing editor settings

The settings interface must:

- Be searchable
- Show current and default values
- Explain each setting
- Validate before saving
- Indicate whether restart is required
- Apply safe settings immediately where possible
- Provide “Open settings file” for advanced users
- Use platform-standard configuration locations

## Explorer and File-Operation Tests

Add automated and manual coverage for:

- Large directories
- Deep directory trees
- Symlink loops
- Broken symlinks
- Permission-denied paths
- Paths disappearing during scans
- Rapid filesystem event bursts
- Out-of-order background results
- Watcher overflow and recovery
- Git repositories with thousands of changed files
- Copy, move, rename, trash, and delete failures
- Name conflicts and apply-to-all choices
- Cross-device moves
- Case-only renames
- Windows locked files
- Windows drive and UNC roots
- macOS and Linux trash behavior
- Unicode and double-width filenames
- Narrow terminals
- ASCII and monochrome modes
- Mouse-disabled sessions
- SSH and tmux
- Open-tab synchronization after rename or move
- Dirty-buffer behavior when a path is deleted or replaced

## Exit Criteria

Version 0.7 is complete when a user can comfortably:

1. Open a project
2. Understand and navigate a polished project explorer
3. Find and preview files without blocking the editor
4. Use the full file manager for multi-selection and common filesystem operations
5. Copy, move, rename, trash, and resolve conflicts safely
6. Search and replace across the project
7. Edit across several tabs and splits
8. Use selections and multi-cursors
9. Run commands in the integrated terminal
10. Understand available actions without external documentation
11. Complete these workflows on Windows, macOS, and Linux
12. Use the editor over SSH without requiring a mouse, Nerd Font, or desktop clipboard

---

# Milestone 0.8 — Zero-Configuration Coding

## Objective

Turn Caret into a useful code editor rather than only a capable text editor.

## Tree-Sitter Integration

Move to:

- One persistent syntax tree per open document
- Full-document parsing
- Incremental tree updates after edits
- Query-based highlighting
- Syntax-aware folding
- Symbol extraction
- Breadcrumbs
- Indentation queries where supported
- Bracket and syntax-node matching

Do not create a new parser for every displayed line.

## Language Registry

Create a data-driven language registry containing:

- Name and extensions
- Comment rules
- Language-server identifier and command
- Formatter
- Tree-sitter language and queries
- Platform-specific executable guidance

Language support should not require hard-coded branches throughout the application.

## Initial Official Languages

Concentrate on five excellent language experiences:

1. Rust
2. C#
3. Python
4. Go
5. JavaScript and TypeScript

Existing configuration-format support can remain, but the official languages should receive complete coding workflows.

## Automatic LSP Behavior

Replace manual startup with:

- Automatic language detection
- Automatic server detection and startup
- Visible startup and indexing state
- Server restart and stop
- Detailed errors and output logs
- Per-project disable option
- Per-language configuration
- Platform-aware executable discovery
- Correct PATH behavior on Windows, macOS, and Linux
- Shell-independent process launching

## Coding Features

Stabilize:

- Completion and documentation
- Hover information
- Go to definition
- Find references
- Rename symbol
- Code actions
- Diagnostics and problems panel
- Document and selection formatting
- Format on save
- Document and workspace symbols
- Signature help
- Workspace edits

## LSP Correctness

Support:

- UTF-8, UTF-16, and UTF-32 position encodings
- Incremental synchronization
- Workspace folders
- Server requests and cancellation
- Graceful restart
- Multiple simultaneous language servers
- Configuration requests
- Progress notifications
- Correct snippet handling
- Platform-specific process and path behavior

## Exit Criteria

Version 0.8 is complete when:

- Opening a supported project automatically activates available coding support
- No manual LSP command is required for the normal workflow
- A missing server produces actionable guidance
- Server crashes do not crash Caret
- Diagnostics, navigation, and completion work consistently in all five official languages
- Tree-sitter updates incrementally
- Explorer renames and moves remain synchronized with LSP state
- Supported language workflows work on Windows, macOS, and Linux

---

# Milestone 0.9 — Installation, Packaging, and Onboarding

## Objective

Make installing and learning Caret easier than configuring another terminal editor.

## Distribution

Provide:

### Windows

- Standalone x64 executable
- Installer and PATH integration
- Winget package
- Scoop package
- Checksums
- Signed binaries when feasible

### macOS

- Apple Silicon binary
- Intel binary while practical
- Universal binary if practical
- Homebrew formula
- Signing and notarization when feasible
- Checksums and uninstall instructions

### Linux

- Standalone x86_64 binary
- ARM64 binary where practical
- Tar archive
- `.deb` package
- RPM package
- AppImage only if it adds real value
- Community packaging guidance for AUR and Nix
- Installation script
- Checksums and uninstall instructions

Users should not need Rust or Cargo installed.

## Release Automation

Every tagged release should automatically:

1. Run formatting checks
2. Run Clippy
3. Run all tests
4. Build supported Windows targets
5. Build supported macOS targets
6. Build supported Linux targets
7. Package artifacts
8. Produce checksums
9. Generate a software bill of materials
10. Generate release notes
11. Upload artifacts
12. Run installation smoke tests

## First-Launch Experience

On first launch:

1. Welcome the user
2. Offer Caret, Conventional, or Vim-inspired keys
3. Let the user preview themes and icon modes
4. Explain the editor, project explorer, fuzzy opener, command palette, terminal, and help
5. Explain macOS modifier-key differences where relevant
6. Offer to open the current directory
7. Provide a short interactive tutorial
8. Make every step skippable

## Help System

Include:

- Search and categories
- Current keymap
- Platform-specific key display
- Clickable commands
- Explorer and file-manager workflows
- File-operation safety and trash behavior
- Examples and troubleshooting
- LSP setup
- Terminal compatibility
- Settings explanations
- Clipboard and SSH guidance

## Exit Criteria

Version 0.9 is complete when:

- A new user can install Caret without building it
- Installation correctly places Caret on PATH
- First launch explains the essential editor and project workflow
- Caret can diagnose common terminal, clipboard, shell, filesystem, preview, and LSP failures
- Installation, upgrade, and removal are tested on Windows, macOS, and Linux
- Release artifacts are available for all three platforms

---

# Milestone 0.10 — Controlled Extensibility

## Objective

Allow useful customization without turning Caret into a plugin-management project.

## Positioning

Call the current system **extensions** unless and until it exposes a broad, stable editor API.

Caret does not need to compete with Neovim's plugin ecosystem before 1.0.

## Extension Protocol

Create a versioned protocol supporting controlled capabilities:

- Register command
- Read active document and selection
- Replace selection or apply document edits
- Open a file
- Show a notification
- Add a language definition
- Add a theme
- Add a safe preview provider
- Run before or after save
- Run on file open

## Reliability Requirements

- Extensions run outside the editor process
- A failed extension cannot crash Caret
- Long-running extensions can be cancelled
- Timeouts are enforced
- Output and errors are logged
- Protocol compatibility is checked
- Extension edits are one undoable document operation
- Extension execution never blocks keyboard input
- Preview extensions receive explicit size, time, and capability limits
- Extension launching works consistently across Windows, macOS, and Linux

## Security

Clearly display:

- Extension source
- Executable being launched
- Requested capabilities
- Project or user scope
- Trust status
- Platform-specific path and permission information

Do not build an online extension marketplace before the protocol is stable.

## Exit Criteria

Version 0.10 is complete when:

- The protocol is documented and versioned
- Broken extensions cannot freeze or crash Caret
- Extension edits participate in undo
- Error reporting is understandable
- Official examples are tested in CI
- Extension behavior is verified on all three platforms

---

# Milestone 1.0 Beta — Hardening and Feature Freeze

## Objective

Stop expanding scope and prove Caret can be depended upon.

## Feature Freeze

During beta:

- No major new subsystems
- No debugger
- No AI assistant
- No remote-development platform
- No extension marketplace
- No attempt at complete Vim compatibility

Only accept:

- Bug fixes
- Performance improvements
- Accessibility fixes
- Compatibility fixes
- Documentation corrections
- Necessary API stabilization

## Performance Program

Create repeatable benchmarks for:

- Startup
- Opening files
- Inserting text
- Deleting large selections
- Undo and redo
- Rendering
- Project-tree scanning
- Directory navigation
- Watcher event bursts
- Preview generation
- Git-status refresh
- Copy and move throughput
- Project search
- Syntax parsing
- LSP startup
- Memory usage

Initial targets:

- No perceptible input lag during ordinary editing
- Near-zero CPU use while idle
- Responsive editing of files containing at least 100,000 lines
- Large-file mode that disables expensive coding features when necessary
- Project scanning and previewing that do not freeze the interface
- LSP, Git, watcher, and filesystem operations that never block text input
- Stable memory use while navigating large projects
- Responsive narrow-terminal and SSH rendering

## Compatibility Testing

Manually test:

### Windows

- Windows Terminal with PowerShell
- Windows Terminal with Command Prompt
- Windows Terminal over SSH
- Windows Server where practical
- Drive roots, UNC paths, locked files, and trash behavior

### macOS

- macOS Terminal
- iTerm2
- Apple Silicon
- Intel where supported
- zsh
- Homebrew installation
- SSH sessions
- Trash, permissions, and symlinks

### Linux

- Ubuntu LTS
- Debian stable
- Fedora
- GNOME Terminal
- Konsole
- Alacritty
- Kitty
- tmux
- OpenSSH
- Headless servers
- Freedesktop trash behavior

### Shared Scenarios

- Narrow terminals
- Mouse-disabled terminals
- Clipboard-disabled SSH sessions
- 256-color terminals
- ASCII-only icon mode
- Unicode and Nerd Font modes
- Long-running editing sessions
- Large and rapidly changing repositories
- Permission failures
- Broken links
- Shell restarts
- Network-mounted projects

## Dogfooding

Use Caret itself for real development work.

Track:

- Crashes
- Lost cursor or explorer positions
- Failed saves
- Broken undo
- Rendering corruption
- Incorrect selections
- Terminal key conflicts
- LSP hangs
- Stale previews or directory views
- Failed or partial file operations
- High CPU or memory use
- Platform-specific inconsistencies
- Workflows that require escaping to another editor or file manager

Any data-loss defect blocks 1.0.

---

# Caret 1.0 Definition

Caret 1.0 should mean:

## Reliability

- Atomic saves
- Crash recovery
- Session recovery
- External-change protection
- Stable undo and redo
- Safe, observable filesystem operations
- No known data-loss bugs

## Editing

- Excellent basic editing
- Find and replace
- Project search
- Fuzzy file opening
- Multiple selections and multi-cursor support
- Tabs and splits
- Configurable keymaps

## Project and Filesystem Experience

- Polished project explorer
- Responsive full file manager
- Parent/current/preview navigation
- Multi-selection
- Copy, move, rename, duplicate, trash, and permanent-delete safeguards
- Conflict resolution and progress reporting
- Background scans, Git status, previews, and watchers that do not block input
- Synchronized open tabs, sessions, recovery data, and LSP paths after file changes
- Useful keyboard, mouse, SSH, ASCII, monochrome, and narrow-terminal workflows

## Coding

- Incremental tree-sitter parsing
- Five officially supported coding languages
- Automatic LSP startup
- Completion
- Diagnostics
- Navigation
- Rename
- Formatting
- Code actions

## User Experience

- Clear first-run onboarding
- Searchable command palette
- Searchable help
- Mouse and keyboard workflows
- Conventional, Caret, and Vim-inspired profiles
- Understandable settings interface
- Platform-aware help and shortcuts
- Consistent semantic themes across editor, explorer, manager, terminal, and overlays

## Platform Support

- First-class Windows support
- First-class macOS support
- First-class Linux support
- Reliable SSH operation
- Reliable tmux operation where supported
- Platform-specific packaging and troubleshooting
- No major feature gaps between operating systems

## Distribution

- Official Windows binaries and installer
- Official macOS Apple Silicon binaries
- Official macOS Intel binaries while supported
- Official Linux binaries
- Automated release pipeline
- Checksums
- Installation documentation
- Winget and Scoop support
- Homebrew support
- `.deb` and RPM support

## Extensibility

- Versioned extension protocol
- Extension isolation and timeouts
- Documented examples
- Stable configuration format

## Documentation

- Installation guide
- Five-minute tutorial
- Editor and file-manager workflow guide
- Keybinding reference
- Settings reference
- LSP troubleshooting
- Extension documentation
- Terminal compatibility guide
- Windows guide
- macOS guide
- Linux guide
- Contributing guide
- Architecture overview

---

# Features That Should Wait Until After 1.0

| Feature | Recommendation |
|---|---|
| Built-in debugger | After 1.0 |
| AI coding assistant | After 1.0 |
| Remote workspace system | After 1.0 |
| Extension marketplace | After 1.0 |
| Collaborative editing | Much later |
| Complete Vim compatibility | Not a primary goal |
| Emacs-style programmability | Not a goal |
| Terminal multiplexer replacement | Not a goal |
| Graphical desktop version | Separate future product decision |
| Dozens of official LSP languages | Add gradually after the first five are reliable |
| Archive browsing as a virtual filesystem | After the core manager is stable |
| Full media preview suite | After the preview API is stable |
| Remote filesystem providers | After 1.0 and only with strong reliability boundaries |

---

# Recommended Immediate GitHub Milestones

## Milestone: 0.6 Reliability

1. Implement and verify atomic save and permission preservation
2. Add crash-recovery journal
3. Add session restoration
4. Preserve CRLF, LF, BOM, and final-newline state
5. Add external-change comparison workflow
6. Add structured logging and `caret doctor`
7. Split application state into focused modules
8. Add persistence failure tests
9. Add Unicode and long-line regression tests
10. Add cross-platform path tests
11. Add platform-specific configuration-directory handling
12. Establish release-blocking bug severity levels

## Milestone: 0.7 Everyday Editing and Project Navigation

1. Create the cached explorer model and directory snapshots
2. Add background scan, Git-status, metadata, and preview workers
3. Add request and generation IDs for stale-result rejection
4. Add debounced filesystem watching and targeted invalidation
5. Redesign the project-explorer header and row presentation
6. Add semantic explorer theme roles and ASCII/Unicode/Nerd icon modes
7. Add inline create and rename workflows
8. Add the responsive full file-manager workspace
9. Add parent/current/preview panes and responsive layout rules
10. Add selection, range selection, invert selection, and selection status
11. Add safe copy, cut, paste, move, duplicate, trash, and delete operations
12. Add operation progress, cancellation, and conflict-resolution dialogs
13. Synchronize moved and renamed paths with tabs, sessions, recovery, history, and LSP
14. Add text, code, directory, structured-data, and binary metadata previews
15. Introduce reusable UI layout and widget boundaries
16. Add explorer and file-manager settings
17. Add fuzzy file opener and recent-file integration
18. Add complete find-and-replace panel
19. Add project-wide search and replacement preview
20. Improve multi-cursor editing
21. Add user-editable keybindings and searchable settings
22. Strengthen document undo architecture
23. Add large-directory, watcher, symlink, permission, and operation-failure tests
24. Add Windows, macOS, Linux, SSH, tmux, narrow-terminal, and monochrome smoke tests

## Milestone: 0.8 Coding

1. Add persistent incremental tree-sitter documents
2. Create a data-driven language registry
3. Automatically detect and start LSP servers
4. Capture and display LSP stderr
5. Add LSP restart and logs
6. Add Python support
7. Add Go support
8. Add JavaScript and TypeScript support
9. Add signature help
10. Add robust position-encoding support
11. Add incremental document synchronization
12. Build per-language integration tests
13. Add platform-aware executable detection
14. Keep explorer path operations synchronized with LSP workspaces

## Milestone: 0.9 Distribution

1. Add Windows release packaging
2. Add Winget and Scoop packages
3. Add macOS Apple Silicon build
4. Add macOS Intel build while practical
5. Add Homebrew formula
6. Add macOS signing and notarization workflow
7. Add Linux tarball
8. Add `.deb` and RPM packages
9. Add installation smoke tests
10. Add upgrade and uninstall tests
11. Add release checksums and SBOM generation

---

# Release Discipline

Every release should have one central promise:

| Release | Promise |
|---|---|
| 0.6 | Caret will not lose your work |
| 0.7 | Caret is comfortable for everyday editing, project navigation, and file management |
| 0.8 | Caret understands your code automatically |
| 0.9 | Anyone can install and learn Caret on Windows, macOS, or Linux |
| 0.10 | Caret can be extended safely |
| 1.0 Beta | Caret is being proven under real use |
| 1.0 | Caret is a dependable cross-platform terminal development environment |

For each issue, require:

```text
User problem:
Proposed behavior:
Out of scope:
Acceptance criteria:
Automated tests:
Manual test:
Documentation:
Supported platforms:
Platform-specific notes:
Performance and cancellation behavior:
Accessibility behavior:
```

---

# Final Product Test

Caret is ready to compete when a new user can:

1. Install it without a toolchain
2. Open a project
3. Understand the interface
4. Navigate a polished project explorer
5. Find, preview, organize, and safely operate on files
6. Edit using familiar controls
7. Search the whole project
8. Open the terminal and run the project
9. See coding errors and completions automatically
10. Recover from a crash
11. Continue working without configuring Caret first
12. Have the same dependable experience on Windows, macOS, Linux, SSH, and tmux

That is the product Caret should become.

Not another Neovim.

Not a smaller Emacs.

Not a clone of Helix, Yazi, or Superfile.

**A friendly, lightweight, polished, cross-platform terminal development environment that works immediately.**
