//! `Str`: the string helpers worth having from Laravel's `Str`, by the
//! developer's ruling a subset, not the whole `Stringable`.
//!
//! Every count is in characters, never bytes, so a multibyte value is never
//! cut inside a character.

mod ascii_languages;
mod ascii_map;
mod inflector;
mod inflector_additional_rules;
mod inflector_rules;

use inflector::{Tongue, upper_first};

/// The words Laravel's `Pluralizer` never pluralizes.
const UNCOUNTABLE: &[&str] = &["recommended", "related"];

/// String helpers in the shape of Laravel's `Str`.
///
/// ```
/// use suprnova::Str;
///
/// assert_eq!(Str::slug("Laravel 5 Framework", "-"), "laravel-5-framework");
/// assert_eq!(Str::mask("taylor@example.com", '*', 3, None), "tay***************");
/// assert_eq!(Str::plural("child", 2), "children");
/// ```
pub struct Str;

impl Str {
    /// A URL slug: the title spelled in ASCII as Laravel spells it, lower
    /// case, `@` written as `at`, and each run of other characters between
    /// letters and digits made one `separator`. `Œuvre d'art` becomes
    /// `oeuvre-dart`. A character Laravel's ASCII map does not know, such
    /// as a Han character or an emoji, is dropped.
    pub fn slug(title: &str, separator: &str) -> String {
        Self::slug_in(title, separator, "en")
    }

    /// A slug using the named language's ASCII spelling. For example,
    /// German `Ärger` becomes `aerger`. An empty language keeps Unicode.
    pub fn slug_in(title: &str, separator: &str, language: &str) -> String {
        let separators: Vec<char> = separator.chars().collect();
        let flip = if separator == "-" { '_' } else { '-' };
        let mut text = String::new();
        let mut in_flip = false;
        let title = if language.is_empty() {
            title.to_owned()
        } else {
            ascii(title, language)
        };
        for c in title.chars() {
            if c == flip && !separators.contains(&flip) {
                if !in_flip {
                    text.push_str(separator);
                }
                in_flip = true;
            } else {
                in_flip = false;
                if c == '@' {
                    text.push_str(separator);
                    text.push_str("at");
                    text.push_str(separator);
                } else {
                    text.push(c);
                }
            }
        }
        let is_separator = |c: char| separators.contains(&c) || c.is_whitespace();
        let mut slug = String::new();
        let mut in_run = false;
        for c in text.to_lowercase().chars() {
            if is_separator(c) {
                if !in_run {
                    slug.push_str(separator);
                }
                in_run = true;
            } else if c.is_alphanumeric() {
                slug.push(c);
                in_run = false;
            }
        }
        slug.trim_matches(|c: char| separators.contains(&c))
            .to_owned()
    }

    /// `value` with the characters from `index` replaced by `character`:
    /// `length` of them, or all to the end when `None`. A negative `index`
    /// counts from the end, and a negative `length` stops that many short of
    /// the end. A range with nothing in it changes nothing.
    pub fn mask(value: &str, character: char, index: isize, length: Option<isize>) -> String {
        let chars: Vec<char> = value.chars().collect();
        let Some((start, count)) = char_range(chars.len(), index, length) else {
            return value.to_owned();
        };
        let mut masked: String = chars[..start].iter().collect();
        masked.extend(std::iter::repeat_n(character, count));
        masked.extend(&chars[start + count..]);
        masked
    }

    /// `value` cut to its first `limit` characters, less any ASCII
    /// whitespace the cut leaves at the end, with `end` after it. A value
    /// no longer than the limit is returned as it is.
    pub fn limit(value: &str, limit: usize, end: &str) -> String {
        if value.chars().count() <= limit {
            return value.to_owned();
        }
        let kept: String = value.chars().take(limit).collect();
        format!("{}{end}", kept.trim_end_matches(PHP_TRIM))
    }

