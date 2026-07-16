//! User-configurable key bindings.  Global shortcuts resolve through this
//! registry (defaults + config overrides) before any hard-coded handling,
//! so users can rebind them, conflicts are detected, and the keybinding
//! browser can list every binding truthfully.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::KeymapProfile;

/// Every rebindable editor action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    Save,
    Quit,
    Find,
    Replace,
    ProjectSearch,
    OpenFile,
    Palette,
    Complete,
    CodeActions,
    Undo,
    Redo,
    SelectAll,
    Copy,
    Cut,
    Paste,
    NextTab,
    PrevTab,
    NewTab,
    CloseTab,
    ToggleTree,
    ToggleOutline,
    FocusTree,
    ToggleComment,
    AddCursorAbove,
    AddCursorBelow,
    SelectOccurrences,
    Back,
    Forward,
    Definition,
    References,
    SwitchSplit,
}

impl Action {
    pub const ALL: [Self; 31] = [
        Self::Save,
        Self::Quit,
        Self::Find,
        Self::Replace,
        Self::ProjectSearch,
        Self::OpenFile,
        Self::Palette,
        Self::Complete,
        Self::CodeActions,
        Self::Undo,
        Self::Redo,
        Self::SelectAll,
        Self::Copy,
        Self::Cut,
        Self::Paste,
        Self::NextTab,
        Self::PrevTab,
        Self::NewTab,
        Self::CloseTab,
        Self::ToggleTree,
        Self::ToggleOutline,
        Self::FocusTree,
        Self::ToggleComment,
        Self::AddCursorAbove,
        Self::AddCursorBelow,
        Self::SelectOccurrences,
        Self::Back,
        Self::Forward,
        Self::Definition,
        Self::References,
        Self::SwitchSplit,
    ];

    /// Stable identifier used in the config file and :bind commands.
    pub fn id(self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::Quit => "quit",
            Self::Find => "find",
            Self::Replace => "replace",
            Self::ProjectSearch => "projectsearch",
            Self::OpenFile => "openfile",
            Self::Palette => "palette",
            Self::Complete => "complete",
            Self::CodeActions => "codeaction",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::SelectAll => "selectall",
            Self::Copy => "copy",
            Self::Cut => "cut",
            Self::Paste => "paste",
            Self::NextTab => "nexttab",
            Self::PrevTab => "prevtab",
            Self::NewTab => "newtab",
            Self::CloseTab => "closetab",
            Self::ToggleTree => "toggletree",
            Self::ToggleOutline => "toggleoutline",
            Self::FocusTree => "focustree",
            Self::ToggleComment => "togglecomment",
            Self::AddCursorAbove => "addcursorabove",
            Self::AddCursorBelow => "addcursorbelow",
            Self::SelectOccurrences => "selectoccurrences",
            Self::Back => "back",
            Self::Forward => "forward",
            Self::Definition => "definition",
            Self::References => "references",
            Self::SwitchSplit => "switchsplit",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Save => "Save the current file",
            Self::Quit => "Quit Caret",
            Self::Find => "Find in the current file",
            Self::Replace => "Find and replace in the current file",
            Self::ProjectSearch => "Search the whole project",
            Self::OpenFile => "Open a file by fuzzy name",
            Self::Palette => "Open the command palette",
            Self::Complete => "Request code completion",
            Self::CodeActions => "Request code actions",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::SelectAll => "Select the whole document",
            Self::Copy => "Copy the selection",
            Self::Cut => "Cut the selection",
            Self::Paste => "Paste",
            Self::NextTab => "Next tab",
            Self::PrevTab => "Previous tab",
            Self::NewTab => "New tab",
            Self::CloseTab => "Close the current tab",
            Self::ToggleTree => "Show or hide the file tree",
            Self::ToggleOutline => "Toggle the symbol outline",
            Self::FocusTree => "Focus the file tree / editor",
            Self::ToggleComment => "Toggle line comments",
            Self::AddCursorAbove => "Add a cursor on the line above",
            Self::AddCursorBelow => "Add a cursor on the line below",
            Self::SelectOccurrences => "Select every occurrence",
            Self::Back => "Go back in navigation history",
            Self::Forward => "Go forward in navigation history",
            Self::Definition => "Go to definition",
            Self::References => "Find references",
            Self::SwitchSplit => "Switch between split panes",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|action| action.id() == id.trim().to_ascii_lowercase())
    }
}

