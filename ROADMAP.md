# Caret Roadmap

# This roadmap describes the project's current direction. It is not a promise of specific features or delivery dates, and priorities may change based on user feedback, maintenance needs, and contributor availability.

## Near-term priorities

- Improve reliability across Windows, Linux, macOS, and SSH sessions
- Expand automated tests for editing, navigation, tabs, project-tree behavior, and terminal integration
- Make installation and upgrades easier with documented release artifacts
- Improve error messages and recovery when files, shells, language servers, or plugins fail
- Continue accessibility work for keyboard-only, monochrome, and reduced-motion use
- Keep documentation, built-in help, and command discovery synchronized

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

---

## Competitive Position

Caret does **not** need to beat Neovim, Emacs, Helix, or Micro in every category.

Caret needs to become the best choice for this user:

> “I want to edit code in the terminal, browse my project, run commands, get completions and errors, and use familiar keyboard shortcuts without configuring an entire development environment.”

### Where Caret Should Win

| Category | Caret’s Intended Advantage |
|---|---|
| Installation | One straightforward installation with no configuration required |
| First launch | Friendly dashboard, clear commands, visible shortcuts |
| Windows | Excellent Windows Terminal, PowerShell, and ConPTY support |
| macOS | Excellent Terminal, iTerm2, Apple Silicon, Intel, zsh, and Homebrew support |
| Linux | Excellent support across major distributions, shells, terminals, SSH, and tmux |
| Project editing | File tree, fuzzy file opening, project search, integrated terminal |
| Coding support | Automatic LSP setup with useful failure explanations |
| Input | Mouse, conventional shortcuts, and optional modal navigation |
| Discoverability | Command palette, contextual hints, searchable help, and settings |
| Configuration | Good defaults with optional, understandable customization |
| SSH usage | Complete keyboard workflow and dependable terminal rendering |
| Safety | Atomic saves, recovery, conflict handling, and session restoration |

---

# Strategic Rules

## 1. Stability Before Feature Count

Caret already has enough visible features to demonstrate the product. New subsystems should temporarily take a back seat to making existing systems trustworthy.

## 2. Zero Configuration Must Be the Default

A configuration file may improve Caret, but it should never be required to make Caret useful.

## 3. Every Important Action Must Be Discoverable

Important actions should have at least two of these entry points:

- Keyboard shortcut
- Command palette entry
- Typed command
- Visible or clickable interface control

## 4. Windows, macOS, and Linux Are First-Class Platforms

No platform should be treated as a secondary build target.

A feature is not complete until it has been checked for:

- Windows Terminal and PowerShell
- macOS Terminal and iTerm2
- Common Linux terminals
- SSH sessions
- tmux where relevant
- Clipboard-disabled and mouse-disabled environments
- Platform-specific file paths and shell behavior

## 5. Caret Mode Is the Primary Experience

Caret can retain Conventional and Vim-inspired profiles, but the product should not become trapped trying to reproduce all of Vim.

## 6. No Feature Is Complete Without Tests and Documentation

A merged feature is not finished until it has:

- Automated tests where practical
- Error handling
- Help text
- Keyboard access
- Documentation
- Release notes
- Platform verification

---

# Platform Support Policy

## Windows

Officially support:

- Windows 11
- Windows Server 2022 where practical
- Windows Terminal
- PowerShell
- Command Prompt
- OpenSSH sessions
- ConPTY
- Standard Windows clipboard behavior
- Drive-letter and UNC paths
- CRLF files

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

## Platform Parity Rule

No release should advertise a feature as complete unless:

- Core behavior works on Windows, macOS, and Linux
- Platform-specific failures produce useful messages
- CI builds all supported platforms
- Manual smoke tests cover the release’s major workflows
- Documentation includes platform-specific notes where needed

---

# Milestone 0.6 — Trustworthy Foundation

## Objective

Make Caret safe enough that users can trust it with important files.

