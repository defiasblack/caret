use std::{fs, io, path::PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::theme::ThemeKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeKind,
    pub tab_width: usize,
    pub show_line_numbers: bool,
    pub tree_width: usize,
    pub show_hidden_files: bool,
    pub restore_session: bool,
    pub max_search_results: usize,
    pub format_on_save: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeKind::Oxide,
            tab_width: 4,
            show_line_numbers: true,
            tree_width: 40,
            show_hidden_files: false,
            restore_session: true,
            max_search_results: 500,
            format_on_save: false,
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = toml::to_string_pretty(settings)
        .map_err(|error| io::Error::other(format!("config serialization failed: {error}")))?;
    fs::write(path, contents)
}

pub fn config_path() -> PathBuf {
    ProjectDirs::from("com", "Caret", "Caret")
        .map(|directories| directories.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("caret-config.toml"))
}
