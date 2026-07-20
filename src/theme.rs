use crossterm::style::Color;
use serde::{Deserialize, Serialize};

fn rgb(value: u32) -> Color {
    Color::Rgb {
        r: ((value >> 16) & 0xff) as u8,
        g: ((value >> 8) & 0xff) as u8,
        b: (value & 0xff) as u8,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeKind {
    #[default]
    Oxide,
    Mono,
    System,
    Aura,
    Ayu,
    Carbonfox,
    CatppuccinFrappe,
    CatppuccinMacchiato,
    Catppuccin,
    Cobalt2,
    Cursor,
    Nord,
    Dracula,
    Everforest,
    Flexoki,
    Github,
    Gruvbox,
    Kanagawa,
    #[serde(rename = "lucent-orng")]
    LucentOrng,
    Material,
    Matrix,
    Mercury,
    Monokai,
    Nightowl,
    OneDark,
    #[serde(rename = "opencode")]
    OpenCode,
    Orng,
    OsakaJade,
    Palenight,
    Solarized,
    Synthwave84,
    Tokyonight,
    Vercel,
    Vesper,
    Zenburn,
}

impl ThemeKind {
    pub const ALL: [Self; 35] = [
        Self::Oxide,
        Self::System,
        Self::Aura,
        Self::Ayu,
        Self::Carbonfox,
        Self::CatppuccinFrappe,
        Self::CatppuccinMacchiato,
        Self::Catppuccin,
        Self::Cobalt2,
        Self::Cursor,
        Self::Dracula,
        Self::Everforest,
        Self::Flexoki,
        Self::Github,
        Self::Gruvbox,
        Self::Kanagawa,
        Self::LucentOrng,
        Self::Material,
        Self::Matrix,
        Self::Mercury,
        Self::Mono,
        Self::Monokai,
        Self::Nightowl,
        Self::Nord,
        Self::OneDark,
        Self::OpenCode,
        Self::Orng,
        Self::OsakaJade,
        Self::Palenight,
        Self::Solarized,
        Self::Synthwave84,
        Self::Tokyonight,
        Self::Vercel,
        Self::Vesper,
        Self::Zenburn,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Oxide => "Oxide",
            Self::Mono => "Monochrome",
            Self::System => "system",
            Self::Aura => "aura",
            Self::Ayu => "ayu",
            Self::Carbonfox => "carbonfox",
            Self::CatppuccinFrappe => "catppuccin-frappe",
            Self::CatppuccinMacchiato => "catppuccin-macchiato",
            Self::Catppuccin => "catppuccin",
            Self::Cobalt2 => "cobalt2",
            Self::Cursor => "cursor",
            Self::Nord => "Nord",
            Self::Dracula => "Dracula",
            Self::Everforest => "everforest",
            Self::Flexoki => "flexoki",
            Self::Github => "github",
            Self::Gruvbox => "gruvbox",
            Self::Kanagawa => "kanagawa",
            Self::LucentOrng => "lucent-orng",
            Self::Material => "material",
            Self::Matrix => "matrix",
            Self::Mercury => "mercury",
            Self::Monokai => "monokai",
            Self::Nightowl => "nightowl",
            Self::OneDark => "one-dark",
            Self::OpenCode => "opencode",
            Self::Orng => "orng",
            Self::OsakaJade => "osaka-jade",
            Self::Palenight => "palenight",
            Self::Solarized => "Solarized Dark",
            Self::Synthwave84 => "synthwave84",
            Self::Tokyonight => "tokyonight",
            Self::Vercel => "vercel",
            Self::Vesper => "vesper",
            Self::Zenburn => "zenburn",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "" | "oxide" => Some(Self::Oxide),
            "mono" | "monochrome" => Some(Self::Mono),
            "system" => Some(Self::System),
            "aura" => Some(Self::Aura),
            "ayu" => Some(Self::Ayu),
            "carbonfox" => Some(Self::Carbonfox),
            "catppuccin-frappe" => Some(Self::CatppuccinFrappe),
            "catppuccin-macchiato" => Some(Self::CatppuccinMacchiato),
            "catppuccin" | "catppuccin-mocha" => Some(Self::Catppuccin),
            "cobalt2" => Some(Self::Cobalt2),
            "cursor" => Some(Self::Cursor),
            "nord" => Some(Self::Nord),
            "dracula" => Some(Self::Dracula),
            "everforest" => Some(Self::Everforest),
            "flexoki" => Some(Self::Flexoki),
            "github" | "github-dark" => Some(Self::Github),
            "gruvbox" => Some(Self::Gruvbox),
            "kanagawa" => Some(Self::Kanagawa),
            "lucent-orng" => Some(Self::LucentOrng),
            "material" => Some(Self::Material),
            "matrix" => Some(Self::Matrix),
            "mercury" => Some(Self::Mercury),
            "monokai" => Some(Self::Monokai),
            "nightowl" | "night-owl" => Some(Self::Nightowl),
            "one-dark" | "onedark" => Some(Self::OneDark),
            "opencode" | "open-code" => Some(Self::OpenCode),
            "orng" => Some(Self::Orng),
            "osaka-jade" => Some(Self::OsakaJade),
            "palenight" => Some(Self::Palenight),
            "solarized" | "solarized-dark" | "solarized dark" => Some(Self::Solarized),
            "synthwave84" => Some(Self::Synthwave84),
            "tokyonight" | "tokyo-night" => Some(Self::Tokyonight),
            "vercel" => Some(Self::Vercel),
            "vesper" => Some(Self::Vesper),
            "zenburn" => Some(Self::Zenburn),
            _ => None,
        }
    }
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
            kind => Self::named(kind),
        }
    }

    fn named(kind: ThemeKind) -> Self {
        let palette = match kind {
            ThemeKind::System => return Self::system(),
            ThemeKind::Aura => (
                0x15141b, 0xedecee, 0x20202a, 0xa277ff, 0xa277ff, 0x61ffca, 0xffca85, 0x6d6d6d,
                0xffca85, 0xf694ff,
            ),
            ThemeKind::Ayu => (
                0x0b0e14, 0xbfbdb6, 0x11151c, 0xe6b450, 0xff8f40, 0xaad94c, 0xd2a6ff, 0x626a73,
                0x59c2ff, 0xe6b450,
            ),
            ThemeKind::Carbonfox => (
                0x161616, 0xf2f4f8, 0x262626, 0x78a9ff, 0xbe95ff, 0x42be65, 0x82cfff, 0x525252,
                0x3ddbd9, 0xee5396,
            ),
            ThemeKind::CatppuccinFrappe => (
                0x303446, 0xc6d0f5, 0x414559, 0x8caaee, 0xca9ee6, 0xa6d189, 0xef9f76, 0x737994,
                0x81c8be, 0x8caaee,
            ),
            ThemeKind::CatppuccinMacchiato => (
                0x24273a, 0xcad3f5, 0x363a4f, 0x8aadf4, 0xc6a0f6, 0xa6da95, 0xf5a97f, 0x6e738d,
                0x8bd5ca, 0x8aadf4,
            ),
            ThemeKind::Catppuccin => (
                0x1e1e2e, 0xcdd6f4, 0x313244, 0x89b4fa, 0xcba6f7, 0xa6e3a1, 0xfab387, 0x6c7086,
                0x94e2d5, 0x89b4fa,
            ),
            ThemeKind::Cobalt2 => (
                0x193549, 0xe1efff, 0x234e69, 0xffc600, 0xff9d00, 0x3ad900, 0xff628c, 0x0088ff,
                0x80ffc2, 0xffc600,
            ),
            ThemeKind::Cursor => (
                0x1e1e1e, 0xd7dae0, 0x2a2a2a, 0x5cc8ff, 0xc099ff, 0x98c379, 0xf2cc60, 0x6b7280,
                0x56b6c2, 0x5cc8ff,
            ),
            ThemeKind::Everforest => (
                0x2d353b, 0xd3c6aa, 0x3d484d, 0xa7c080, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x859289,
                0x83c092, 0xa7c080,
            ),
            ThemeKind::Flexoki => (
                0x100f0f, 0xcecdc3, 0x282726, 0x4385be, 0x8b7ec8, 0x879a39, 0xd0a215, 0x878580,
                0x3aa99f, 0x4385be,
            ),
            ThemeKind::Github => (
                0x0d1117, 0xe6edf3, 0x161b22, 0x58a6ff, 0xff7b72, 0xa5d6ff, 0x79c0ff, 0x8b949e,
                0xd2a8ff, 0x58a6ff,
            ),
            ThemeKind::Gruvbox => (
                0x282828, 0xebdbb2, 0x3c3836, 0xfabd2f, 0xfb4934, 0xb8bb26, 0xd3869b, 0x928374,
                0x8ec07c, 0xfabd2f,
            ),
            ThemeKind::Kanagawa => (
                0x1f1f28, 0xdcd7ba, 0x2a2a37, 0x7e9cd8, 0x957fb8, 0x98bb6c, 0xe6c384, 0x727169,
                0x7aa89f, 0x7e9cd8,
            ),
            ThemeKind::LucentOrng => (
                0x1c1917, 0xf5f5f4, 0x292524, 0xff7a00, 0xff9f1c, 0xa3e635, 0x38bdf8, 0x78716c,
                0xf97316, 0xff7a00,
            ),
            ThemeKind::Material => (
                0x263238, 0xeeffff, 0x37474f, 0x80cbc4, 0xc792ea, 0xc3e88d, 0xf78c6c, 0x546e7a,
                0x82aaff, 0x80cbc4,
            ),
            ThemeKind::Matrix => (
                0x000000, 0x00ff41, 0x001a00, 0x00ff41, 0x00ff41, 0x00cc33, 0x66ff66, 0x008f11,
                0x00ff41, 0x00ff41,
            ),
            ThemeKind::Mercury => (
                0xf7f8fa, 0x273142, 0xe9edf2, 0x3478f6, 0x8e44ad, 0x2e9d62, 0xd97706, 0x8590a6,
                0x0e7490, 0x3478f6,
            ),
            ThemeKind::Monokai => (
                0x272822, 0xf8f8f2, 0x3e3d32, 0xa6e22e, 0xf92672, 0xe6db74, 0xae81ff, 0x75715e,
                0x66d9ef, 0xa6e22e,
            ),
            ThemeKind::Nightowl => (
                0x011627, 0xd6deeb, 0x0b2942, 0x82aaff, 0xc792ea, 0xaddb67, 0xf78c6c, 0x637777,
                0x7fdbca, 0x82aaff,
            ),
            ThemeKind::OneDark => (
                0x282c34, 0xabb2bf, 0x21252b, 0x61afef, 0xc678dd, 0x98c379, 0xd19a66, 0x5c6370,
                0xe5c07b, 0x61afef,
            ),
            ThemeKind::OpenCode => (
                0x0f0f0f, 0xe4e4e7, 0x1b1b1b, 0xfb923c, 0xc084fc, 0xa3e635, 0x38bdf8, 0x71717a,
                0x22d3ee, 0xfb923c,
            ),
            ThemeKind::Orng => (
                0x1d120b, 0xffe8d1, 0x2f1c0f, 0xff7a00, 0xff9f1c, 0xb8e986, 0x62d0ff, 0x9b6b4a,
                0xffd166, 0xff7a00,
            ),
            ThemeKind::OsakaJade => (
                0x111c18, 0xc1d8c5, 0x1e302a, 0x8bd5ca, 0xc792ea, 0xaddb67, 0xf78c6c, 0x5c8374,
                0x7fdbca, 0x8bd5ca,
            ),
            ThemeKind::Palenight => (
                0x292d3e, 0xa6accd, 0x343b51, 0x82aaff, 0xc792ea, 0xc3e88d, 0xf78c6c, 0x676e95,
                0xffcb6b, 0x82aaff,
            ),
            ThemeKind::Synthwave84 => (
                0x262335, 0xf8f8f2, 0x34294f, 0xff7edb, 0xff7edb, 0xfede5d, 0x36f9f6, 0x848bbd,
                0x72f1b8, 0xff7edb,
            ),
            ThemeKind::Tokyonight => (
                0x1a1b26, 0xc0caf5, 0x24283b, 0x7aa2f7, 0xbb9af7, 0x9ece6a, 0xff9e64, 0x565f89,
                0x2ac3de, 0x7aa2f7,
            ),
            ThemeKind::Vercel => (
                0x000000, 0xededed, 0x1a1a1a, 0x0070f3, 0xc084fc, 0x50e3c2, 0xf5a623, 0x666666,
                0x79ffe1, 0x0070f3,
            ),
            ThemeKind::Vesper => (
                0x101010, 0xb8b8b8, 0x1a1a1a, 0xffc799, 0xa8a8a8, 0x99ffe4, 0xffc799, 0x505050,
                0x99ffe4, 0xffc799,
            ),
            ThemeKind::Zenburn => (
                0x3f3f3f, 0xdcdccc, 0x4f4f4f, 0x8cd0d3, 0xefef8f, 0xcc9393, 0xf0dfaf, 0x7f9f7f,
                0x93e0e3, 0x8cd0d3,
            ),
            _ => return Self::oxide(),
        };
        Self::from_palette(palette)
    }

    fn from_palette(palette: (u32, u32, u32, u32, u32, u32, u32, u32, u32, u32)) -> Self {
        let (
            background,
            foreground,
            surface,
            accent,
            keyword,
            string,
            number,
            comment,
            type_name,
            heading,
        ) = palette;
        let mut theme = Self::oxide();
        theme.background = rgb(background);
        theme.foreground = rgb(foreground);
        theme.muted = rgb(comment);
        theme.gutter = rgb(comment);
        theme.gutter_current = rgb(accent);
        theme.current_line = rgb(surface);
        theme.top_bar = rgb(surface);
        theme.top_bar_text = rgb(accent);
        theme.status_bar = rgb(surface);
        theme.status_text = rgb(foreground);
        theme.prompt_bar = rgb(background);
        theme.prompt_text = rgb(foreground);
        theme.normal_mode = rgb(accent);
        theme.insert_mode = rgb(heading);
        theme.search_mode = rgb(type_name);
        theme.command_mode = rgb(keyword);
        theme.keyword = rgb(keyword);
        theme.string = rgb(string);
        theme.number = rgb(number);
        theme.comment = rgb(comment);
        theme.type_name = rgb(type_name);
        theme.punctuation = rgb(foreground);
        theme.heading = rgb(heading);
        theme.search_background = rgb(accent);
        theme.search_foreground = rgb(background);
        theme.border = rgb(comment);
        theme.overlay = rgb(surface);
        theme.overlay_text = rgb(foreground);
        theme.error = rgb(0xff6b6b);
        theme.success = rgb(string);
        theme
    }

    fn system() -> Self {
        let mut theme = Self::oxide();
        theme.background = Color::Reset;
        theme.foreground = Color::Reset;
        theme.current_line = Color::DarkGrey;
        theme.top_bar = Color::DarkGrey;
        theme.top_bar_text = Color::Yellow;
        theme.status_bar = Color::DarkGrey;
        theme.status_text = Color::Reset;
        theme.prompt_bar = Color::Reset;
        theme.prompt_text = Color::Reset;
        theme.overlay = Color::DarkGrey;
        theme.overlay_text = Color::Reset;
        theme.border = Color::Grey;
        theme.punctuation = Color::Reset;
        theme
    }

    fn oxide() -> Self {
        Self {
            background: Color::Rgb {
                r: 19,
                g: 22,
                b: 30,
            },
            foreground: Color::Rgb {
                r: 221,
                g: 226,
                b: 238,
            },
            muted: Color::Rgb {
                r: 105,
                g: 116,
                b: 135,
            },
            gutter: Color::Rgb {
                r: 71,
                g: 80,
                b: 98,
            },
            gutter_current: Color::Rgb {
                r: 239,
                g: 173,
                b: 79,
            },
            current_line: Color::Rgb {
                r: 25,
                g: 30,
                b: 41,
            },
            top_bar: Color::Rgb {
                r: 38,
                g: 45,
                b: 61,
            },
            top_bar_text: Color::Rgb {
                r: 239,
                g: 173,
                b: 79,
            },
            status_bar: Color::Rgb {
                r: 38,
                g: 45,
                b: 61,
            },
            status_text: Color::Rgb {
                r: 221,
                g: 226,
                b: 238,
            },
            prompt_bar: Color::Rgb {
                r: 14,
                g: 17,
                b: 24,
            },
            prompt_text: Color::Rgb {
                r: 190,
                g: 198,
                b: 215,
            },
            normal_mode: Color::Rgb {
                r: 92,
                g: 201,
                b: 156,
            },
            insert_mode: Color::Rgb {
                r: 92,
                g: 157,
                b: 255,
            },
            search_mode: Color::Rgb {
                r: 239,
                g: 173,
                b: 79,
            },
            command_mode: Color::Rgb {
                r: 200,
                g: 120,
                b: 255,
            },
            keyword: Color::Rgb {
                r: 200,
                g: 120,
                b: 255,
            },
            string: Color::Rgb {
                r: 142,
                g: 205,
                b: 124,
            },
            number: Color::Rgb {
                r: 86,
                g: 201,
                b: 221,
            },
            comment: Color::Rgb {
                r: 105,
                g: 116,
                b: 135,
            },
            type_name: Color::Rgb {
                r: 239,
                g: 173,
                b: 79,
            },
            punctuation: Color::Rgb {
                r: 147,
                g: 160,
                b: 184,
            },
            heading: Color::Rgb {
                r: 92,
                g: 157,
                b: 255,
            },
            search_background: Color::Rgb {
                r: 122,
                g: 82,
                b: 24,
            },
            search_foreground: Color::Rgb {
                r: 255,
                g: 243,
                b: 204,
            },
            border: Color::Rgb {
                r: 71,
                g: 80,
                b: 98,
            },
            overlay: Color::Rgb {
                r: 30,
                g: 36,
                b: 49,
            },
            overlay_text: Color::Rgb {
                r: 221,
                g: 226,
                b: 238,
            },
            error: Color::Rgb {
                r: 255,
                g: 100,
                b: 110,
            },
            success: Color::Rgb {
                r: 92,
                g: 201,
                b: 156,
            },
        }
    }

    fn mono() -> Self {
        Self {
            background: Color::Black,
            foreground: Color::White,
            muted: Color::DarkGrey,
            gutter: Color::DarkGrey,
            gutter_current: Color::White,
            current_line: Color::Rgb {
                r: 28,
                g: 28,
                b: 28,
            },
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
        theme.background = Color::Rgb {
            r: 46,
            g: 52,
            b: 64,
        };
        theme.foreground = Color::Rgb {
            r: 216,
            g: 222,
            b: 233,
        };
        theme.current_line = Color::Rgb {
            r: 59,
            g: 66,
            b: 82,
        };
        theme.top_bar = Color::Rgb {
            r: 59,
            g: 66,
            b: 82,
        };
        theme.prompt_bar = Color::Rgb {
            r: 36,
            g: 41,
            b: 51,
        };
        theme.keyword = Color::Rgb {
            r: 180,
            g: 142,
            b: 173,
        };
        theme.string = Color::Rgb {
            r: 163,
            g: 190,
            b: 140,
        };
        theme.type_name = Color::Rgb {
            r: 136,
            g: 192,
            b: 208,
        };
        theme.heading = Color::Rgb {
            r: 129,
            g: 161,
            b: 193,
        };
        theme.top_bar_text = Color::Rgb {
            r: 136,
            g: 192,
            b: 208,
        };
        theme
    }

    fn dracula() -> Self {
        let mut theme = Self::oxide();
        theme.background = Color::Rgb {
            r: 40,
            g: 42,
            b: 54,
        };
        theme.foreground = Color::Rgb {
            r: 248,
            g: 248,
            b: 242,
        };
        theme.current_line = Color::Rgb {
            r: 68,
            g: 71,
            b: 90,
        };
        theme.top_bar = Color::Rgb {
            r: 68,
            g: 71,
            b: 90,
        };
        theme.prompt_bar = Color::Rgb {
            r: 33,
            g: 34,
            b: 44,
        };
        theme.keyword = Color::Rgb {
            r: 255,
            g: 121,
            b: 198,
        };
        theme.string = Color::Rgb {
            r: 241,
            g: 250,
            b: 140,
        };
        theme.number = Color::Rgb {
            r: 189,
            g: 147,
            b: 249,
        };
        theme.type_name = Color::Rgb {
            r: 139,
            g: 233,
            b: 253,
        };
        theme.heading = Color::Rgb {
            r: 80,
            g: 250,
            b: 123,
        };
        theme.top_bar_text = Color::Rgb {
            r: 255,
            g: 184,
            b: 108,
        };
        theme
    }

    fn solarized() -> Self {
        let mut theme = Self::oxide();
        theme.background = Color::Rgb { r: 0, g: 43, b: 54 };
        theme.foreground = Color::Rgb {
            r: 131,
            g: 148,
            b: 150,
        };
        theme.current_line = Color::Rgb { r: 7, g: 54, b: 66 };
        theme.top_bar = Color::Rgb { r: 7, g: 54, b: 66 };
        theme.prompt_bar = Color::Rgb { r: 0, g: 34, b: 43 };
        theme.keyword = Color::Rgb {
            r: 133,
            g: 153,
            b: 0,
        };
        theme.string = Color::Rgb {
            r: 42,
            g: 161,
            b: 152,
        };
        theme.number = Color::Rgb {
            r: 211,
            g: 54,
            b: 130,
        };
        theme.type_name = Color::Rgb {
            r: 181,
            g: 137,
            b: 0,
        };
        theme.heading = Color::Rgb {
            r: 38,
            g: 139,
            b: 210,
        };
        theme.top_bar_text = Color::Rgb {
            r: 203,
            g: 75,
            b: 22,
        };
        theme
    }
}

