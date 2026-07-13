# Accessibility and terminal compatibility

Caret is a keyboard-first terminal application. This audit records the
accessibility behavior that is implemented, the terminal assumptions it makes,
and the compatibility checks required for releases.

## Accessibility baseline

- Every mouse action has a keyboard equivalent. Context menus, theme and keymap
  galleries, the dashboard, file tree, tabs, dialogs, and command palette can
  all be operated without a mouse.
- Focus and selection use text markers, bold text, and color. Meaning is never
  conveyed by color alone: Git changes use `+`, `~`, and `-`; notifications use
  an icon and message; LSP state is written as text.
- The Monochrome theme provides a high-contrast fallback. Use `:theme mono` or
  launch with the standard `NO_COLOR` environment variable.
- `:set reducedmotion` disables the animated LSP activity indicator. The
  `CARET_REDUCED_MOTION=1` environment variable enables the same behavior for a
  session. Use `:set noreducedmotion` to restore animation.
- Dynamic content is confined to stable status and notification rows, and
  rendering uses synchronized terminal updates to reduce flicker.
- Wide Unicode characters and tabs are measured in terminal cells. A regression
  test covers double-width text truncation.
- Caret refuses layouts smaller than 44 columns by 8 rows and shows a readable
  explanation instead of drawing corrupted controls.

Terminal screen readers generally expose terminal output as a flat character
grid. Caret therefore keeps mode, focus, background state, errors, and action
hints as visible text. There is no terminal-standard semantic accessibility API
for richer control roles.

## Keyboard-only smoke test

For each release, verify the following without using a mouse:

1. Launch `caret`, select a recent project, open Help, and return to the dashboard.
2. Move between the file tree and editor with `Ctrl-E`; open and close tabs.
3. Open the command palette with `Ctrl-Shift-P`, filter it, and run a command.
4. Open `:themes` and `:keymaps`; select and cancel with arrows, Enter, and Esc.
5. Open context menus with a mouse once, then operate them entirely with arrows,
   Enter, and Esc. (Terminals do not provide a portable keyboard context-menu
   event, so the same actions also remain available through shortcuts/commands.)
6. Trigger save, warning, and error notifications and confirm their icon, text,
   and color remain distinguishable in Monochrome.

## Compatibility matrix

| Environment | Input/render path | Expected support | Release check |
|---|---|---:|---|
| Windows Terminal + PowerShell | ConPTY, true color, mouse | Full | Manual smoke test |
| Windows Terminal + OpenSSH | ConPTY/PTY, true color, mouse forwarding | Full | Manual SSH smoke test |
| Linux xterm-compatible terminal | PTY, ANSI/true color, mouse | Full | Manual smoke test |
| macOS Terminal / iTerm2 | PTY, ANSI/true color, mouse | Full | Manual smoke test |
| tmux 3.x | Nested PTY; mouse must be enabled/passed through | Full | Manual smoke test |
| 256-color terminal | RGB colors are terminal-quantized | Functional | Monochrome recommended |
| Mouse-disabled SSH client | Keyboard path only | Full keyboard operation | Keyboard-only test above |
| `TERM=dumb` / non-interactive pipe | No cursor-addressable TUI | Unsupported | Use `caret --help` only |

Automated tests cover terminal layouts at 44x8, 80x24, 120x30, and 200x60,
plus Unicode cell-width truncation. Manual checks remain necessary because PTY,
mouse, Alt-key, clipboard, font, and color behavior belongs to the terminal
emulator rather than Caret.

## Known limitations

- Terminal clipboard access can be unavailable in headless SSH sessions; Caret
  keeps an internal copy buffer as a fallback.
- Some terminal or shell configurations reserve Alt-key combinations. Equivalent
  commands and clickable controls remain available.
- A font without box-drawing or symbol glyphs may show fallback boxes. The
  Monochrome theme improves contrast but does not replace the terminal font.