/// A key with its modifiers, in normalized form (letters lowercase,
/// BackTab stored as Shift+Tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    /// Normalizes a raw terminal key event for lookup.
    pub fn from_event(key: KeyEvent) -> Self {
        let mut modifiers =
            key.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let code = match key.code {
            KeyCode::BackTab => {
                modifiers |= KeyModifiers::SHIFT;
                KeyCode::Tab
            }
            // Ctrl+Space arrives as Null in several terminals.
            KeyCode::Null => KeyCode::Char(' '),
            KeyCode::Char(character) => KeyCode::Char(character.to_ascii_lowercase()),
            other => other,
        };
        Self { code, modifiers }
    }

    /// Parses "ctrl+shift+p", "alt+enter", "f3", "ctrl+\\", ...
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut modifiers = KeyModifiers::NONE;
        let mut code = None;
        for token in text.split('+').map(str::trim) {
            if token.is_empty() {
                return Err("Empty key name in chord".to_string());
            }
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
                "alt" | "option" | "opt" => modifiers |= KeyModifiers::ALT,
                "shift" => modifiers |= KeyModifiers::SHIFT,
                "cmd" | "command" | "super" | "win" => {
                    return Err(
                        "The terminal reserves Cmd/Win shortcuts; use ctrl, alt, or shift"
                            .to_string(),
                    )
                }
                "tab" => code = Some(KeyCode::Tab),
                "enter" | "return" => code = Some(KeyCode::Enter),
                "space" => code = Some(KeyCode::Char(' ')),
                "up" => code = Some(KeyCode::Up),
                "down" => code = Some(KeyCode::Down),
                "left" => code = Some(KeyCode::Left),
                "right" => code = Some(KeyCode::Right),
                "home" => code = Some(KeyCode::Home),
                "end" => code = Some(KeyCode::End),
                "pageup" => code = Some(KeyCode::PageUp),
                "pagedown" => code = Some(KeyCode::PageDown),
                "backspace" => code = Some(KeyCode::Backspace),
                "delete" | "del" => code = Some(KeyCode::Delete),
                "esc" | "escape" => code = Some(KeyCode::Esc),
                key if key.len() == 1 => {
                    code = Some(KeyCode::Char(key.chars().next().unwrap()));
                }
                key if key.starts_with('f') && key[1..].parse::<u8>().is_ok() => {
                    let number: u8 = key[1..].parse().unwrap();
                    if (1..=12).contains(&number) {
                        code = Some(KeyCode::F(number));
                    } else {
                        return Err(format!("Function keys go from f1 to f12, not {key}"));
                    }
                }
                other => return Err(format!("Unknown key: {other}")),
            }
        }
        let Some(code) = code else {
            return Err("Chord needs a key, for example ctrl+shift+p".to_string());
        };
        Ok(Self { code, modifiers })
    }

    /// Human-readable form, using macOS symbols on macOS.
    pub fn display(&self) -> String {
        self.display_for(cfg!(target_os = "macos"))
    }

    fn display_for(&self, mac: bool) -> String {
        let mut parts = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push(if mac { "⌃" } else { "Ctrl" });
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push(if mac { "⌥" } else { "Alt" });
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push(if mac { "⇧" } else { "Shift" });
        }
        let key = match self.code {
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char(character) => character.to_uppercase().to_string(),
            KeyCode::F(number) => format!("F{number}"),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Up => "Up".to_string(),
            KeyCode::Down => "Down".to_string(),
            KeyCode::Left => "Left".to_string(),
            KeyCode::Right => "Right".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::PageUp => "PageUp".to_string(),
            KeyCode::PageDown => "PageDown".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Delete => "Delete".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            other => format!("{other:?}"),
        };
        if mac {
            format!("{}{key}", parts.concat())
        } else if parts.is_empty() {
            key
        } else {
            format!("{}+{key}", parts.join("+"))
        }
    }
}

