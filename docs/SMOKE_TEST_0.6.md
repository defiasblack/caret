# Milestone 0.6 manual smoke test

Run this checklist on Windows, macOS, and Linux before a 0.6 release. Use a
disposable copy of a project; the forced-termination checks intentionally stop
the editor without allowing normal cleanup.

1. Open a UTF-8 file with CRLF endings and a BOM. Edit and save it. Confirm the
   BOM, CRLF endings, and final-newline state are unchanged.
2. Start editing a named file, wait at least two seconds, then forcibly stop
   Caret (Task Manager on Windows, `kill -9` on macOS/Linux). Restart Caret and
   confirm the recovery notice lists the filename and timestamp. Use
   `:recovercompare 1`, then `:recover 1`; verify the unsaved text and cursor
   position return. Run `:discardrecovery` after verification.
3. Edit a file in Caret, modify or delete it from another program, then return
   to Caret. Confirm that save is blocked until you explicitly choose Reload,
   Keep/Overwrite, or Compare. Verify Compare does not modify either version.
4. Edit a file, begin saving a large change, and forcibly stop Caret. Confirm
   the original file is either intact or the complete replacement—not a
   truncated or partial file.
5. Open two files, set a split, move both cursors, hide/show the sidebar, then
   quit normally. Restart Caret and verify tabs, active tab, cursors, scroll
   positions, split, sidebar state, and project root restore. Confirm no
   terminal process is restored.
6. Run `caret doctor`, then run `:doctor` and `:copydiagnostics`; confirm the
   report includes OS, terminal, shell, configuration, recovery, log, and
   clipboard capability information.
