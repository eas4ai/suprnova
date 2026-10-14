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
    let (name, rest) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
    if !["mark", "em", "strong", "b", "span"].contains(&name) || post != format!("</{name}>") {
        return Err(FrameworkError::param(
            "highlight tags must use matching mark, em, strong, b or span elements",
        ));
    }
    let rest = rest.trim();
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
        .chain(query.phrases.clone())
        .collect();
    let prefixes: Vec<_> = query
        .terms
        .iter()
        .filter(|term| term.prefix)
        .map(|term| &term.original)
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
