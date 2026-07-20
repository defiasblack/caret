use std::{fs, io, path::PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::document::FinalNewline;
use crate::theme::ThemeKind;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeymapProfile {
    #[default]
    Caret,
    Vim,
    Conventional,
}

impl KeymapProfile {
    pub const ALL: [Self; 3] = [Self::Caret, Self::Vim, Self::Conventional];

    pub fn name(self) -> &'static str {
        match self {
            Self::Caret => "Caret",
            Self::Vim => "Vim",
            Self::Conventional => "Conventional",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Caret => "Insert-first with fast Normal-mode commands",
            Self::Vim => "Modal editing; files open in Normal mode",
            Self::Conventional => "Always typing; familiar Ctrl shortcuts",
        }
    }
}

/// What Caret shows when it launches without a file or directory argument.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartupView {
    /// Reopen the previous session if one exists, otherwise fall back to `Folder`.
    Session,
    /// Open the current directory's file tree (default).
    #[default]
    Folder,
    /// Open a single empty, unsaved buffer.
    Empty,
    /// Show the recent-projects welcome dashboard.
    Dashboard,
}

impl StartupView {
    pub fn name(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Folder => "folder",
            Self::Empty => "empty",
            Self::Dashboard => "dashboard",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeKind,
    pub keymap: KeymapProfile,
    pub tab_width: usize,
    pub show_line_numbers: bool,
    pub tree_width: usize,
    pub show_hidden_files: bool,
    pub restore_session: bool,
    pub startup: StartupView,
    pub recent_projects: Vec<PathBuf>,
    pub recent_files: Vec<PathBuf>,
    pub reduced_motion: bool,
    pub custom_theme: Option<String>,
    pub max_search_results: usize,
    pub format_on_save: bool,
    pub auto_indent: bool,
    pub trim_trailing_whitespace_on_save: bool,
    pub final_newline: FinalNewline,
    pub undo_history_limit: usize,
    /// Custom key bindings: action id → chord, e.g. `find = "ctrl+g"`.
    pub custom_keys: std::collections::BTreeMap<String, String>,
}

/// Describes one setting for the searchable settings browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingInfo {
    pub name: &'static str,
    pub current: String,
    pub default: String,
    pub description: &'static str,
    pub validation: &'static str,
    pub restart_required: bool,
}

impl Settings {
    pub fn setting_infos(&self) -> Vec<SettingInfo> {
        vec![
            SettingInfo {
                name: "theme",
                current: self
                    .custom_theme
                    .as_deref()
                    .unwrap_or_else(|| self.theme.name())
                    .to_string(),
                default: "oxide".to_string(),
                description: "Color palette; use :theme or :themes to change it",
                validation: "built-in or plugin theme name",
                restart_required: false,
            },
            SettingInfo {
                name: "keymap",
                current: self.keymap.name().to_ascii_lowercase(),
                default: "caret".to_string(),
                description: "Editing profile; use :keymap to change it",
                validation: "caret | vim | conventional",
                restart_required: false,
            },
            SettingInfo {
                name: "tabstop",
                current: self.tab_width.to_string(),
                default: "4".to_string(),
                description: "Spaces inserted for a tab or indentation step",
                validation: "integer from 1 to 16",
                restart_required: false,
            },
            SettingInfo {
                name: "number",
                current: on_off(self.show_line_numbers),
                default: "on".to_string(),
                description: "Show line numbers in the editor gutter",
                validation: "on/off via :set number or :set nonumber",
                restart_required: false,
            },
            SettingInfo {
                name: "treewidth",
                current: self.tree_width.to_string(),
                default: "40".to_string(),
                description: "Width of the project tree sidebar",
                validation: "integer from 22 to 80",
                restart_required: false,
            },
            SettingInfo {
                name: "hidden",
                current: on_off(self.show_hidden_files),
                default: "off".to_string(),
                description: "Show hidden files in the project tree",
                validation: "toggle with :set hidden or :set nohidden",
                restart_required: false,
            },
            SettingInfo {
                name: "restoresession",
                current: on_off(self.restore_session),
                default: "on".to_string(),
                description: "Persist and restore tabs, splits, and sidebar state",
                validation: "on/off; applies on next launch",
                restart_required: true,
            },
            SettingInfo {
                name: "startup",
                current: self.startup.name().to_string(),
                default: "folder".to_string(),
                description: "Initial view when Caret opens without a target",
                validation: "session | folder | empty | dashboard",
                restart_required: true,
            },
            SettingInfo {
                name: "recentprojects",
                current: format!("{} saved", self.recent_projects.len()),
                default: "empty".to_string(),
                description: "Recently opened project roots used by :welcome",
                validation: "managed automatically",
                restart_required: false,
            },
            SettingInfo {
                name: "recentfiles",
                current: format!("{} saved", self.recent_files.len()),
                default: "empty".to_string(),
                description: "Recently opened files used by the fuzzy file picker",
                validation: "managed automatically",
                restart_required: false,
            },
            SettingInfo {
                name: "reducedmotion",
                current: on_off(self.reduced_motion),
                default: "off".to_string(),
                description: "Disable animated background activity indicators",
                validation: "on/off via :set reducedmotion",
                restart_required: false,
            },
            SettingInfo {
                name: "customtheme",
                current: self.custom_theme.as_deref().unwrap_or("none").to_string(),
                default: "none".to_string(),
                description: "Active plugin theme override, if one is selected",
                validation: "managed by :theme and plugins",
                restart_required: false,
            },
            SettingInfo {
                name: "maxsearchresults",
                current: self.max_search_results.to_string(),
                default: "500".to_string(),
                description: "Maximum project-search results kept in memory",
                validation: "integer; values below 50 are clamped",
                restart_required: false,
            },
            SettingInfo {
                name: "formatonsave",
                current: on_off(self.format_on_save),
                default: "off".to_string(),
                description: "Run the active language formatter before saving",
                validation: "on/off via :set formatonsave",
                restart_required: false,
            },
            SettingInfo {
                name: "autoindent",
                current: on_off(self.auto_indent),
                default: "on".to_string(),
                description: "Copy indentation when opening or inserting lines",
                validation: "on/off via :set autoindent",
                restart_required: false,
            },
            SettingInfo {
                name: "trimonsave",
                current: on_off(self.trim_trailing_whitespace_on_save),
                default: "off".to_string(),
                description: "Trim trailing whitespace before writing a file",
                validation: "on/off via :set trimonsave",
                restart_required: false,
            },
            SettingInfo {
                name: "finalnewline",
                current: self.final_newline.name().to_string(),
                default: "preserve".to_string(),
                description: "Policy for the final newline when saving",
                validation: "preserve | always | strip",
                restart_required: false,
            },
            SettingInfo {
                name: "undolimit",
                current: self.undo_history_limit.to_string(),
                default: "1000".to_string(),
                description: "Maximum operation groups retained per buffer",
                validation: "integer from 10 to 100000",
                restart_required: false,
            },
            SettingInfo {
                name: "customkeys",
                current: format!("{} custom", self.custom_keys.len()),
                default: "none".to_string(),
                description: "User key overrides configured with :bind",
                validation: "managed by :bind, :unbind, and :bindreset",
                restart_required: false,
            },
        ]
    }
}

