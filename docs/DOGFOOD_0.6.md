# Milestone 0.6 dogfooding record

Milestone 0.6 is not complete until Caret has been used for sustained real
editing with no unresolved data-loss defect. Automated PTY and failure-path
tests are prerequisites, not substitutes for this gate.

## Entry requirements

- Record the exact release-candidate commit.
- Confirm its complete Windows, macOS, and Linux CI run is green.
- Use disposable branches or backed-up projects until the pass is complete.
- Stop the pass immediately for a partial, missing, silently overwritten, or
  unrecoverable document. Record the defect and add a regression test with the
  fix before restarting the pass.

## Minimum pass

- At least one meaningful editing session in a real terminal on Windows,
  macOS, and Linux.
- At least eight total hours and five sessions across the three platforms.
- Exercise normal saves, Save As, multiple tabs, splits, project search,
  external-change handling, session restart, forced-termination recovery,
  terminal use, and at least one read-only or permission-failure path.
- Inspect saved files and recovery/session behavior rather than relying only on
  status messages.

## Session log

| Platform | OS/terminal/filesystem | Commit | Date/tester | Duration | Workflows exercised | Defects |
|---|---|---|---|---|---|---|
| Windows |  |  |  |  |  | Pending |
| macOS |  |  |  |  |  | Pending |
| Linux |  |  |  |  |  | Pending |
| Additional |  |  |  |  |  |  |
| Additional |  |  |  |  |  |  |

## Exit decision

- Total recorded time: **0 hours**
- Total recorded sessions: **0**
- Unresolved data-loss defects: **unknown until the pass is performed**
- Gate result: **Pending**

The result may be changed to Pass only after the minimum pass is met and every
discovered data-loss defect is resolved and covered by a regression test.
