# Caret Roadmap

This list tracks Caret features that remain after the completed productivity
milestone.

## Editing


## Coding intelligence

- Language Server Protocol integration: completion, hover, definitions,
  references, rename, formatting, code actions, and diagnostics

## Project workflow

- Git status, gutter indicators, diff viewer, and basic stage/history actions
- Integrated terminal pane

## Customization and ecosystem

- Context menus for files, tabs, and selected text
- Plugin API for commands, languages, themes, and editor behavior

## Polish

- Recent-project dashboard
- Better progress, notification, and background-task status UI
- Accessibility audit and terminal compatibility test matrix

## Known bugs

- C# `:def` returns no definition in Caret even though the same `csharp-ls`
  request succeeds against the loaded solution in an isolated protocol test.
