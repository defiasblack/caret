# Repository repair validation

Validated on 2026-10-09. The changes preserve commands, keybindings, supported
Office formats, configuration compatibility, and the session JSON schema.

## Automated checks

Windows local validation passes formatting, warnings-denied Clippy, all-target
and all-feature tests in debug and release, the real Office helper isolation
test, and five real-PTY smoke tests. The final Windows suite contains 270 passing
unit tests plus six passing integration tests. Two optional LSP tests require
external language servers and remain ignored. Two manual benchmarks are ignored
by the normal suite and were run explicitly in release mode.

The [cross-platform CI run](https://github.com/defiasblack/caret/actions/runs/37896391443)
passes Ubuntu formatting, Clippy and tests, plus Windows, Linux and macOS release
builds, platform tests, PTY smoke tests and command-line diagnostics. The Linux
installer smoke test also passes. CI uses current stable Rust; the allocation
reservation loop also compiles on the locally installed Rust 1.97.

Commands (add a separate target directory when needed):

```text
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --release --locked
cargo test --release --locked --all-targets --all-features
cargo test --release --locked --bin caret benchmark_ -- --ignored --nocapture --test-threads=1
```

Local Windows tests use an isolated `TEMP`/`TMP` directory so file-manager tests
that enumerate the parent of their fixture do not scan unrelated system temp
files. CI's runner temp directories are already isolated. The PTY harness
services ConPTY cursor requests across read boundaries and during file/exit waits;
five consecutive parallel Windows PTY suites passed after this transport repair.

## Regression coverage

- Saved revisions survive undo/save/redo, branching and history eviction;
  save errors do not mark unsaved work clean. A synchronized post-write external
  edit verifies that the saved fingerprint describes the bytes actually written.
- Snapshot tests rewrite the file at a deterministic read boundary and verify
  matching decoded content, BOM/newline metadata and fingerprint. Opening an old
  snapshot cannot adopt a newer disk version as the save baseline.
- Copy tests create a competing destination immediately before installation,
  preserve preexisting directories on failure, cancel mid-file and before
  installation, and verify cleanup ownership. A forced partial source-removal
  failure exercises the cross-device recovery boundary and retains both completed
  destination files.
- Workbook fixtures independently offset values to B3 and formulas to D5.
  Sparse, formula-only and empty sheets, cell/display budgets, narrow viewports,
  paging, resizing, partial-column hit testing and footer clicks have regressions.
- Plugin tests simultaneously send 2 MiB input and drain 1 MiB from each output
  pipe; malformed JSON and blocked-input timeouts have explicit coverage.
  Timed-out children are killed and reaped.
- Session tests cover named tabs interleaved with untitled tabs, active untitled
  fallbacks, both split-pane mappings, missing restored files, cursor clamping,
  and dropping splits containing an untitled or missing pane.
- Deterministic disk-worker channels verify one in-flight request, two-second
  throttling, and rejection of obsolete paths/save generations. Background polling
  continues during input activity.
- Office supervision tests cover cancellation, obsolete generations, deadlines,
  malformed/oversized responses and input rejection before parsing. Integration tests
  invoke the actual helper with malformed archives, oversized inputs, and a sparse
  A1-to-XFD1048576 range that would request billions of dense cells. Allocation
  fails inside the helper before the OS allocation; a subsequent normal load
  succeeds. PTY tests exercise the responsive helper path and viewer close action.

Office's 512 MiB limit covers live Rust allocations and conservatively reserves
old-plus-new allocation size before reallocating. It excludes OS/runtime overhead,
as documented in the README. The helper parses locally without launching Office,
executing macros or calculating formulas. Its process supervisor enforces the
five-second deadline and cancellation independently of parser behavior.

## Release measurements

Windows, optimized builds, identical locked dependencies. A comparison harness
compiled the baseline syntax module from `a6c4a31` and the repaired module with
optimization level 3 and thin LTO. Each syntax sample draws 30 lines 100 times
from a 20,000-line Rust document; parsing/setup is excluded.

| Measurement | Baseline | Repaired |
| --- | ---: | ---: |
| 100 draws near line 0 | 30.317 ms | 29.737 ms |
| 100 draws near line 19,970 | 1,005.886 ms | 32.373 ms |
| 20 fingerprints of a 16 MiB file | 183.869 ms | 109.630 ms |

Deep-document highlighting is approximately 31 times faster in this sample;
its cost is now close to the first viewport, because line offsets are indexed
directly instead of scanning the document prefix. Timing is diagnostic evidence,
not a flaky CI assertion.

The in-repository release benchmarks measured 10,000 coalesced foreground disk
polls at 0.939 ms, with 20 streaming 16 MiB worker hashes at 103.337 ms. Hashing
runs off the UI thread, uses a fixed 64 KiB read buffer, and permits at most one
in-flight observation with bounded request/result channels. Immediate save-time
conflict validation remains synchronous.
