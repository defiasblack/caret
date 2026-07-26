# Caret architecture

Caret keeps the terminal event loop in `app.rs`, but reliability-sensitive
responsibilities live behind focused boundaries:

- `src/document.rs` owns text decoding, file-format metadata, fingerprints, and
  the durable write protocol. Conditional writes verify the expected
  fingerprint immediately before replacement, so polling and saving cannot
  silently race an external edit.
- `src/platform/` owns OS-specific file replacement, shell selection, and
  application/configuration locations. Windows uses `MoveFileExW` with
  replacement and write-through flags; Unix uses same-directory rename.
- `src/editor.rs` owns buffer editing, cursor/display coordinates, undo history,
  external-file fingerprints, and save-format behavior.
- `src/recovery.rs` owns crash-journal serialization and per-process-instance
  journals. One corrupt journal is reported without hiding valid snapshots
  written by another Caret process.
- `src/session.rs` owns workspace/session serialization and deliberately has no
  terminal-process state.
- `src/diagnostics.rs` owns structured JSONL logging, diagnostic reports, and
  support paths.
- `src/lsp.rs` owns LSP framing, URI/path conversion, and server stderr capture.
- `src/explorer.rs` owns background explorer requests, stale-result rejection,
  Git-status snapshots, refresh throttling, and application of completed work on
  the UI thread. `src/project.rs` remains the synchronous in-memory tree model
  and only requests background refreshes; it never launches Git itself.
- `src/app/persistence.rs` owns application save/quit policy, while
  `src/app/settings.rs` owns settings inspection and validated `:set` changes.
- `src/file_ops.rs` is the no-replace safety boundary for the current
  synchronous project-tree mutations. The non-blocking 0.7 operation service
  will replace it without weakening those invariants.
- `src/ui.rs` renders state and does not perform persistence itself.

The application object remains the coordinator for user events and background
work. New data-loss-sensitive behavior must be added to the focused lower
boundary first, with a failure-path test before the UI calls it.
