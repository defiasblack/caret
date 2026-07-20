# Caret architecture

Caret keeps the terminal event loop in `app.rs`, but reliability-sensitive
responsibilities live behind focused boundaries:

- `src/document.rs` owns text decoding, file-format metadata, fingerprints, and
  the durable write protocol.
- `src/platform/` owns OS-specific file replacement, shell selection, and
  application/configuration locations. Windows uses `MoveFileExW` with
  replacement and write-through flags; Unix uses same-directory rename.
- `src/editor.rs` owns buffer editing, cursor/display coordinates, undo history,
  external-file fingerprints, and save-format behavior.
- `src/recovery.rs` owns the crash journal and recovery serialization.
- `src/session.rs` owns workspace/session serialization and deliberately has no
  terminal-process state.
- `src/diagnostics.rs` owns structured JSONL logging, diagnostic reports, and
  support paths.
- `src/lsp.rs` owns LSP framing, URI/path conversion, and server stderr capture.
- `src/app/persistence.rs` owns application save/quit policy, while
  `src/app/settings.rs` owns settings inspection and validated `:set` changes.
- `src/ui.rs` renders state and does not perform persistence itself.

The application object remains the coordinator for user events and background
work. New data-loss-sensitive behavior must be added to the focused lower
boundary first, with a failure-path test before the UI calls it.