This is the highest-priority milestone. Do not add major user-facing subsystems until it is complete.

## Data Safety

Implement:

### Atomic File Saving

- Write changes to a temporary file in the same directory
- Flush and synchronize the temporary file
- Preserve file permissions where possible
- Rename the completed temporary file over the original
- Never truncate the original before the replacement is ready
- Handle Windows rename semantics correctly
- Handle macOS and Linux permission behavior correctly

### Crash-Recovery Journal

- Periodically record unsaved buffer changes
- Detect recovery data on startup
- Show filename, timestamp, and available actions
- Allow users to recover, compare, or discard the recovery version
- Store recovery files in the correct platform-specific application data directory

### Session Restoration

Restore:

- Open projects
- Tabs
- Active tab
- Cursor positions
- Scroll positions
- Split layout
- Sidebar state

Do not silently restore terminal processes.

### External-Change Conflict Handling

- Clearly distinguish reload, overwrite, and compare
- Never overwrite a changed disk file without confirmation
- Handle deleted or renamed files gracefully
- Handle network-mounted files and delayed timestamps carefully

### File-Format Handling

- Detect and preserve LF versus CRLF
- Correctly handle UTF-8 BOM files
- Detect binary files before displaying them as text
- Show a useful error for unsupported encodings
- Preserve final-newline state
- Avoid changing line endings unless the user requests it

## Architecture

Break central application logic into focused systems:

```text
src/
  app/
    state.rs
    events.rs
    update.rs
  commands/
    registry.rs
    editor.rs
    project.rs
    workspace.rs
  document/
    buffer.rs
    history.rs
    persistence.rs
    recovery.rs
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
    editor.rs
    overlays.rs
    widgets.rs
```

The exact names may change, but responsibilities should no longer be concentrated in one application object.

## Diagnostics and Support

Add:

- Structured log file
- `caret doctor`
- Version, terminal, operating system, shell, and configuration report
- LSP process output and errors
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
- Windows paths
- macOS paths
- Linux paths
- Shell detection
- Platform-specific application data directories

## Exit Criteria

Version 0.6 is complete only when:

- Killing Caret during a save cannot destroy the original file
- An unsaved document can be restored after forced termination
- External disk changes cannot be overwritten accidentally
- Windows, macOS, and Linux CI pass
- All current editor features still work after architecture changes
- Caret can be used for sustained editing without known data-loss defects
- Core workflows have been manually smoke-tested on all three platforms

---

# Milestone 0.7 — Excellent Everyday Editing

## Objective

Make Caret substantially more comfortable than a basic terminal editor for normal daily work.

## File Navigation

Implement:

- Fuzzy file opener
- Recently opened files
- Recently opened projects
- File tree filtering
- Reveal current file
- Create, rename, move, duplicate, and delete
- Delete confirmation with clear path
- `.gitignore` support
- Hidden-file toggle
- Symlink handling
- File operation undo where practical
- Correct handling of Windows drive roots
- Correct handling of macOS and Linux symlinks
- Correct handling of case-sensitive and case-insensitive file systems

Suggested default:

```text
Ctrl-P — Open file by name
```

## Find and Replace

Implement:

- Find in current file
- Replace one
- Replace all
- Case sensitivity toggle
- Whole-word toggle
- Regular-expression mode
- Search history
- Search within selection
- Project-wide search
- Project-wide replacement with preview

Project search should show:

- File
- Line number
- Matching text
- Search options
- Keyboard navigation
- Mouse navigation

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

Replace whole-document snapshot history with operation-oriented history that supports:

- Efficient large-document undo
- Grouped typing operations
- Selection restoration
- Multi-cursor restoration
- Clear undo boundaries
- Configurable history limits
- Optional persistent undo later

## Keymaps

Support three official profiles:

| Profile | Purpose |
|---|---|
| Caret | Recommended balanced workflow |
| Conventional | Familiar shortcuts similar to graphical editors |
| Vim-inspired | Modal navigation without claiming complete Vim compatibility |

