use ratatui::style::Color;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub bg_alt: Color,
    pub fg: Color,
    pub dim: Color,
    pub border: Color,
    pub accent: Color,
    pub accent2: Color,
    pub green: Color,
    pub red: Color,
    pub yellow: Color,
    pub magenta: Color,
    pub cyan: Color,
    pub orange: Color,
    pub sel: Color,
}

const fn hex(v: u32) -> Color {
    Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const THEMES: [Theme; 4] = [
    Theme {
        name: "tokyonight",
        bg: hex(0x1a1b26),
        bg_alt: hex(0x16161e),
        fg: hex(0xc0caf5),
        dim: hex(0x565f89),
        border: hex(0x3b4261),
        accent: hex(0x7aa2f7),
        accent2: hex(0xbb9af7),
        green: hex(0x9ece6a),
        red: hex(0xf7768e),
        yellow: hex(0xe0af68),
        magenta: hex(0xbb9af7),
        cyan: hex(0x7dcfff),
        orange: hex(0xff9e64),
        sel: hex(0x283457),
    },
    Theme {
        name: "catppuccin",
        bg: hex(0x1e1e2e),
        bg_alt: hex(0x181825),
        fg: hex(0xcdd6f4),
        dim: hex(0x6c7086),
        border: hex(0x45475a),
        accent: hex(0x89b4fa),
        accent2: hex(0xcba6f7),
        green: hex(0xa6e3a1),
        red: hex(0xf38ba8),
        yellow: hex(0xf9e2af),
        magenta: hex(0xf5c2e7),
        cyan: hex(0x94e2d5),
        orange: hex(0xfab387),
        sel: hex(0x313244),
    },
    Theme {
        name: "gruvbox",
        bg: hex(0x282828),
        bg_alt: hex(0x1d2021),
        fg: hex(0xebdbb2),
        dim: hex(0x928374),
        border: hex(0x504945),
        accent: hex(0x83a598),
        accent2: hex(0xd3869b),
        green: hex(0xb8bb26),
        red: hex(0xfb4934),
        yellow: hex(0xfabd2f),
        magenta: hex(0xd3869b),
        cyan: hex(0x8ec07c),
        orange: hex(0xfe8019),
        sel: hex(0x3c3836),
    },
    Theme {
        name: "kanagawa",
        bg: hex(0x1f1f28),
        bg_alt: hex(0x16161d),
        fg: hex(0xdcd7ba),
        dim: hex(0x727169),
        border: hex(0x54546d),
        accent: hex(0x7e9cd8),
        accent2: hex(0x957fb8),
        green: hex(0x98bb6c),
        red: hex(0xff5d62),
        yellow: hex(0xe6c384),
        magenta: hex(0x957fb8),
        cyan: hex(0x7fb4ca),
        orange: hex(0xffa066),
        sel: hex(0x2d4f67),
    },
];

pub fn by_name(name: &str) -> usize {
    THEMES.iter().position(|t| t.name.eq_ignore_ascii_case(name)).unwrap_or(0)
}

pub fn blend(a: Color, b: Color, t: f32) -> Color {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
            Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
        }
        _ => b,
    }
}

pub fn parse_hex(s: &str) -> Option<Color> {
    let v = u32::from_str_radix(s.trim_start_matches('#'), 16).ok()?;
    (s.trim_start_matches('#').len() == 6).then(|| hex(v))
}

/// GitHub linguist colours for the languages most likely to show up.
pub fn language_color(lang: &str, fallback: Color) -> Color {
    let v = match lang {
        "Rust" => 0xdea584,
        "Go" => 0x00add8,
        "C#" => 0x178600,
        "TypeScript" => 0x3178c6,
        "JavaScript" => 0xf1e05a,
        "Python" => 0x3572a5,
        "Bicep" => 0x519aba,
        "HTML" => 0xe34c26,
        "CSS" | "SCSS" => 0x663399,
        "Shell" => 0x89e051,
        "PowerShell" => 0x012456,
        "Java" => 0xb07219,
        "Kotlin" => 0xa97bff,
        "Swift" => 0xf05138,
        "Lua" => 0x000080,
        "Dockerfile" => 0x384d54,
        "HCL" => 0x844fba,
        "Nix" => 0x7e7eff,
        "C" => 0x555555,
        "C++" => 0xf34b7d,
        "Ruby" => 0x701516,
        "PHP" => 0x4f5d95,
        "Vue" => 0x41b883,
        "Svelte" => 0xff3e00,
        "Razor" => 0x512be4,
        "Jupyter Notebook" => 0xda5b0b,
        "Makefile" => 0x427819,
        "Markdown" => 0x083fa1,
        "Vim Script" | "Vim script" => 0x199f4b,
        _ => return fallback,
    };
    hex(v)
}