    /// Keep the first `words` runs of non-space characters, including HTML
    /// markup, and append `end` when cut. Whitespace between words stays.
    /// A zero limit leaves the value unchanged, as Laravel does.
    pub fn words(value: &str, words: usize, end: &str) -> String {
        if words == 0 {
            return value.to_owned();
        }
        let mut count = 0;
        let mut in_word = false;
        for (at, c) in value.char_indices() {
            if c.is_whitespace() {
                in_word = false;
            } else if !in_word {
                if count == words {
                    return format!("{}{end}", value[..at].trim_end_matches(PHP_TRIM));
                }
                count += 1;
                in_word = true;
            }
        }
        value.to_owned()
    }

    /// As [`limit`](Self::limit), but the cut falls at the last ASCII
    /// whitespace within the limit, so no word is broken; a no-break space
    /// is not a place to break. Each run of line breaks counts as one
    /// space.
    pub fn limit_words(value: &str, limit: usize, end: &str) -> String {
        if value.chars().count() <= limit {
            return value.to_owned();
        }
        let flat = one_space_per_line_break(value);
        let flat = flat.trim_matches(PHP_TRIM);
        let chars: Vec<char> = flat.chars().collect();
        if chars.len() <= limit {
            return flat.to_owned();
        }
        let kept: String = chars[..limit].iter().collect();
        let kept = kept.trim_end_matches(PHP_TRIM);
        if chars[limit] == ' ' {
            return format!("{kept}{end}");
        }
        match kept.rfind(|c: char| c.is_ascii_whitespace() || c == '\x0B') {
            Some(at) => format!("{}{end}", &kept[..at]),
            None => format!("{kept}{end}"),
        }
    }

    /// The first match of `phrase` in `text`, ignoring case, with up to
    /// `radius` characters on each side, and `omission` where the excerpt
    /// cuts the text. `None` when the phrase is not there.
    pub fn excerpt(text: &str, phrase: &str, radius: usize, omission: &str) -> Option<String> {
        let pattern = format!("(?is)^(.*?)({})(.*)$", regex::escape(phrase));
        let captures = regex::Regex::new(&pattern).ok()?.captures(text)?;
        let before = captures
            .get(1)
            .map_or("", |m| m.as_str())
            .trim_start_matches(PHP_TRIM);
        let found = captures.get(2).map_or("", |m| m.as_str());
        let after = captures
            .get(3)
            .map_or("", |m| m.as_str())
            .trim_end_matches(PHP_TRIM);

        let before_chars: Vec<char> = before.chars().collect();
        let start: String = before_chars[before_chars.len().saturating_sub(radius)..]
            .iter()
            .collect();
        let start = start.trim_start_matches(invisible);
        let start = if start == before {
            start.to_owned()
        } else {
            format!("{omission}{start}")
        };

        let end: String = after.chars().take(radius).collect();
        let end = end.trim_end_matches(invisible);
        let end = if end == after {
            end.to_owned()
        } else {
            format!("{end}{omission}")
        };
        Some(format!("{start}{found}{end}"))
    }

    /// The plural of `word` for `count`, by the rules of the language of the
    /// current `Lang` locale: English, Esperanto, French, Italian,
    /// Norwegian Bokmål, Portuguese, Spanish or Turkish, and English for
    /// any other. A count of 1 or -1
    /// leaves the word, and the result keeps the word's case.
    pub fn plural(word: &str, count: i64) -> String {
        let ends_in_a_word_character = word
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric() || ('\u{80}'..='\u{FFFF}').contains(&c));
        if count.unsigned_abs() == 1
            || !ends_in_a_word_character
            || UNCOUNTABLE.contains(&word.to_lowercase().as_str())
        {
            return word.to_owned();
        }
        let inflected = if word.to_uppercase() == word {
            current_tongue().pluralize(&word.to_lowercase())
        } else {
            current_tongue().pluralize(word)
        };
        match_case(&inflected, word)
    }

    /// The plural form with its count before it, as Laravel's
    /// `prependCount` option. The current locale formats the integer.
    pub fn plural_with_count(word: &str, count: i64) -> String {
        #[cfg(feature = "localization")]
        let prefix = crate::localization::format_integer(count);
        #[cfg(not(feature = "localization"))]
        let prefix = count.to_string();
        format!("{prefix} {}", Self::plural(word, count))
    }

    /// Inflect the last word of a studly-cased value, keeping its prefix.
    /// `VerifiedHuman` becomes `VerifiedHumans` for a count of two.
    pub fn plural_studly(value: &str, count: i64) -> String {
        let start = value
            .char_indices()
            .filter(|(at, c)| *at > 0 && c.is_ascii_uppercase())
            .map(|(at, _)| at)
            .next_back()
            .unwrap_or(0);
        format!(
            "{}{}",
            &value[..start],
            Self::plural(&value[start..], count)
        )
    }

    /// Inflect the last word of a Pascal-cased value, as
    /// [`plural_studly`](Self::plural_studly) does.
    pub fn plural_pascal(value: &str, count: i64) -> String {
        Self::plural_studly(value, count)
    }

    /// The singular of `word`, by the rules of the current locale's language
    /// as [`plural`](Self::plural) chooses them, keeping the word's case.
    pub fn singular(word: &str) -> String {
        let inflected = if word.to_uppercase() == word {
            current_tongue().singularize(&word.to_lowercase())
        } else {
            current_tongue().singularize(word)
        };
        match_case(&inflected, word)
    }
}