#[cfg(test)]
mod tests {
    use super::{Theme, ThemeKind};

    #[test]
    fn requested_theme_catalog_is_available() {
        let names = [
            "system",
            "aura",
            "ayu",
            "carbonfox",
            "catppuccin-frappe",
            "catppuccin-macchiato",
            "catppuccin",
            "cobalt2",
            "cursor",
            "dracula",
            "everforest",
            "flexoki",
            "github",
            "gruvbox",
            "kanagawa",
            "lucent-orng",
            "material",
            "matrix",
            "mercury",
            "monokai",
            "nightowl",
            "nord",
            "one-dark",
            "opencode",
            "orng",
            "osaka-jade",
            "palenight",
            "solarized",
            "synthwave84",
            "tokyonight",
            "vercel",
            "vesper",
            "zenburn",
        ];

        for name in names {
            let kind = ThemeKind::parse(name).expect(name);
            let theme = Theme::for_kind(kind);
            assert!(!kind.name().is_empty());
            if kind != ThemeKind::System {
                assert_ne!(theme.foreground, theme.background, "{name}");
            }
        }
    }

    #[test]
    fn theme_gallery_contains_existing_caret_themes_too() {
        assert_eq!(ThemeKind::ALL.len(), 35);
        assert!(ThemeKind::ALL.contains(&ThemeKind::Oxide));
        assert!(ThemeKind::ALL.contains(&ThemeKind::Mono));
    }
}