/// A chord some terminals swallow before Caret sees it.
pub fn chord_warning(chord: &KeyChord) -> Option<&'static str> {
    let control = chord.modifiers.contains(KeyModifiers::CONTROL);
    let alt = chord.modifiers.contains(KeyModifiers::ALT);
    match chord.code {
        KeyCode::Tab if control => {
            Some("many terminals keep Ctrl+Tab for their own tabs; Alt+N/P also switch tabs")
        }
        KeyCode::Char(' ') if control => {
            Some("tmux prefixes and some input methods intercept Ctrl+Space; :complete also works")
        }
        KeyCode::Char('t' | 'w') if control => {
            Some("browser-hosted terminals may intercept this shortcut")
        }
        KeyCode::Char(character) if alt && character.is_ascii_alphabetic() => {
            Some("macOS Terminal sends Option-letter as text unless configured as Meta")
        }
        KeyCode::F(1) => Some("some terminals open their own help on F1"),
        KeyCode::F(11) => Some("F11 usually toggles the terminal's fullscreen"),
        _ => None,
    }
}

fn default_bindings() -> Vec<(KeyChord, Action)> {
    fn chord(text: &str) -> KeyChord {
        KeyChord::parse(text).expect("default chord parses")
    }
    vec![
        (chord("ctrl+s"), Action::Save),
        (chord("ctrl+q"), Action::Quit),
        (chord("ctrl+f"), Action::Find),
        (chord("ctrl+h"), Action::Replace),
        (chord("ctrl+shift+f"), Action::ProjectSearch),
        (chord("ctrl+p"), Action::OpenFile),
        (chord("ctrl+shift+p"), Action::Palette),
        (chord("ctrl+space"), Action::Complete),
        (chord("ctrl+."), Action::CodeActions),
        (chord("ctrl+z"), Action::Undo),
        (chord("ctrl+y"), Action::Redo),
        (chord("ctrl+a"), Action::SelectAll),
        (chord("ctrl+c"), Action::Copy),
        (chord("ctrl+x"), Action::Cut),
        (chord("ctrl+v"), Action::Paste),
        (chord("ctrl+tab"), Action::NextTab),
        (chord("ctrl+pagedown"), Action::NextTab),
        (chord("alt+n"), Action::NextTab),
        (chord("ctrl+shift+tab"), Action::PrevTab),
        (chord("ctrl+pageup"), Action::PrevTab),
        (chord("alt+p"), Action::PrevTab),
        (chord("ctrl+t"), Action::NewTab),
        (chord("ctrl+w"), Action::CloseTab),
        (chord("ctrl+b"), Action::ToggleTree),
        (chord("ctrl+o"), Action::ToggleOutline),
        (chord("ctrl+e"), Action::FocusTree),
        (chord("ctrl+/"), Action::ToggleComment),
        (chord("ctrl+alt+up"), Action::AddCursorAbove),
        (chord("ctrl+alt+down"), Action::AddCursorBelow),
        (chord("ctrl+shift+l"), Action::SelectOccurrences),
        (chord("alt+left"), Action::Back),
        (chord("alt+right"), Action::Forward),
        (chord("f12"), Action::Definition),
        (chord("shift+f12"), Action::References),
        (chord("ctrl+\\"), Action::SwitchSplit),
    ]
}

