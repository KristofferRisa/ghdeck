use chrono::{DateTime, Utc};

pub fn secs(s: i64) -> String {
    match s {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s if s < 7 * 86_400 => format!("{}d", s / 86_400),
        s if s < 30 * 86_400 => format!("{}w", s / (7 * 86_400)),
        s if s < 365 * 86_400 => format!("{}mo", s / (30 * 86_400)),
        s => format!("{}y", s / (365 * 86_400)),
    }
}

pub fn ago(t: DateTime<Utc>) -> String {
    secs((Utc::now() - t).num_seconds().max(0))
}

pub fn ago_opt(t: Option<DateTime<Utc>>) -> String {
    t.map(ago).unwrap_or_else(|| "–".into())
}

pub fn dur(d: chrono::Duration) -> String {
    let s = d.num_seconds().max(0);
    match s {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m{:02}s", s / 60, s % 60),
        s => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
    }
}

pub fn count(n: u64) -> String {
    match n {
        n if n < 1_000 => n.to_string(),
        n if n < 10_000 => format!("{:.1}k", n as f64 / 1_000.0),
        n if n < 1_000_000 => format!("{}k", n / 1_000),
        n => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

pub fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut o: String = s.chars().take(n.saturating_sub(1)).collect();
        o.push('…');
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(secs(59), "59s");
        assert_eq!(secs(3600 * 5), "5h");
        assert_eq!(count(1234), "1.2k");
        assert_eq!(count(123_456), "123k");
        assert_eq!(dur(chrono::Duration::seconds(125)), "2m05s");
        assert_eq!(trunc("abcdef", 4), "abc…");
    }
}
