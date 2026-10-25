//! Save, settings persistence, and quit safety at the application boundary.

use std::path::PathBuf;

use super::{App, Mode};

impl App {
    pub(super) fn persist_settings(&mut self) {
        // Tests must never overwrite the user's real configuration file.
        if cfg!(test) {
            return;
        }
        if let Err(error) = crate::config::save(&self.settings) {
            self.message = format!("Could not save settings: {error}");
        }
    }

    pub(super) fn save_all(&mut self) {
        self.save_all_internal();
    }

    pub(super) fn save_all_internal(&mut self) -> bool {
        let (saved, errors) = self.editor.save_all();
        let _ = self.project.refresh();
        self.project.request_git_refresh();

        if errors.is_empty() {
            self.message = if saved == 0 {
                "All named tabs are already saved".to_string()
            } else {
                format!("Saved {saved} tab(s)")
            };
            true
        } else {
            self.message = format!(
                "Saved {saved}; {} tab(s) failed: {}",
                errors.len(),
                errors.join("; ")
            );
            false
        }
    }

    pub(super) fn save(&mut self) {
        self.save_internal();
    }

    pub(super) fn save_as(&mut self, path: PathBuf, overwrite_confirmed: bool) {
        let result = if overwrite_confirmed {
            self.editor.save_as_overwrite(&path)
        } else {
            self.editor.save_as(&path)
        };
        match result {
            Ok(()) => {
                let hooks = match self.run_save_hooks() {
                    Ok(count) => count,
                    Err(error) => {
                        self.message = error;
                        return;
                    }
                };
                let _ = self.project.refresh();
                self.project.request_git_refresh();
                self.refresh_git_line_changes();
                self.message = if hooks == 0 {
                    format!("Saved {}", path.display())
                } else {
                    format!("Saved {} · running {hooks} plugin hook(s)", path.display())
                };
                if self.plugin_task.is_none() {
                    self.request_formatting_after_save();
                }
            }
            Err(error) => self.message = format!("Save failed: {error}"),
        }
    }

    pub(super) fn save_internal(&mut self) -> bool {
        if !self.editor.has_pending_external_change() && self.editor.changed_on_disk() {
            // Check again at the point of saving instead of relying only on
            // the background poll. This closes the race where a file changes
            // immediately before Ctrl-S.
            self.editor.keep_disk_change();
        }
        if self.editor.has_pending_external_change() {
            self.pending_save_after_disk_change = true;
            self.mode = Mode::ReloadConfirm;
            self.message = "Disk changes were kept — [R] Reload   [K] Overwrite with current buffer   [C] Compare   [Esc] Cancel".to_string();
            return false;
        }
        match self.editor.save() {
            Ok(()) => {
                let hooks = match self.run_save_hooks() {
                    Ok(count) => count,
                    Err(error) => {
                        self.message = error;
                        return false;
                    }
                };
                let _ = self.project.refresh();
                self.project.request_git_refresh();
                self.refresh_git_line_changes();
                let name = self
                    .editor
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "[No Name]".to_string());
                self.message = if hooks == 0 {
                    format!("Saved {name}")
                } else {
                    format!("Saved {name} · running {hooks} plugin hook(s)",)
                };
                if self.plugin_task.is_none() {
                    self.request_formatting_after_save();
                }
                true
            }
            Err(error) => {
                self.message = format!("Save failed: {error}");
                false
            }
        }
    }

    pub(super) fn request_quit(&mut self, force: bool) {
        if !force && self.plugin_task.as_ref().is_some_and(|task| task.save) {
            self.plugin_quit_pending = true;
            self.message = "Finishing save hooks before quitting…".to_string();
            return;
        }
        if force || !self.editor.any_dirty() {
            self.finish_quit();
            return;
        }

        let dirty = self.editor.dirty_titles();
        self.explorer_focused = false;
        self.mode = Mode::QuitConfirm;
        self.message = format!("{} unsaved tab(s): {}", dirty.len(), dirty.join(", "));
    }

    pub(super) fn finish_quit(&mut self) {
        self.plugin_worker.cancel_and_wait();
        self.plugin_task = None;
        self.plugin_quit_pending = false;
        let session = self
            .settings
            .restore_session
            .then(|| self.current_session_state());
        if let Err(error) = self.persistence_worker.finish(session) {
            let _ = crate::diagnostics::append(
                "persistence",
                &format!("final checkpoint failed: {error}"),
            );
        }
        self.should_quit = true;
    }
}
