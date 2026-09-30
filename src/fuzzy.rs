use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};
use ratatui::{style::Style, text::Span};

pub struct Hit<T> {
    pub item: T,
    pub idx: Vec<u32>,
}

/// fzf-style filtering (smart case, `'exact`, `^prefix`, `suffix$`, `!negate`) via nucleo,
/// the matcher behind helix. Results are sorted by score; ties keep input order.
pub fn filter<T>(items: impl IntoIterator<Item = T>, key: impl Fn(&T) -> String, query: &str) -> Vec<Hit<T>> {
    let q = query.trim();
    if q.is_empty() {
        return items.into_iter().map(|item| Hit { item, idx: vec![] }).collect();
    }
    let mut m = Matcher::new(Config::DEFAULT);
    let pat = Pattern::parse(q, CaseMatching::Smart, Normalization::Smart);
    let mut buf = Vec::new();
    let mut out: Vec<(u32, Hit<T>)> = Vec::new();
    for item in items {
        let k = key(&item);
        let mut idx = Vec::new();
        if let Some(score) = pat.indices(Utf32Str::new(&k, &mut buf), &mut m, &mut idx) {
            idx.sort_unstable();
            idx.dedup();
            out.push((score, Hit { item, idx }));
        }
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.0));
    out.into_iter().map(|(_, h)| h).collect()
}

/// Splits `text` into spans, styling matched character positions with `hl` on top of the
/// per-character base style.
pub fn highlight(text: &str, idx: &[u32], base: impl Fn(usize) -> Style, hl: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut cur = String::new();
    let mut cur_style: Option<Style> = None;
    for (i, ch) in text.chars().enumerate() {
        let st = if idx.binary_search(&(i as u32)).is_ok() { base(i).patch(hl) } else { base(i) };
        if cur_style != Some(st) {
            if let Some(s) = cur_style {
                spans.push(Span::styled(std::mem::take(&mut cur), s));
            }
            cur_style = Some(st);
        }
        cur.push(ch);
    }
    if let Some(s) = cur_style {
        spans.push(Span::styled(cur, s));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_and_ranks() {
        let items = vec!["acme/Web-Storefront", "ratatui/ratatui", "acme/Admin-Atlas"];
        let hits = filter(items, |s| s.to_string(), "wstore");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item, "acme/Web-Storefront");
        assert!(!hits[0].idx.is_empty());
        assert_eq!(filter(vec!["a", "b"], |s| s.to_string(), "").len(), 2);
    }
}