fn on_off(value: bool) -> String {
    if value {
        "on".to_string()
    } else {
        "off".to_string()
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeKind::Oxide,
            keymap: KeymapProfile::Caret,
            tab_width: 4,
            show_line_numbers: true,
            tree_width: 40,
            show_hidden_files: false,
            restore_session: true,
            startup: StartupView::Folder,
            recent_projects: Vec::new(),
            recent_files: Vec::new(),
            reduced_motion: false,
            custom_theme: None,
            max_search_results: 500,
            format_on_save: false,
            auto_indent: true,
            trim_trailing_whitespace_on_save: false,
            final_newline: FinalNewline::Preserve,
            undo_history_limit: 1_000,
            custom_keys: std::collections::BTreeMap::new(),
        }
    }
}

pub fn load() -> (Settings, Option<String>) {
    let path = config_path();
    let Ok(contents) = fs::read_to_string(&path) else {
        return (Settings::default(), None);
    };

    match toml::from_str(&contents) {
        Ok(settings) => (settings, None),
        Err(error) => (
            Settings::default(),
            Some(format!("Config ignored: {error}")),
        ),
    }
}

pub fn save(settings: &Settings) -> io::Result<()> {
    let path = config_path();
    let contents = toml::to_string_pretty(settings)
        .map_err(|error| io::Error::other(format!("config serialization failed: {error}")))?;
    crate::document::atomic_write(&path, contents.as_bytes())
}

pub fn config_path() -> PathBuf {
    ProjectDirs::from("com", "Caret", "Caret")
        .map(|directories| directories.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("caret-config.toml"))
}

pub fn plugins_dir() -> PathBuf {
    config_path()
        .parent()
        .map(|parent| parent.join("plugins"))
        .unwrap_or_else(|| PathBuf::from("plugins"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_configs_default_to_caret_keymap() {
        let settings: Settings = toml::from_str("theme = 'nord'").expect("parse settings");
        assert_eq!(settings.keymap, KeymapProfile::Caret);
    }

    #[test]
    fn keymap_profiles_round_trip_through_toml() {
        let settings = Settings {
            keymap: KeymapProfile::Conventional,
            ..Settings::default()
        };
        let encoded = toml::to_string(&settings).expect("encode settings");
        let decoded: Settings = toml::from_str(&encoded).expect("decode settings");
        assert_eq!(decoded.keymap, KeymapProfile::Conventional);
    }

    #[test]
    fn startup_defaults_to_folder_and_round_trips() {
        let settings: Settings = toml::from_str("theme = 'nord'").expect("parse settings");
        assert_eq!(settings.startup, StartupView::Folder);

        let encoded = toml::to_string(&Settings {
            startup: StartupView::Session,
            ..Settings::default()
        })
        .expect("encode settings");
        let decoded: Settings = toml::from_str(&encoded).expect("decode settings");
        assert_eq!(decoded.startup, StartupView::Session);
    }

    #[test]
    fn setting_browser_describes_every_saved_setting() {
        let settings = Settings::default();
        let rows = settings.setting_infos();

        assert_eq!(rows.len(), 19);
        assert_eq!(rows[0].name, "theme");
        assert_eq!(rows[0].default, "oxide");
        assert!(rows
            .iter()
            .any(|row| row.name == "startup" && row.restart_required));
        assert!(rows.iter().any(|row| row.name == "undolimit"));
        assert!(rows.iter().all(|row| !row.description.is_empty()));
        assert!(rows.iter().all(|row| !row.validation.is_empty()));
    }
}