/// The resolved binding table: defaults, minus overridden actions, plus the
/// user's custom chords from the config file.
#[derive(Debug, Clone)]
pub struct KeyBindings {
    bindings: Vec<(KeyChord, Action)>,
    custom: BTreeMap<String, String>,
    /// Problems found while applying the config (unknown actions, bad
    /// chords, conflicts).  Shown once at startup.
    pub warnings: Vec<String>,
}

impl KeyBindings {
    pub fn from_custom(custom: &BTreeMap<String, String>) -> Self {
        let mut warnings = Vec::new();
        let mut bindings = default_bindings();

        for (action_id, chord_text) in custom {
            let Some(action) = Action::from_id(action_id) else {
                warnings.push(format!("[keys] unknown action: {action_id}"));
                continue;
            };
            let chord = match KeyChord::parse(chord_text) {
                Ok(chord) => chord,
                Err(error) => {
                    warnings.push(format!("[keys] {action_id}: {error}"));
                    continue;
                }
            };
            if let Some((_, taken)) = bindings
                .iter()
                .find(|(existing, other)| *existing == chord && *other != action)
            {
                warnings.push(format!(
                    "[keys] {} conflicts with {} on {}",
                    action_id,
                    taken.id(),
                    chord.display()
                ));
                continue;
            }
            bindings.retain(|(_, existing)| *existing != action);
            bindings.push((chord, action));
        }

        Self {
            bindings,
            custom: custom.clone(),
            warnings,
        }
    }

    pub fn action_for(&self, key: KeyEvent) -> Option<Action> {
        let chord = KeyChord::from_event(key);
        self.bindings
            .iter()
            .find(|(candidate, _)| *candidate == chord)
            .map(|(_, action)| *action)
    }

    /// The primary chord shown for an action in menus and help.
    pub fn chord_for(&self, action: Action) -> Option<KeyChord> {
        self.bindings
            .iter()
            .find(|(_, candidate)| *candidate == action)
            .map(|(chord, _)| *chord)
    }

    pub fn is_custom(&self, action: Action) -> bool {
        self.custom.contains_key(action.id())
    }

    /// Validates a rebind request.  Returns the parsed chord.
    pub fn validate(&self, action: Action, chord_text: &str) -> Result<KeyChord, String> {
        let chord = KeyChord::parse(chord_text)?;
        if let Some((_, taken)) = self
            .bindings
            .iter()
            .find(|(existing, other)| *existing == chord && *other != action)
        {
            return Err(format!(
                "{} is already bound to {} ({})",
                chord.display(),
                taken.id(),
                taken.description()
            ));
        }
        Ok(chord)
    }
}

