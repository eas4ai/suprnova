//! Highlight trusted tags over escaped source text, preserving original Unicode offsets.

use super::{
    expansion::{ExpandedQuery, tokenize},
    settings::QuerySettings,
};
use crate::{FrameworkError, inertia::escape_html_attr as escape};
use unicode_segmentation::UnicodeSegmentation;

/// Matching opening and closing tags prevent settings from introducing executable markup.
pub fn validate_tags(pre: &str, post: &str) -> Result<(), FrameworkError> {
    let inner = pre
        .strip_prefix('<')
        .and_then(|tag| tag.strip_suffix('>'))
        .ok_or_else(|| FrameworkError::param("highlight_pre_tag"))?;
    // HTML whitespace excludes Unicode spaces and ASCII vertical tab.
    let html_space = |ch: char| matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000c}');
    let (name, rest) = inner.split_once(html_space).unwrap_or((inner, ""));
    if !["mark", "em", "strong", "b", "span"].contains(&name) || post != format!("</{name}>") {
        return Err(FrameworkError::param(
            "highlight tags must use matching mark, em, strong, b or span elements",
        ));
    }
    let rest = rest.trim_matches(html_space);
    if !rest.is_empty() {
        let value = rest
            .strip_prefix("class=")
            .ok_or_else(|| FrameworkError::param("highlight class attribute"))?;
        let class = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                value
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .ok_or_else(|| FrameworkError::param("highlight class attribute"))?;
        if class
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '-' | '_')))
        {
            return Err(FrameworkError::param("highlight class attribute"));
        }
    }
    Ok(())
}

/// Cropping happens on source characters before adding tags, so markup is never split.
/// Overlapping phrase and word matches merge into one span to avoid nested tags.
pub fn highlight(
    text: &str,
    query: &ExpandedQuery,
    settings: &QuerySettings,
    crop: Option<usize>,
) -> Result<String, FrameworkError> {
    validate_tags(&settings.highlight_pre_tag, &settings.highlight_post_tag)?;
    if crop == Some(0) {
        return Err(FrameworkError::param("crop length must be positive"));
    }
    let spans = matched_spans(text, query);
    let (start, end) = crop_bounds(text, &spans, crop);
    Ok(render_spans(text, &spans, start, end, settings))
}

/// Native match offsets retain database stemming while sharing safe character cropping.
pub(crate) fn format_native(
    marked: &str,
    settings: &QuerySettings,
    crop: Option<usize>,
) -> Result<String, FrameworkError> {
    validate_tags(&settings.highlight_pre_tag, &settings.highlight_post_tag)?;
    if crop == Some(0) {
        return Err(FrameworkError::param("crop length must be positive"));
    }
    let mut text = String::with_capacity(marked.len());
    let mut spans = Vec::new();
    let mut opened = None;
    for character in marked.chars() {
        match character {
            '\u{001e}' => {
                if opened.is_some() {
                    return Err(FrameworkError::param(
                        "native highlight markers must not nest",
                    ));
                }
                opened = Some(text.len());
            }
            '\u{001f}' => {
                let start = opened.take().ok_or_else(|| {
                    FrameworkError::param("native highlight closing marker has no opening marker")
                })?;
                if start < text.len() {
                    spans.push((start, text.len()));
                }
            }
            character => text.push(character),
        }
    }
    if opened.is_some() {
        return Err(FrameworkError::param(
            "native highlight opening marker has no closing marker",
        ));
    }
    let (start, end) = crop_bounds(&text, &spans, crop);
    Ok(render_spans(&text, &spans, start, end, settings))
}