/// The characters PHP's `trim` removes by default, which Laravel's `limit`
/// trims with.
const PHP_TRIM: &[char] = &[' ', '\t', '\n', '\r', '\0', '\x0B'];

/// Laravel's `INVISIBLE_CHARACTERS`, plus its default trim whitespace.
fn invisible(c: char) -> bool {
    c.is_whitespace()
        || c == '\0'
        || matches!(c,
            '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{115f}' | '\u{1160}'
            | '\u{17b4}' | '\u{17b5}' | '\u{180e}' | '\u{200b}'..='\u{200f}'
            | '\u{2060}'..='\u{2065}' | '\u{206a}'..='\u{206f}' | '\u{2800}'
            | '\u{3164}' | '\u{feff}' | '\u{ffa0}' | '\u{1d159}'
            | '\u{1d173}'..='\u{1d17a}' | '\u{e0020}'
        )
}

/// The longest key in [`ascii_map::MAP`], in characters.
const LONGEST_KEY: usize = 5;

/// `value` spelled in ASCII as Laravel's `Str::ascii` spells it, which is
/// voku/portable-ascii's `to_ascii` with the named language: each sequence
/// its map knows is replaced, the longest first, as PHP's `strtr` does,
/// and every character still outside printable ASCII is dropped, a tab or
/// line break becoming a space.
fn ascii(value: &str, language: &str) -> String {
    let printable = |c: char| (' '..='~').contains(&c);
    if value.chars().all(printable) {
        return value.to_owned();
    }
    let language = language.to_ascii_lowercase().replace('-', "_");
    let mut parts = language.splitn(2, '_');
    let first = parts.next().unwrap_or("");
    let language = if parts.next().is_some_and(|rest| rest.starts_with(first)) {
        language.replacen(&format!("{first}_{first}"), first, 1)
    } else {
        language
    };
    let overrides = ascii_languages::MAPS
        .binary_search_by(|(key, _)| key.cmp(&language.as_str()))
        .ok()
        .map_or(&[][..], |found| ascii_languages::MAPS[found].1);
    let mut replaced = String::with_capacity(value.len());
    let mut rest = value;
    'next: while let Some(first) = rest.chars().next() {
        let ends: Vec<usize> = rest
            .char_indices()
            .take(LONGEST_KEY)
            .map(|(at, c)| at + c.len_utf8())
            .collect();
        for &end in ends.iter().rev() {
            if let Ok(found) = overrides.binary_search_by(|(key, _)| (*key).cmp(&rest[..end])) {
                replaced.push_str(overrides[found].1);
                rest = &rest[end..];
                continue 'next;
            }
            if let Ok(found) = ascii_map::MAP.binary_search_by(|(key, _)| (*key).cmp(&rest[..end]))
            {
                replaced.push_str(ascii_map::MAP[found].1);
                rest = &rest[end..];
                continue 'next;
            }
        }
        replaced.push(first);
        rest = &rest[first.len_utf8()..];
    }
    if replaced.chars().all(printable) {
        return replaced;
    }
    replaced
        .replace("\r\n", " ")
        .replace(['\n', '\r', '\t'], " ")
        .chars()
        // voku keeps these two control characters too; the slug drops them.
        .filter(|&c| printable(c) || c == '\x10' || c == '\x13')
        .collect()
}

