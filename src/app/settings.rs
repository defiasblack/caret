//! Settings inspection and validated runtime configuration changes.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{App, Mode};
use crate::{config, document::FinalNewline};

impl App {
    pub(super) fn open_settings_browser(&mut self) {
        self.settings_browser_input.clear();
        self.settings_browser_selected = 0;
        self.settings_browser_scroll = 0;
        self.mode = Mode::SettingsBrowser;
        self.message =
            "Settings · type to search · Enter inspects · Esc closes · edit with :set".to_string();
    }

    pub(super) fn handle_settings_browser(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = self.preferred_editor_mode();
                self.message.clear();
            }
            KeyCode::Enter => {
                if let Some(setting) = self.setting_rows().get(self.settings_browser_selected) {
                    self.message = format!(
                        "{} = {} · {}",
                        setting.name, setting.current, setting.validation
                    );
                }
            }
            KeyCode::Up => {
                self.settings_browser_selected = self.settings_browser_selected.saturating_sub(1)
            }
            KeyCode::Down => {
                self.settings_browser_selected = (self.settings_browser_selected + 1)
                    .min(self.setting_rows().len().saturating_sub(1));
            }
            KeyCode::PageUp => {
                self.settings_browser_selected = self.settings_browser_selected.saturating_sub(8)
            }
            KeyCode::PageDown => {
                self.settings_browser_selected = (self.settings_browser_selected + 8)
                    .min(self.setting_rows().len().saturating_sub(1));
            }
            KeyCode::Home => self.settings_browser_selected = 0,
            KeyCode::End => {
                self.settings_browser_selected = self.setting_rows().len().saturating_sub(1)
            }
            KeyCode::Backspace => {
                self.settings_browser_input.pop();
                self.settings_browser_selected = 0;
                self.settings_browser_scroll = 0;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.settings_browser_input.push(character);
                self.settings_browser_selected = 0;
                self.settings_browser_scroll = 0;
            }
            _ => {}
        }
        self.ensure_settings_browser_visible();
    }

    pub fn setting_rows(&self) -> Vec<config::SettingInfo> {
        let query = self.settings_browser_input.to_ascii_lowercase();
        self.settings
            .setting_infos()
            .into_iter()
            .filter(|setting| {
                query.is_empty()
                    || setting.name.contains(&query)
                    || setting.current.to_ascii_lowercase().contains(&query)
                    || setting.default.to_ascii_lowercase().contains(&query)
                    || setting.description.to_ascii_lowercase().contains(&query)
                    || setting.validation.to_ascii_lowercase().contains(&query)
            })
            .collect()
    }

    fn ensure_settings_browser_visible(&mut self) {
        let count = self.setting_rows().len();
        if count == 0 {
            self.settings_browser_selected = 0;
            self.settings_browser_scroll = 0;
            return;
        }
        self.settings_browser_selected = self.settings_browser_selected.min(count - 1);
        if self.settings_browser_selected < self.settings_browser_scroll {
            self.settings_browser_scroll = self.settings_browser_selected;
        } else if self.settings_browser_selected >= self.settings_browser_scroll + 8 {
            self.settings_browser_scroll = self.settings_browser_selected + 1 - 8;
        }
    }

    pub(super) fn execute_set(&mut self, argument: &str) {
        match argument {
            "number" | "nu" => {
                self.settings.show_line_numbers = true;
                self.apply_editor_settings();
                self.persist_settings();
                self.message = "Line numbers enabled".to_string();
            }
            "nonumber" | "nonu" => {
                self.settings.show_line_numbers = false;
                self.apply_editor_settings();
                self.persist_settings();
                self.message = "Line numbers disabled".to_string();
            }
            "hidden" | "showhidden" => {
                self.settings.show_hidden_files = true;
                self.project.show_hidden = true;
                match self.project.refresh() {
                    Ok(()) => {
                        self.persist_settings();
                        self.message = "Hidden files shown".to_string();
                    }
                    Err(error) => self.message = format!("Refresh failed: {error}"),
                }
            }
            "nohidden" | "noshowhidden" => {
                self.settings.show_hidden_files = false;
                self.project.show_hidden = false;
                match self.project.refresh() {
                    Ok(()) => {
                        self.persist_settings();
                        self.message = "Hidden files hidden".to_string();
                    }
                    Err(error) => self.message = format!("Refresh failed: {error}"),
                }
            }
            "restoresession" => {
                self.settings.restore_session = true;
                self.persist_settings();
                self.message = "Session restoration enabled (next launch)".to_string();
            }
            "norestoresession" => {
                self.settings.restore_session = false;
                self.persist_settings();
                self.message = "Session restoration disabled (next launch)".to_string();
            }
            "formatonsave" | "fos" => {
                self.settings.format_on_save = true;
                self.persist_settings();
                self.message = "Format on save enabled".to_string();
            }
            "noformatonsave" | "nofos" => {
                self.settings.format_on_save = false;
                self.persist_settings();
                self.message = "Format on save disabled".to_string();
            }
            "reducedmotion" | "reduce-motion" => {
                self.settings.reduced_motion = true;
                self.persist_settings();
                self.message = "Reduced motion enabled".to_string();
            }
            "noreducedmotion" | "no-reduce-motion" => {
                self.settings.reduced_motion = false;
                self.persist_settings();
                self.message = "Reduced motion disabled".to_string();
            }
            "autoindent" | "ai" => {
                self.settings.auto_indent = true;
                self.apply_editor_settings();
                self.persist_settings();
                self.message = "Auto-indent enabled".to_string();
            }
            "noautoindent" | "noai" => {
                self.settings.auto_indent = false;
                self.apply_editor_settings();
                self.persist_settings();
                self.message = "Auto-indent disabled".to_string();
            }
            "trimonsave" => {
                self.settings.trim_trailing_whitespace_on_save = true;
                self.apply_editor_settings();
                self.persist_settings();
                self.message = "Trailing whitespace is trimmed on save".to_string();
            }
            "notrimonsave" => {
                self.settings.trim_trailing_whitespace_on_save = false;
                self.apply_editor_settings();
                self.persist_settings();
                self.message = "Trailing whitespace is kept on save".to_string();
            }
            value if value.starts_with("finalnewline=") => {
                let choice = value.split_once('=').map(|(_, value)| value);
                let policy = match choice {
                    Some("preserve") => Some(FinalNewline::Preserve),
                    Some("always") => Some(FinalNewline::Always),
                    Some("strip") => Some(FinalNewline::Strip),
                    _ => None,
                };
                match policy {
                    Some(policy) => {
                        self.settings.final_newline = policy;
                        self.apply_editor_settings();
                        self.persist_settings();
                        self.message = format!("Final newline on save: {}", policy.name());
                    }
                    None => {
                        self.message =
                            "Final newline must be preserve, always, or strip".to_string()
                    }
                }
            }
            value if value.starts_with("undolimit=") => {
                let number = value
                    .split_once('=')
                    .and_then(|(_, value)| value.parse::<usize>().ok());
                match number {
                    Some(number @ 10..=100_000) => {
                        self.settings.undo_history_limit = number;
                        self.apply_editor_settings();
                        self.persist_settings();
                        self.message = format!("Undo history limit: {number} steps");
                    }
                    _ => {
                        self.message = "Undo limit must be between 10 and 100000 steps".to_string()
                    }
                }
            }
            value if value.starts_with("maxsearchresults=") => {
                let number = value
                    .split_once('=')
                    .and_then(|(_, value)| value.parse::<usize>().ok());
                match number {
                    Some(number @ 50..=100_000) => {
                        self.settings.max_search_results = number;
                        self.persist_settings();
                        self.message = format!("Maximum search results: {number}");
                    }
                    _ => {
                        self.message =
                            "Maximum search results must be between 50 and 100000".to_string()
                    }
                }
            }
            value if value.starts_with("tabstop=") || value.starts_with("ts=") => {
                let number = value
                    .split_once('=')
                    .and_then(|(_, value)| value.parse::<usize>().ok());

                match number {
                    Some(number @ 1..=16) => {
                        self.settings.tab_width = number;
                        self.apply_editor_settings();
                        self.persist_settings();
                        self.message = format!("Tab width: {number}");
                    }
                    _ => self.message = "Tab width must be between 1 and 16".to_string(),
                }
            }
            value if value.starts_with("treewidth=") => {
                let number = value
                    .split_once('=')
                    .and_then(|(_, value)| value.parse::<usize>().ok());

                match number {
                    Some(number @ 22..=80) => {
                        self.project.width = number;
                        self.settings.tree_width = number;
                        self.persist_settings();
                        self.message = format!("Tree width: {number}");
                    }
                    _ => self.message = "Tree width must be between 22 and 80".to_string(),
                }
            }
            value if value.starts_with("startup=") => {
                let choice = value.split_once('=').map(|(_, value)| value);
                let view = match choice {
                    Some("session") => Some(crate::config::StartupView::Session),
                    Some("folder") => Some(crate::config::StartupView::Folder),
                    Some("empty") => Some(crate::config::StartupView::Empty),
                    Some("dashboard") => Some(crate::config::StartupView::Dashboard),
                    _ => None,
                };
                match view {
                    Some(view) => {
                        self.settings.startup = view;
                        self.persist_settings();
                        self.message = format!(
                            "Startup view: {} (applies on next launch)",
                            choice.unwrap_or_default()
                        );
                    }
                    None => {
                        self.message =
                            "Startup must be session, folder, empty, or dashboard".to_string()
                    }
                }
            }
            _ => {
                self.message =
                    "Try :set number, :set startup=folder, :set formatonsave, or :set tabstop=4"
                        .to_string()
            }
        }
    }
}