fn matched_spans(text: &str, query: &ExpandedQuery) -> Vec<(usize, usize)> {
    let words: Vec<_> = text
        .unicode_word_indices()
        .flat_map(|(start, word)| {
            tokenize(word)
                .into_iter()
                .map(move |normalized| (start, start + word.len(), normalized))
        })
        .collect();
    let alternatives: Vec<_> = query
        .terms
        .iter()
        .flat_map(|term| &term.alternatives)
        .map(|alternative| tokenize(alternative))
        .chain(
            query
                .phrases
                .iter()
                .map(|phrase| phrase.iter().flat_map(|word| tokenize(word)).collect()),
        )
        .collect();
    let prefixes: Vec<_> = query
        .terms
        .iter()
        .filter(|term| term.prefix)
        .flat_map(|term| tokenize(&term.original))
        .collect();
    let mut matches = Vec::new();
    for (i, (start, end, word)) in words.iter().enumerate() {
        if prefixes
            .iter()
            .any(|prefix| word.starts_with(prefix.as_str()))
        {
            matches.push((*start, *end));
        }
        for alternative in &alternatives {
            match_phrase(&words, i, alternative, &mut matches);
        }
    }
    matches.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in matches {
        if let Some(previous) = merged.last_mut()
            && start <= previous.1
        {
            previous.1 = previous.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
}

fn crop_bounds(text: &str, spans: &[(usize, usize)], crop: Option<usize>) -> (usize, usize) {
    let Some(length) = crop else {
        return (0, text.len());
    };
    let boundaries: Vec<_> = text
        .char_indices()
        .map(|(at, _)| at)
        .chain(std::iter::once(text.len()))
        .collect();
    let first = spans.first().map_or(0, |span| span.0);
    let first_character = boundaries.partition_point(|at| *at < first);
    let start_character = first_character.saturating_sub(length / 3);
    let mut start = boundaries[start_character];
    let mut end = boundaries[start_character
        .saturating_add(length)
        .min(boundaries.len() - 1)];
    for span in spans {
        if span.0 < start && span.1 > start {
            start = span.0;
        }
        if span.0 < end && span.1 > end {
            end = span.1;
        }
    }
    (start, end)
}

fn render_spans(
    text: &str,
    spans: &[(usize, usize)],
    start: usize,
    end: usize,
    settings: &QuerySettings,
) -> String {
    let mut rendered = String::new();
    let marker = escape(&settings.crop_marker);
    if start > 0 {
        rendered.push_str(&marker);
    }
    let mut cursor = start;
    for &(from, to) in spans
        .iter()
        .filter(|(from, to)| *from >= start && *to <= end)
    {
        rendered.push_str(&escape(&text[cursor..from]));
        rendered.push_str(&settings.highlight_pre_tag);
        rendered.push_str(&escape(&text[from..to]));
        rendered.push_str(&settings.highlight_post_tag);
        cursor = to;
    }
    rendered.push_str(&escape(&text[cursor..end]));
    if end < text.len() {
        rendered.push_str(&marker);
    }
    rendered
}

fn match_phrase(
    words: &[(usize, usize, String)],
    at: usize,
    phrase: &[String],
    matches: &mut Vec<(usize, usize)>,
) {
    if !phrase.is_empty()
        && at.saturating_add(phrase.len()) <= words.len()
        && words[at..at + phrase.len()]
            .iter()
            .map(|(_, _, word)| word)
            .eq(phrase.iter())
    {
        matches.push((words[at].0, words[at + phrase.len() - 1].1));
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;

    #[test]
    fn native_highlights_escape_source_before_adding_validated_tags() {
        let settings = QuerySettings::default();
        let formatted = format_native(
            "<script>alert(1)</script> \u{001e}Rust\u{001f} & \"quoted\"",
            &settings,
            None,
        )
        .expect("balanced native markers");
        assert_eq!(
            formatted,
            "&lt;script&gt;alert(1)&lt;/script&gt; <em>Rust</em> &amp; &quot;quoted&quot;"
        );
        assert!(!formatted.contains('\u{001e}'));
        assert!(!formatted.contains('\u{001f}'));
    }

    #[test]
    fn native_crops_use_unicode_characters_around_the_native_match() {
        let settings = QuerySettings {
            crop_marker: "~".into(),
            ..QuerySettings::default()
        };
        let formatted = format_native(
            "甲乙丙丁戊己庚辛壬癸 \u{001e}東京\u{001f} 都の街 末尾",
            &settings,
            Some(6),
        )
        .expect("Unicode crop");
        assert!(formatted.contains("<em>東京</em>"));
        assert!(formatted.starts_with('~'));
        assert!(formatted.ends_with('~'));
        let visible = formatted.replace("<em>", "").replace("</em>", "");
        assert_eq!(visible.trim_matches('~').chars().count(), 6);
    }

    #[test]
    fn native_snippet_omission_markers_remain_visible() {
        let settings = QuerySettings::default();
        assert_eq!(
            format_native("...near \u{001e}match\u{001f}...", &settings, None)
                .expect("native snippet"),
            "...near <em>match</em>..."
        );
    }

    #[test]
    fn native_highlights_reject_unbalanced_or_nested_markers() {
        let settings = QuerySettings::default();
        for malformed in [
            "\u{001e}open",
            "close\u{001f}",
            "\u{001e}outer\u{001e}inner\u{001f}\u{001f}",
        ] {
            assert!(format_native(malformed, &settings, None).is_err());
        }
        assert!(format_native("text", &settings, Some(0)).is_err());
    }
}
