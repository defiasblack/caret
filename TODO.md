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


## Milestone 0.7 — Ergonomics

- [x] Configurable startup view (`startup = session | folder | empty | dashboard`); the default now opens the current folder's file tree instead of always showing the welcome dashboard, which is reachable on demand via `:welcome`.

## Known bugs

- C# `:def` returns no definition in Caret even though the same `csharp-ls`
  request succeeds against the loaded solution in an isolated protocol test.