Add:

- User-defined bindings
- Key conflict detection
- Searchable keybinding browser
- Reset-to-default option
- Per-profile help
- Clear warning when a terminal intercepts a shortcut
- Platform-aware defaults for macOS modifier keys
- Documentation for Ctrl, Alt, Option, and Command differences

Do not attempt complete Vim operator, register, and text-object compatibility before 1.0.

## Settings Experience

Create a real settings interface:

- Search settings
- Change values without editing TOML manually
- Explain each setting
- Show the default value
- Validate before saving
- Indicate whether restart is required
- Provide “Open settings file” for advanced users
- Store settings in standard platform-specific configuration locations

## Exit Criteria

Version 0.7 is complete when a user can comfortably:

1. Open a project
2. Find files
3. Search the project
4. Edit across several tabs
5. Use selections and multi-cursors
6. Run commands
7. Save safely
8. Understand available actions without external documentation
9. Complete these workflows on Windows, macOS, and Linux

---

# Milestone 0.8 — Zero-Configuration Coding

## Objective

Turn Caret into a useful code editor rather than only a capable text editor.

## Rebuild Tree-Sitter Integration

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

Create a data-driven language registry:

```toml
name = "Rust"
extensions = ["rs"]
comment_token = "//"
language_id = "rust"
lsp_command = "rust-analyzer"
formatter = "rustfmt"
```

Language support should not require hard-coded branches throughout the application.

## Initial Official Languages

Concentrate on five excellent language experiences:

1. Rust
2. C#
3. Python
4. Go
5. JavaScript and TypeScript

Existing syntax support for configuration formats can remain, but these five should receive complete coding workflows.

Do not advertise dozens of languages until they are genuinely tested.

## Automatic LSP Behavior

Replace manual `:lsp` startup with:

- Automatic language detection
- Automatic server detection
- Automatic startup when appropriate
- Visible startup and indexing state
- Server restart
- Server stop
- Detailed error information
- Server output logs
- Per-project disable option
- Per-language configuration
- Platform-aware executable discovery
- Correct PATH handling on Windows, macOS, and Linux
- Shell-independent process launching

When a server is unavailable, Caret should say something useful:

```text
Python language support is not installed.

Recommended server: basedpyright

[View installation instructions] [Disable for Python] [Not now]
```

## Coding Features

Stabilize:

- Completion
- Completion documentation
- Hover information
- Go to definition
- Find references
- Rename symbol
- Code actions
- Diagnostics
- Document formatting
- Format selection
- Format on save
- Document symbols
- Workspace symbols
- Signature help

## Diagnostics Interface

Provide:

- Gutter markers
- Inline indicators
- Problems panel
- Filter by severity
- Jump to next or previous problem
- Clear source and message
- Quick-fix access
- Source name, such as compiler or language server

## LSP Correctness

Support:

- UTF-8, UTF-16, and UTF-32 position encodings
- Incremental document synchronization
- Workspace folders
- Server requests
- Cancellation
- Graceful restart
- Multiple simultaneous language servers
- Configuration requests
- Progress notifications
- Workspace edits
- Correct snippet handling
- Platform-specific process and path behavior

## Exit Criteria

Version 0.8 is complete when:

- Opening a supported project automatically activates available coding support
- No `:lsp` command is required for the normal workflow
- A missing server produces actionable guidance
- Server crashes do not crash Caret
- Diagnostics, navigation, and completion work consistently in all five official languages
- Tree-sitter updates incrementally
- Supported language workflows work on Windows, macOS, and Linux

---

# Milestone 0.9 — Installation, Packaging, and Onboarding

## Objective

Make installing and learning Caret easier than configuring another terminal editor.

## Windows Distribution

Provide:

- Standalone x64 executable
- Installer
- PATH integration
- Start-menu shortcut that opens Windows Terminal
- Winget package
- Scoop package
- Checksums
- Signed binaries when feasible

