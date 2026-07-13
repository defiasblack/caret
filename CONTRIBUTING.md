# Contributing to Caret

Thank you for helping improve Caret. Contributions of all sizes are welcome, including bug reports, documentation fixes, tests, accessibility improvements, and new editor features.

## Before you begin

- Search existing issues and pull requests before opening a duplicate.
- Use the bug-report or feature-request form when creating an issue.
- For security vulnerabilities, follow [SECURITY.md](SECURITY.md) instead of opening a public issue.
- Keep each pull request focused on one clear change whenever practical.

## Development setup

Caret is written in Rust and uses Cargo.

1. Install the current stable Rust toolchain with `rustup`.
2. Install the formatting and linting components:

   ```bash
   rustup component add rustfmt clippy
   ```

3. Fork and clone the repository:

   ```bash
   git clone https://github.com/YOUR-USERNAME/caret.git
   cd caret
   ```

4. Build and run Caret:

   ```bash
   cargo build
   cargo run -- .
   ```

5. Build an optimized binary when needed:

   ```bash
   cargo build --release
   ```

## Making changes

Create a descriptive branch from `main`:

```bash
git switch -c feature/short-description
```

Good branch prefixes include `feature/`, `fix/`, `docs/`, `test/`, and `chore/`.

When changing behavior:

- Add or update tests where practical.
- Update user-facing documentation and keyboard-command tables.
- Consider Linux, Windows, macOS, SSH, mouse, and keyboard-only use.
- Avoid unrelated formatting or refactoring in the same pull request.
- Preserve existing keymap profiles unless the change intentionally updates them.

## Required checks

Run these commands before submitting a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features
cargo test --all-targets --all-features
cargo build --release
```

Fix formatting errors and investigate new Clippy warnings. If a warning must remain, explain why in the pull request.

## Commit messages

Use short, clear, imperative commit messages. Examples:

```text
Fix cursor movement after tab close
Add Python comment toggling
Document plugin save hooks
```

Conventional Commit prefixes such as `fix:`, `feat:`, and `docs:` are welcome but not required.

## Pull requests

A good pull request includes:

- A clear explanation of what changed and why
- A link to any related issue
- Reproduction steps for bug fixes
- Testing performed and platforms tested
- Screenshots or terminal recordings for visible UI changes when useful
- Documentation updates for user-facing behavior

Draft pull requests are welcome for work in progress. Mark the pull request ready for review once the implementation and required checks are complete.

## Reporting bugs

Please include:

- Caret version or commit
- Operating system and version
- Terminal emulator and shell
- Exact steps to reproduce the problem
- Expected and actual behavior
- Relevant logs, screenshots, or recordings

Remove passwords, tokens, private paths, and other sensitive information before posting.

## Feature requests

Describe the user problem first, then the proposed solution. Explain alternatives you considered and how the feature fits Caret's goal of being a polished, approachable terminal editor.

## Code of Conduct

By participating, you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
