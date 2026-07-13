use crossterm::style::Color;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeKind {
    Oxide,
    Mono,
    Nord,
    Dracula,
    Solarized,
}

impl ThemeKind {
    pub const ALL: [Self; 5] = [Self::Oxide, Self::Nord, Self::Dracula, Self::Solarized, Self::Mono];
    pub fn name(self) -> &'static str { match self { Self::Oxide => "Oxide", Self::Mono => "Monochrome", Self::Nord => "Nord", Self::Dracula => "Dracula", Self::Solarized => "Solarized Dark" } }
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub muted: Color,
    pub gutter: Color,
    pub gutter_current: Color,
    pub current_line: Color,
    pub top_bar: Color,
    pub top_bar_text: Color,
    pub status_bar: Color,
    pub status_text: Color,
    pub prompt_bar: Color,
    pub prompt_text: Color,
    pub normal_mode: Color,
    pub insert_mode: Color,
    pub search_mode: Color,
    pub command_mode: Color,
    pub keyword: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub type_name: Color,
    pub punctuation: Color,
    pub heading: Color,
    pub search_background: Color,
    pub search_foreground: Color,
    pub border: Color,
    pub overlay: Color,
    pub overlay_text: Color,
    pub error: Color,
    pub success: Color,
}

impl Theme {
    pub fn for_kind(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::Oxide => Self::oxide(),
            ThemeKind::Mono => Self::mono(),
            ThemeKind::Nord => Self::nord(),
            ThemeKind::Dracula => Self::dracula(),
            ThemeKind::Solarized => Self::solarized(),
        }
    }

    fn oxide() -> Self {
        Self {
            background: Color::Rgb { r: 19, g: 22, b: 30 },
            foreground: Color::Rgb { r: 221, g: 226, b: 238 },
            muted: Color::Rgb { r: 105, g: 116, b: 135 },
            gutter: Color::Rgb { r: 71, g: 80, b: 98 },
            gutter_current: Color::Rgb { r: 239, g: 173, b: 79 },
            current_line: Color::Rgb { r: 25, g: 30, b: 41 },
            top_bar: Color::Rgb { r: 38, g: 45, b: 61 },
            top_bar_text: Color::Rgb { r: 239, g: 173, b: 79 },
            status_bar: Color::Rgb { r: 38, g: 45, b: 61 },
            status_text: Color::Rgb { r: 221, g: 226, b: 238 },
            prompt_bar: Color::Rgb { r: 14, g: 17, b: 24 },
            prompt_text: Color::Rgb { r: 190, g: 198, b: 215 },
            normal_mode: Color::Rgb { r: 92, g: 201, b: 156 },
            insert_mode: Color::Rgb { r: 92, g: 157, b: 255 },
            search_mode: Color::Rgb { r: 239, g: 173, b: 79 },
            command_mode: Color::Rgb { r: 200, g: 120, b: 255 },
            keyword: Color::Rgb { r: 200, g: 120, b: 255 },
            string: Color::Rgb { r: 142, g: 205, b: 124 },
            number: Color::Rgb { r: 86, g: 201, b: 221 },
            comment: Color::Rgb { r: 105, g: 116, b: 135 },
            type_name: Color::Rgb { r: 239, g: 173, b: 79 },
            punctuation: Color::Rgb { r: 147, g: 160, b: 184 },
            heading: Color::Rgb { r: 92, g: 157, b: 255 },
            search_background: Color::Rgb { r: 122, g: 82, b: 24 },
            search_foreground: Color::Rgb { r: 255, g: 243, b: 204 },
            border: Color::Rgb { r: 71, g: 80, b: 98 },
            overlay: Color::Rgb { r: 30, g: 36, b: 49 },
            overlay_text: Color::Rgb { r: 221, g: 226, b: 238 },
            error: Color::Rgb { r: 255, g: 100, b: 110 },
            success: Color::Rgb { r: 92, g: 201, b: 156 },
        }
    }

    fn mono() -> Self {
        Self {
            background: Color::Black,
            foreground: Color::White,
            muted: Color::DarkGrey,
            gutter: Color::DarkGrey,
            gutter_current: Color::White,
            current_line: Color::Rgb { r: 28, g: 28, b: 28 },
            top_bar: Color::DarkGrey,
            top_bar_text: Color::White,
            status_bar: Color::DarkGrey,
            status_text: Color::White,
            prompt_bar: Color::Black,
            prompt_text: Color::White,
            normal_mode: Color::White,
            insert_mode: Color::White,
            search_mode: Color::White,
            command_mode: Color::White,
            keyword: Color::White,
            string: Color::Grey,
            number: Color::White,
            comment: Color::DarkGrey,
            type_name: Color::Grey,
            punctuation: Color::Grey,
            heading: Color::White,
            search_background: Color::White,
            search_foreground: Color::Black,
            border: Color::Grey,
            overlay: Color::DarkGrey,
            overlay_text: Color::White,
            error: Color::White,
            success: Color::White,
        }
    }

    fn nord() -> Self {
        let mut theme = Self::oxide();
        theme.background = Color::Rgb { r: 46, g: 52, b: 64 }; theme.foreground = Color::Rgb { r: 216, g: 222, b: 233 };
        theme.current_line = Color::Rgb { r: 59, g: 66, b: 82 }; theme.top_bar = Color::Rgb { r: 59, g: 66, b: 82 };
        theme.prompt_bar = Color::Rgb { r: 36, g: 41, b: 51 }; theme.keyword = Color::Rgb { r: 180, g: 142, b: 173 };
        theme.string = Color::Rgb { r: 163, g: 190, b: 140 }; theme.type_name = Color::Rgb { r: 136, g: 192, b: 208 };
        theme.heading = Color::Rgb { r: 129, g: 161, b: 193 }; theme.top_bar_text = Color::Rgb { r: 136, g: 192, b: 208 };
        theme
    }

    fn dracula() -> Self {
        let mut theme = Self::oxide();
        theme.background = Color::Rgb { r: 40, g: 42, b: 54 }; theme.foreground = Color::Rgb { r: 248, g: 248, b: 242 };
        theme.current_line = Color::Rgb { r: 68, g: 71, b: 90 }; theme.top_bar = Color::Rgb { r: 68, g: 71, b: 90 };
        theme.prompt_bar = Color::Rgb { r: 33, g: 34, b: 44 }; theme.keyword = Color::Rgb { r: 255, g: 121, b: 198 };
        theme.string = Color::Rgb { r: 241, g: 250, b: 140 }; theme.number = Color::Rgb { r: 189, g: 147, b: 249 };
        theme.type_name = Color::Rgb { r: 139, g: 233, b: 253 }; theme.heading = Color::Rgb { r: 80, g: 250, b: 123 };
        theme.top_bar_text = Color::Rgb { r: 255, g: 184, b: 108 }; theme
    }

    fn solarized() -> Self {
        let mut theme = Self::oxide();
        theme.background = Color::Rgb { r: 0, g: 43, b: 54 }; theme.foreground = Color::Rgb { r: 131, g: 148, b: 150 };
        theme.current_line = Color::Rgb { r: 7, g: 54, b: 66 }; theme.top_bar = Color::Rgb { r: 7, g: 54, b: 66 };
        theme.prompt_bar = Color::Rgb { r: 0, g: 34, b: 43 }; theme.keyword = Color::Rgb { r: 133, g: 153, b: 0 };
        theme.string = Color::Rgb { r: 42, g: 161, b: 152 }; theme.number = Color::Rgb { r: 211, g: 54, b: 130 };
        theme.type_name = Color::Rgb { r: 181, g: 137, b: 0 }; theme.heading = Color::Rgb { r: 38, g: 139, b: 210 };
        theme.top_bar_text = Color::Rgb { r: 203, g: 75, b: 22 }; theme
    }
}