/// Non-rebindable keys, listed in the browser for the current profile.
pub fn fixed_keys(profile: KeymapProfile) -> Vec<(&'static str, &'static str)> {
    let mut keys: Vec<(&'static str, &'static str)> = vec![
        ("F1 / ?", "Open help"),
        ("Esc", "Leave insert mode / close panels"),
        ("Tab / Shift+Tab", "Indent / outdent (Normal mode)"),
        ("Alt+Up / Alt+Down", "Move the current line"),
        ("Alt+Shift+Arrows", "Column (rectangle) selection"),
        ("Ctrl+D", "Select the next occurrence"),
        ("Ctrl+J", "Join with the line below"),
        ("Ctrl+Left / Ctrl+Right", "Move by word"),
        ("F2", "Rename symbol (editor) or file (tree)"),
        ("F7", "Duplicate the current line"),
        ("Ctrl+`", "Open or focus the terminal"),
        ("Ctrl+Shift+`", "Close the terminal"),
        ("Alt+1..9", "Jump to tab by number"),
        ("/ or f (in file tree)", "Filter the file tree"),
        (". (in file tree)", "Show hidden and ignored files"),
    ];
    match profile {
        KeymapProfile::Vim | KeymapProfile::Caret => {
            keys.extend([
                ("i / a / o / O", "Enter Insert mode"),
                ("h j k l", "Move the cursor (Normal mode)"),
                ("w / b", "Next / previous word"),
                ("gg / G", "File start / end"),
                ("dd / yy / p", "Delete / yank / paste line"),
                ("u / Ctrl+R", "Undo / redo (Normal mode)"),
                ("/", "Search (Normal mode)"),
                ("n / N", "Next / previous match"),
                ("%", "Jump to matching bracket"),
                ("q{reg} / @{reg}", "Record / replay macro"),
                ("zc zo za zM zR", "Close/open/toggle/all folds"),
            ]);
        }
        KeymapProfile::Conventional => {
            keys.extend([
                ("Typing", "Always inserts text"),
                ("Shift+Arrows", "Select text"),
            ]);
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_parse_and_display_round_trip() {
        let chord = KeyChord::parse("ctrl+shift+p").unwrap();
        assert_eq!(chord.code, KeyCode::Char('p'));
        assert!(chord.modifiers.contains(KeyModifiers::CONTROL));
        assert!(chord.modifiers.contains(KeyModifiers::SHIFT));
        assert_eq!(chord.display_for(false), "Ctrl+Shift+P");
        assert_eq!(chord.display_for(true), "⌃⇧P");

        assert_eq!(KeyChord::parse("f3").unwrap().display_for(false), "F3");
        assert!(KeyChord::parse("cmd+s").is_err());
        assert!(KeyChord::parse("ctrl+f99").is_err());
        assert!(KeyChord::parse("ctrl+").is_err());
    }

    #[test]
    fn events_normalize_backtab_and_letter_case() {
        let bindings = KeyBindings::from_custom(&BTreeMap::new());
        let backtab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::CONTROL);
        assert_eq!(bindings.action_for(backtab), Some(Action::PrevTab));

        let upper = KeyEvent::new(
            KeyCode::Char('L'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(bindings.action_for(upper), Some(Action::SelectOccurrences));
    }

    #[test]
    fn custom_bindings_replace_defaults() {
        let mut custom = BTreeMap::new();
        custom.insert("find".to_string(), "ctrl+g".to_string());
        let bindings = KeyBindings::from_custom(&custom);

        let new_chord = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL);
        assert_eq!(bindings.action_for(new_chord), Some(Action::Find));
        // The old chord no longer triggers the action.
        let old_chord = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL);
        assert_eq!(bindings.action_for(old_chord), None);
        assert!(bindings.is_custom(Action::Find));
        assert!(bindings.warnings.is_empty());
    }

    #[test]
    fn conflicting_bindings_are_rejected_with_a_warning() {
        let mut custom = BTreeMap::new();
        custom.insert("find".to_string(), "ctrl+s".to_string());
        let bindings = KeyBindings::from_custom(&custom);
        assert_eq!(bindings.warnings.len(), 1);
        assert!(
            bindings.warnings[0].contains("save"),
            "{:?}",
            bindings.warnings
        );
        // The conflicting override was not applied.
        let chord = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(bindings.action_for(chord), Some(Action::Save));
    }

    #[test]
    fn validate_names_the_conflicting_action() {
        let bindings = KeyBindings::from_custom(&BTreeMap::new());
        let error = bindings.validate(Action::Find, "ctrl+z").unwrap_err();
        assert!(error.contains("undo"), "{error}");
        assert!(bindings.validate(Action::Find, "ctrl+g").is_ok());
        // Rebinding an action to its own chord is allowed.
        assert!(bindings.validate(Action::Find, "ctrl+f").is_ok());
    }

    #[test]
    fn risky_chords_warn_about_terminal_interception() {
        assert!(chord_warning(&KeyChord::parse("ctrl+tab").unwrap()).is_some());
        assert!(chord_warning(&KeyChord::parse("alt+x").unwrap()).is_some());
        assert!(chord_warning(&KeyChord::parse("ctrl+g").unwrap()).is_none());
    }
}