## macOS Distribution

Provide:

- Apple Silicon binary
- Intel binary while practical
- Universal binary if practical
- Homebrew formula
- Signed release artifacts
- Notarization when feasible
- Clear handling of Gatekeeper and quarantine prompts
- Checksums
- Simple uninstall instructions

## Linux Distribution

Provide:

- Standalone x86_64 binary
- ARM64 binary where practical
- Tar archive
- `.deb` package
- RPM package
- AppImage if it provides real value
- AUR package or community packaging guidance
- Nix package or community packaging guidance
- Installation shell script
- Checksums
- Clear uninstall instructions

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
2. Ask them to choose Caret, Conventional, or Vim-inspired keys
3. Let them preview themes
4. Explain `Ctrl-P`, `Ctrl-Shift-P`, `Ctrl-E`, `Ctrl-S`, and `F1`
5. Explain macOS modifier-key differences where relevant
6. Offer to open the current directory
7. Provide a short interactive tutorial
8. Make every step skippable

## Help System

The help system should include:

- Search
- Categories
- Current keymap
- Platform-specific key display
- Clickable commands
- Examples
- Troubleshooting
- LSP setup
- Terminal compatibility
- Settings explanations
- Clipboard and SSH guidance

## Exit Criteria

Version 0.9 is complete when:

- A new user can install Caret without building it
- Installation correctly places Caret on PATH
- First launch explains the essential workflow
- Caret can diagnose common terminal, clipboard, shell, and LSP failures
- Installation, upgrade, and removal are tested on Windows, macOS, and Linux
- Release artifacts are available for all three platforms

---

# Milestone 0.10 — Controlled Extensibility

## Objective

Allow useful customization without turning Caret into a plugin-management project.

## Positioning

Call the current system **extensions** unless and until it exposes a broad, stable editor API.

Caret does not need to compete with Neovim’s plugin ecosystem before 1.0.

## Extension Protocol

Create a versioned protocol:

```toml
api_version = 1
name = "Example Extension"
version = "1.0.0"
```

Support controlled capabilities:

- Register command
- Read active document
- Read selection
- Replace selection
- Apply document edits
- Open file
- Show notification
- Add language definition
- Add theme
- Run before or after save
- Run on file open

## Reliability Requirements

- Extensions run outside the editor process
- A failed extension cannot crash Caret
- Long-running extensions can be cancelled
- Timeouts are enforced
- Output and errors are logged
- Caret explains why an extension failed
- Protocol compatibility is checked
- Extension actions are applied as one undoable edit
- Extension execution never blocks keyboard input
- Extension launching works consistently across Windows, macOS, and Linux

## Security

Because extensions can execute programs, Caret must clearly display:

- Extension source
- Executable being launched
- Requested capabilities
- Project or user scope
- Trust status
- Platform-specific path and permission information

Do not build an online extension marketplace before the protocol is stable.

## Official Examples

Ship examples for:

- Sort JSON
- Format selected text
- Insert timestamp
- Add custom language comment rules
- Add a theme
- Run a project-specific command

## Exit Criteria

Version 0.10 is complete when:

- The protocol is documented and versioned
- Broken extensions cannot freeze or crash Caret
- Extension edits participate in undo
- Error reporting is understandable
- At least five official examples are tested in CI
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
- No remote development platform
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
- Project search
- Syntax parsing
- LSP startup
- Memory usage

Initial targets:

- No perceptible input lag during ordinary editing
- Near-zero CPU use while idle
- Responsive editing of files containing at least 100,000 lines
- Large-file mode that disables expensive coding features when necessary
- Project scanning that does not freeze the interface
- LSP and Git operations that never block text input

## Compatibility Testing

Manually test:

### Windows

- Windows Terminal with PowerShell
- Windows Terminal with Command Prompt
- Windows Terminal over SSH
- Windows Server where practical

### macOS

