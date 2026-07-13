use std::{fs, io, path::PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

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
    pub recent_projects: Vec<PathBuf>,
    pub reduced_motion: bool,
    pub custom_theme: Option<String>,
    pub max_search_results: usize,
    pub format_on_save: bool,
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
            recent_projects: Vec::new(),
            reduced_motion: false,
            custom_theme: None,
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
}
