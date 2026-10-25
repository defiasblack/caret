//! Asynchronous plugin actions and document-bound save-hook sequencing.
use super::*;

impl App {
    pub(super) fn plugin_context(&self, arguments: Vec<String>, event: &str) -> PluginContext {
        PluginContext {
            project: self.project.root.display().to_string(),
            file: self
                .editor
                .path
                .as_ref()
                .map(|path| path.display().to_string()),
            language: self.language_name(),
            text: String::new(),
            selection: None,
            cursor_line: self.editor.cursor.line,
            cursor_column: self.editor.cursor.column,
            arguments,
            event: event.to_string(),
        }
    }

    pub(super) fn run_plugin_command(&mut self, name: &str, arguments: Vec<String>, event: &str) {
        if let Err(error) = self.start_plugin_task(vec![name.to_string()], arguments, event, false)
        {
            self.message = error;
        }
    }

    pub(super) fn start_plugin_task(
        &mut self,
        commands: Vec<String>,
        arguments: Vec<String>,
        event: &str,
        save: bool,
    ) -> Result<(), String> {
        if self.plugin_task.is_some() {
            return Err("A plugin is already running; retry when it finishes".to_string());
        }
        self.plugin_task = Some(PluginTask {
            document: self.editor.stamp().0,
            path: self.editor.path.clone(),
            stamp: self.editor.stamp(),
            cursor: self.editor.cursor,
            anchor: self.editor.selection_anchor,
            remaining: commands.into(),
            arguments,
            event: event.to_string(),
            save,
            registry: self.plugins.clone(),
        });
        self.dispatch_plugin_task();
        Ok(())
    }

    pub(super) fn dispatch_plugin_task(&mut self) {
        let Some(mut task) = self.plugin_task.take() else {
            return;
        };
        let Some(index) = self.editor.index_for_document(task.document) else {
            return;
        };
        let previous = self.editor.active_index();
        self.editor.select(index);
        task.stamp = self.editor.stamp();
        task.cursor = self.editor.cursor;
        task.anchor = self.editor.selection_anchor;
        if let Some(name) = task.remaining.pop_front() {
            let context = self.plugin_context(task.arguments.clone(), &task.event);
            self.plugin_worker.submit(crate::plugin::PluginRun {
                registry: task.registry.clone(),
                name: name.clone(),
                context,
                text: self.editor.snapshot_buffer(),
                selection: self.editor.selection_range(),
            });
            self.message = format!("Running plugin {name}…");
        }
        self.editor.select(previous);
        self.plugin_task = Some(task);
    }

    pub(super) fn poll_plugin(&mut self) -> bool {
        let Some(result) = self.plugin_worker.poll() else {
            return false;
        };
        let Some(task) = self.plugin_task.take() else {
            return false;
        };
        let pending_quit = std::mem::take(&mut self.plugin_quit_pending);
        let Some(index) = self.editor.index_for_document(task.document) else {
            self.message = "Plugin result ignored: originating document was closed".to_string();
            return true;
        };
        let editor = self.editor.editor_at(index).unwrap();
        if editor.stamp() != task.stamp || editor.path != task.path {
            self.message =
                "Plugin result ignored: document changed while plugin was running".to_string();
            return true;
        }
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                self.message = error;
                return true;
            }
        };
        if (response.replace_selection.is_some() || response.insert_text.is_some())
            && (editor.cursor != task.cursor || editor.selection_anchor != task.anchor)
        {
            self.message = "Plugin result ignored: cursor or selection changed".to_string();
            return true;
        }
        let previous = self.editor.active_index();
        self.editor.select(index);
        let opens_document = response.open.is_some();
        self.apply_plugin_response(response);
        if !opens_document {
            self.editor.select(previous);
        }
        if task.save && task.remaining.is_empty() {
            let index = self.editor.index_for_document(task.document).unwrap();
            let previous = self.editor.active_index();
            self.editor.select(index);
            let saved = if self.editor.dirty {
                self.editor.save()
            } else {
                Ok(())
            };
            self.editor.select(previous);
            if let Err(error) = saved {
                self.message = format!("Plugin save hook failed to save changes: {error}");
                return true;
            }
        }
        if !task.remaining.is_empty() {
            self.plugin_task = Some(task);
            self.plugin_quit_pending = pending_quit;
            self.dispatch_plugin_task();
        } else {
            if task.save {
                self.message = "Saved document and completed plugin save hooks".to_string();
                if let Some(index) = self.editor.index_for_document(task.document) {
                    let previous = self.editor.active_index();
                    self.editor.select(index);
                    if self
                        .editor
                        .path
                        .as_ref()
                        .and_then(|path| lsp::server_for_extension(path))
                        .is_some_and(|server| Some(server.id) == self.lsp_server_id)
                    {
                        self.request_formatting_after_save();
                    }
                    self.editor.select(previous);
                }
            }
            if pending_quit {
                self.request_quit(false);
            }
        }
        true
    }

    pub(super) fn apply_plugin_response(&mut self, response: PluginResponse) {
        if let Some(text) = response.replace_document {
            self.editor.replace_text(&text);
        } else if let Some(text) = response.replace_selection {
            if self.editor.selection_range().is_some() {
                self.editor.checkpoint();
                self.editor.delete_selection();
                self.editor.insert_text(&text);
            } else {
                self.message =
                    "Plugin requested replace_selection, but nothing is selected".to_string();
            }
        } else if let Some(text) = response.insert_text {
            self.editor.checkpoint();
            self.editor.delete_selection();
            self.editor.insert_text(&text);
        }

        if let Some(path) = response.open {
            let path = PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else {
                self.project.root.join(path)
            };
            match self.editor.open_or_switch(&path) {
                Ok(_) => {
                    self.explorer_focused = false;
                    self.mode = self.preferred_editor_mode();
                    self.after_tab_switch();
                }
                Err(error) => {
                    self.message = format!("Plugin could not open {}: {error}", path.display())
                }
            }
        }
        if let Some(message) = response.message {
            self.message = message;
        }
    }

    pub(super) fn run_save_hooks(&mut self) -> Result<usize, String> {
        let commands = self.plugins.on_save_commands();
        let count = commands.len();
        if count > 0 {
            self.start_plugin_task(commands, Vec::new(), "on_save", true)?;
        }
        Ok(count)
    }
}
