# Visual viewer validation

The viewer is read-only and uses the existing document-opening routes. PDFium
runs only in `--visual-worker`; image decoders and terminal encoding run there
too. The editor owns at most one helper and accepts a result only when the
complete request (including generation) still matches. PDFium is a pinned,
optional runtime; image decoding is linked into the executable.

Windows checks on 2026-10-25:

- `cargo fmt --all -- --check`
- `cargo clippy --offline --locked --all-targets --all-features -- -D warnings`
- `cargo test --offline --locked --all-targets --all-features`: 316 unit tests
  and 11 integration tests passed; five unrelated optional/manual tests ignored.
- Actual helper tests load a two-page PDF, distinguish portrait and landscape
  page dimensions/graphics, clamp page requests, and encode Sixel, Kitty, and
  color cells. Images cover alpha composition, panning, invalid data, oversized
  input, and excessive decoded pixel dimensions.
- Unit tests check aspect ratio, viewport/zoom budgets, cancellation/deadline
  reap, palette/RLE/chunk encoding, reload generations, stale-result rejection,
  control-character sanitization, and retention of dirty editor buffers.
- PTYs cover source editing/selection, existing Office loading, PDF page/zoom
  controls, image colors, reload failures, and return to editing. A Windows
  inbox ConPTY can strip Sixel data even when the installed Windows Terminal
  supports it. Windows PTYs therefore validate the color-cell PDF path; native
  Sixel output is checked directly through helper and drawing tests. Unix PTYs
  exercise the Sixel path.

The CI matrix installs the same verified PDFium runtime and runs checks on
Windows, Linux, and macOS. Source users without PDFium can run ordinary tests;
native PDF assertions report an explicit skip locally and fail if missing in CI.

Manual visual sign-off remains useful for terminal fonts/DPI, graphics
passthrough through multiplexers, and physical display appearance. The installed
computer-use skill forbids automating terminal apps, so no live-window visual
verification is claimed. Terminal pixel reporting is used where available;
`CARET_CELL_PIXELS=WIDTHxHEIGHT` overrides the documented 8x16 fallback.
