# Milestone 0.6 manual smoke test

Run this checklist on Windows, macOS, and Linux before a 0.6 release. Use a
disposable copy of a project; the forced-termination checks intentionally stop
the editor without allowing normal cleanup.

The automated suite now covers the durable save failure paths, BOM/line-ending
handling, Unicode cell positioning, external-change protection, invalid
configuration, session serialization, platform replacement, LSP stderr
capture, and real-PTY edit/save/quit, repeated-save, external-conflict,
forced-termination, recovery, and re-save workflows. The checklist below
remains the human release sign-off for platform terminal and filesystem
differences that automation can miss.

Test only candidate `0.6.0-rc.2`, code commit
`bf5883d1a5b8e83a2b3843e9d7b5620a91262c2f`, which passed
[CI run 30183710766](https://github.com/defiasblack/caret/actions/runs/30183710766).

1. Open a UTF-8 file with CRLF endings and a BOM. Edit and save it. Confirm the
   BOM, CRLF endings, and final-newline state are unchanged.
2. Start editing a named file, wait at least two seconds, then forcibly stop
   Caret (Task Manager on Windows, `kill -9` on macOS/Linux). Restart Caret and
   confirm the recovery notice lists the filename and timestamp. Open a
   different file, use `:recovercompare 1`, then `:recover 1`; verify Caret
   switches to the snapshot's recorded file and restores its unsaved text and
   cursor without modifying the unrelated file. Run `:discardrecovery` after
   verification.
3. Edit a file in Caret, modify or delete it from another program, then return
   to Caret. Confirm that save is blocked until you explicitly choose Reload,
   Keep/Overwrite, or Compare. Verify Compare does not modify either version.
4. Repeat the conflict test with multiple dirty tabs and `:wa`. Confirm the
   changed disk file is not overwritten. Run `:w other-existing-file` and
   confirm Save As refuses to replace it; use `:w! other-existing-file` only
   after checking the destination and confirm the explicit overwrite succeeds.
5. Start two Caret processes, create unsaved work in both, wait for both to
   checkpoint, then forcibly stop both. Start a third Caret process and confirm
   both recovery snapshots are listed and recoverable.
6. Edit a file, begin saving a large change, and forcibly stop Caret. Confirm
   the original file is either intact or the complete replacement—not a
   truncated or partial file.
7. Open two files, set a split, move both cursors, hide/show the sidebar, then
   quit normally. Restart Caret and verify tabs, active tab, cursors, scroll
   positions, split, sidebar state, and project root restore. Confirm no
   terminal process is restored.
8. In the project tree, attempt to create, copy, move, or rename onto an
   existing destination. Confirm the destination is unchanged. Attempt to
   permanently delete an open dirty file and confirm Caret refuses.
9. Open a project through a slow or network-mounted directory where practical.
   Expand directories, edit a file externally, and save. Confirm the UI remains
   usable and delayed metadata never causes an implicit overwrite.
10. Run `caret doctor`, then run `:doctor` and `:copydiagnostics`; confirm the
   report includes OS, terminal, shell, configuration, recovery, log, and
   clipboard capability information.

## Sign-off

Record the exact release-candidate commit. A blank or failed row keeps milestone
0.6 open.

| Platform | OS version | Terminal/filesystem | Commit | Tester/date | Result |
|---|---|---|---|---|---|
| Windows |  |  |  |  | Pending |
| macOS |  |  |  |  | Pending |
| Linux |  |  |  |  | Pending |