/// `value` with each run of line breaks made one space, as Laravel's
/// `limit` does before it looks for a word boundary.
fn one_space_per_line_break(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut in_break = false;
    for c in value.chars() {
        if c == '\r' || c == '\n' {
            if !in_break {
                out.push(' ');
            }
            in_break = true;
        } else {
            out.push(c);
            in_break = false;
        }
    }
    out
}

/// The language whose rules apply: the current locale's.
fn current_tongue() -> Tongue {
    #[cfg(feature = "localization")]
    let language = crate::Lang::locale().language();
    // Without localization there is no locale, so the rules are English's;
    // they come through the same mapping as every other language.
    #[cfg(not(feature = "localization"))]
    let language = "en";
    Tongue::for_language(&language)
}

/// The start and length, in characters, of `mb_substr(value, index,
/// length)` over a value of `len` characters; `None` when it is empty.
fn char_range(len: usize, index: isize, length: Option<isize>) -> Option<(usize, usize)> {
    let len_i = len as isize;
    let start = if index < 0 {
        (len_i + index).max(0)
    } else {
        index
    };
    if start >= len_i {
        return None;
    }
    let end = match length {
        None => len_i,
        Some(length) if length < 0 => len_i + length,
        Some(length) => start.saturating_add(length).min(len_i),
    };
    (end > start).then(|| (start as usize, (end - start) as usize))
}

/// `value` in the case of `comparison`, as Laravel's `Pluralizer`
/// matches it: all lower, all upper, a capital first letter, or a capital
/// on each word.
fn match_case(value: &str, comparison: &str) -> String {
    if comparison.to_lowercase() == comparison {
        value.to_lowercase()
    } else if comparison.to_uppercase() == comparison {
        value.to_uppercase()
    } else if upper_words(comparison) == comparison {
        upper_words(value)
    } else if upper_first(comparison) == comparison {
        upper_first(value)
    } else {
        value.to_owned()
    }
}

/// `value` with the first character of each word in upper case.
fn upper_words(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut at_start = true;
    for c in value.chars() {
        if at_start {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        at_start = c.is_whitespace();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_follow_mb_substr() {
        assert_eq!(char_range(6, 1, Some(-2)), Some((1, 3)));
        assert_eq!(char_range(6, -2, None), Some((4, 2)));
        assert_eq!(char_range(6, -10, Some(2)), Some((0, 2)));
        assert_eq!(char_range(6, 6, None), None);
        assert_eq!(char_range(6, 2, Some(-5)), None);
    }

    #[test]
    fn the_ascii_map_is_sorted_with_short_keys() {
        assert!(
            ascii_map::MAP.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "the binary search needs the keys sorted and distinct"
        );
        assert!(
            ascii_map::MAP
                .iter()
                .all(|(key, _)| (1..=LONGEST_KEY).contains(&key.chars().count()))
        );
        assert!(
            ascii_languages::MAPS
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0)
        );
        for (_, map) in ascii_languages::MAPS {
            assert!(map.windows(2).all(|pair| pair[0].0 < pair[1].0));
            assert!(map.iter().all(|(key, replacement)| {
                (1..=LONGEST_KEY).contains(&key.chars().count())
                    && replacement.chars().all(|c| (' '..='~').contains(&c))
            }));
        }
        assert!(
            ascii_map::MAP
                .iter()
                .all(|(_, ascii)| ascii.chars().all(|c| (' '..='~').contains(&c)))
        );
    }

    #[test]
    fn case_matches_the_comparison() {
        assert_eq!(match_case("people", "Person"), "People");
        assert_eq!(match_case("cars", "CAR"), "CARS");
        assert_eq!(match_case("new cars", "New Car"), "New Cars");
        assert_eq!(match_case("iphones", "iPhone"), "iphones");
    }
}
