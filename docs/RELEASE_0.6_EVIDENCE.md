# Milestone 0.6 release evidence

This is the release gate for the requirements in `ROADMAP.md`. Automated
evidence is kept separate from manual sign-off so CI coverage is never mistaken
for real-terminal validation.

## Automated evidence

| Requirement | Evidence | Status |
|---|---|---|
| Atomic save never exposes a partial replacement | Same-directory temporary file, flush/sync, permission copy, platform replacement, interrupted-save and write-failure tests | Pass |
| External changes are never overwritten implicitly | Conditional fingerprint check immediately before replacement; normal save, Save All, Save As, project replace, plugin, workspace-edit, and formatting paths share protection tests | Pass |
| Forced termination leaves recoverable work | A real PTY session types unsaved work, becomes idle, is forcibly terminated, and a new PTY session discovers, restores, saves, and cleanly exits; a lower-level process test independently checks journal contents | Pass |
| Concurrent Caret instances preserve recovery | Unique process-instance journals are combined; corrupt-journal isolation and legacy-journal tests | Pass |
| File formats remain stable | BOM, LF/CRLF, final-newline, unsupported UTF-8, binary, empty-file, and long-line tests | Pass |
| Sessions restore safe workspace state | Project, tabs, active view, cursor/scroll, split, and sidebar serialization tests; terminal processes are excluded | Pass |
| Platform and filesystem failures are recoverable | Windows replacement, application paths, shell/PTY, symlink loop/broken link, permission-denied, disappearing-root, read-only, deleted-file, and no-replace operation tests | Pass |
| Diagnostics are actionable | Structured log, configuration/OS/terminal/shell/recovery/clipboard report, LSP stderr, and panic-terminal restoration coverage | Pass |
| Supported-platform automation passes | GitHub Actions format, Clippy, full tests, release build, and `caret doctor` on Windows, macOS, and Linux | Must pass on the release-candidate commit |
| Real-terminal core safety paths work | Cross-platform PTY tests cover edit/save/quit, 25 consecutive complete saves, external-change refusal and explicit confirmation, idle checkpointing, forced termination, recovery discovery, restore, and re-save | Must pass on the release-candidate commit |

Local Windows verification on 2026-07-25 must include:

```text
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --release --locked
target\release\caret.exe doctor
```

Result on 2026-07-25: pass — formatting and Clippy were clean, 182 tests
passed with one environment-dependent `rust-analyzer` integration test skipped,
the release build completed, and the release binary produced a valid diagnostic
report. The build used `target\verification` because another running Caret
process held the ordinary release executable open.

## Manual release gates

The milestone remains incomplete until both rows are recorded as passing.

| Gate | Evidence location | Status |
|---|---|---|
| Core and forced-termination workflows pass in real terminals on Windows, macOS, and Linux | Sign-off table in `docs/SMOKE_TEST_0.6.md` | Pending |
| Sustained editing produces no known unresolved data-loss defect | `docs/DOGFOOD_0.6.md` | Pending |

Any data-loss defect reopens this milestone and requires a regression test with
the fix.