- macOS Terminal
- iTerm2
- Apple Silicon
- Intel where supported
- zsh
- Homebrew installation
- SSH sessions

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

### Shared Scenarios

- Narrow terminals
- Mouse-disabled terminals
- Clipboard-disabled SSH sessions
- 256-color terminals
- Unicode fonts
- Long-running editing sessions
- External file changes
- Shell restarts
- Network-mounted projects

## Dogfooding

Use Caret itself for real development work.

Track:

- Crashes
- Lost cursor positions
- Failed saves
- Broken undo
- Rendering corruption
- Incorrect selections
- Terminal key conflicts
- LSP hangs
- High CPU or memory use
- Platform-specific inconsistencies
- Workflows that require escaping to another editor

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
- No known data-loss bugs

## Editing

- Excellent basic editing
- Find and replace
- Project search
- Fuzzy file opening
- Multiple selections
- Multi-cursor support
- Tabs and splits
- Configurable keymaps

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

## Platform Support

- First-class Windows support
- First-class macOS support
- First-class Linux support
- Reliable SSH operation
- Reliable tmux operation where supported
- Platform-specific packaging
- Platform-specific troubleshooting
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

---

# Recommended Immediate GitHub Milestones

## Milestone: 0.6 Reliability

Create these issues first:

1. Implement atomic save and permission preservation
2. Add crash-recovery journal
3. Add session restoration
4. Preserve CRLF, LF, BOM, and final-newline state
5. Add external-change comparison workflow
6. Add structured logging and `caret doctor`
7. Split application state into focused modules
8. Add persistence failure tests
9. Add Unicode and long-line regression tests
10. Add Windows, macOS, and Linux path tests
11. Add platform-specific configuration-directory handling
12. Establish release-blocking bug severity levels

## Milestone: 0.7 Everyday Editing

1. Add fuzzy file opener
2. Add complete find-and-replace panel
3. Add project-wide search
4. Add project replacement preview
5. Improve multi-cursor editing
6. Add user-editable keybindings
7. Add searchable settings
8. Add command and keybinding search
9. Strengthen undo architecture
10. Add file-operation confirmations and tests
11. Add macOS key display rules
12. Add case-sensitive and case-insensitive filesystem tests

## Milestone: 0.8 Coding

1. Add persistent incremental tree-sitter documents
2. Create data-driven language registry
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
14. Add Windows, macOS, and Linux LSP test environments

## Milestone: 0.9 Distribution

1. Add Windows release packaging
2. Add Winget package
3. Add Scoop package
4. Add macOS Apple Silicon build
5. Add macOS Intel build while practical
6. Add Homebrew formula
7. Add macOS signing and notarization workflow
8. Add Linux tarball
9. Add `.deb` package
10. Add RPM package
11. Add installation smoke tests
12. Add upgrade and uninstall tests
13. Add release checksums and SBOM generation

---

# Release Discipline

Every release should have one central promise:

| Release | Promise |
|---|---|
| 0.6 | Caret will not lose your work |
| 0.7 | Caret is comfortable for everyday editing |
| 0.8 | Caret understands your code automatically |
| 0.9 | Anyone can install and learn Caret on Windows, macOS, or Linux |
| 0.10 | Caret can be extended safely |
| 1.0 Beta | Caret is being proven under real use |
| 1.0 | Caret is a dependable cross-platform terminal code editor |

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
```

---

# Final Product Test

Caret is ready to compete when a new user can:

1. Install it without a toolchain
2. Open a project
3. Understand the interface
4. Find a file
5. Edit using familiar controls
6. Search the whole project
7. Open the terminal and run the project
8. See coding errors and completions automatically
9. Recover from a crash
10. Continue working without configuring Caret first
11. Have the same dependable experience on Windows, macOS, and Linux

That is the product Caret should become.

Not another Neovim.

Not a smaller Emacs.

Not a clone of Helix.

**A friendly, lightweight, cross-platform terminal development environment that works immediately.**
