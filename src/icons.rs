/// Glyph set. `nerd` needs a Nerd Font (LazyVim users usually have one); `unicode` works
/// in any terminal.
#[derive(Debug, Clone, Copy)]
pub struct Icons {
    pub logo: &'static str,
    pub repo: &'static str,
    pub pr: &'static str,
    pub issue: &'static str,
    pub star: &'static str,
    pub fork: &'static str,
    pub run: &'static str,
    pub deploy: &'static str,
    pub branch: &'static str,
    pub bell: &'static str,
    pub flame: &'static str,
    pub tag: &'static str,
    pub pulse: &'static str,
    pub search: &'static str,
    pub host: &'static str,
    pub user: &'static str,
    pub lock: &'static str,
    pub env: &'static str,
    pub clock: &'static str,
    pub comment: &'static str,
    pub cmd: &'static str,
    pub sep_r: &'static str,
    pub sep_l: &'static str,
}

pub const NERD: Icons = Icons {
    logo: "\u{f09b}",
    repo: "\u{f401}",
    pr: "\u{f407}",
    issue: "\u{f41b}",
    star: "\u{f41e}",
    fork: "\u{f402}",
    run: "\u{f40a}",
    deploy: "\u{f427}",
    branch: "\u{f418}",
    bell: "\u{f49a}",
    flame: "\u{f490}",
    tag: "\u{f412}",
    pulse: "\u{f469}",
    search: "\u{f422}",
    host: "\u{f473}",
    user: "\u{f415}",
    lock: "\u{f47b}",
    env: "\u{f473}",
    clock: "\u{f43a}",
    comment: "\u{f41f}",
    cmd: "\u{f489}",
    sep_r: "\u{e0b4}",
    sep_l: "\u{e0b6}",
};

pub const UNICODE: Icons = Icons {
    logo: "◈",
    repo: "▣",
    pr: "⇄",
    issue: "◎",
    star: "★",
    fork: "⑂",
    run: "▶",
    deploy: "⇪",
    branch: "⎇",
    bell: "✉",
    flame: "♨",
    tag: "#",
    pulse: "∿",
    search: "›",
    host: "⌂",
    user: "@",
    lock: "◆",
    env: "◇",
    clock: "◷",
    comment: "✎",
    cmd: ":",
    sep_r: "",
    sep_l: "",
};

pub fn by_name(name: &str) -> Icons {
    if name.eq_ignore_ascii_case("unicode") || name.eq_ignore_ascii_case("ascii") { UNICODE } else { NERD }
}

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const SUP: [&str; 10] = ["⁰", "¹", "²", "³", "⁴", "⁵", "⁶", "⁷", "⁸", "⁹"];
