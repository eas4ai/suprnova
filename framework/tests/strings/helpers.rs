//! PAR-035: slug, mask, limits, words and excerpt.

use suprnova::Str;

#[test]
fn slug_spells_ascii_and_joins_words() {
    assert_eq!(Str::slug("Laravel 5 Framework", "-"), "laravel-5-framework");
    assert_eq!(
        Str::slug("Œuvre d'art_2 @home", "-"),
        "oeuvre-dart-2-at-home"
    );
    assert_eq!(Str::slug("foo bar", "_"), "foo_bar");
    assert_eq!(Str::slug("  --Hello,   World!--  ", "-"), "hello-world");
    assert_eq!(Str::slug("Ünïcödé Straße", "-"), "unicode-strasse");
    assert_eq!(Str::slug("", "-"), "");
}

/// Laravel 13.34.0 gave each of these, through voku/portable-ascii 2.1.1.
#[test]
fn slug_spells_every_script_as_laravel_does() {
    for (title, slug) in [
        ("北亰 city", "city"),
        ("げんまい茶", ""),
        ("بسم الله", "bsm-allh"),
        ("I ❤ Rust", "i-rust"),
        ("© 2026", "2026"),
        ("™ brand", "brand"),
        ("½ price", "price"),
        ("ǅungla", "ungla"),
        ("DŽungla", "dzungla"),
        ("Ærøskøbing Straße", "aeroskobing-strasse"),
        ("Ελληνικά νέα", "ellinika-nea"),
        ("Привет мир", "privet-mir"),
        ("Tiếng Việt", "tieng-viet"),
        ("Ça va\tbien", "ca-va-bien"),
        ("e\u{301}te\u{301}", "ete"),
        ("Ⓐ ₀ ①", "0"),
    ] {
        assert_eq!(Str::slug(title, "-"), slug, "the slug of {title:?}");
    }
}

#[test]
fn slug_uses_the_named_languages_complete_transliteration_map() {
    for (title, language, expected) in [
        ("Ärger", "de", "aerger"),
        ("Ärger Öl Über", "DE-de", "aerger-oel-ueber"),
        ("Ärger Öl Über", "de_AT", "aerger-oel-ueber"),
        ("Ärger", "en", "arger"),
        ("Ärger", "", "ärger"),
        ("Привет мир", "ru", "privet-mir"),
        ("Straße", "de_at", "strasze"),
        ("Ĉu ĝuste?", "eo", "cxu-gxuste"),
        ("Århus Øresund", "da", "aarhus-oeresund"),
        ("ου", "el", "u"),
        ("ου", "el__greeklish", "ou"),
        ("", "de", ""),
    ] {
        assert_eq!(
            Str::slug_in(title, "-", language),
            expected,
            "{language}: {title}"
        );
    }
    assert_eq!(Str::slug_in("Ärger Öl", "_", "de"), "aerger_oel");
    assert_eq!(Str::slug_in("a/b", "", "en"), "ab");
}

#[test]
fn words_counts_space_separated_runs_without_stripping_markup() {
    assert_eq!(
        Str::words("Perfectly balanced, as all things should be.", 3, " >>>"),
        "Perfectly balanced, as >>>"
    );
    assert_eq!(
        Str::words("<b>bold</b> text here", 2, "..."),
        "<b>bold</b> text..."
    );
    assert_eq!(Str::words("  héllo\t世界\nagain", 2, "…"), "  héllo\t世界…");
    assert_eq!(Str::words("one two  ", 2, "..."), "one two  ");
    assert_eq!(Str::words("one", usize::MAX, "..."), "one");
    assert_eq!(Str::words("one two", 0, "..."), "one two");
    assert_eq!(Str::words("   ", 1, "..."), "   ");
    assert_eq!(Str::words("", 1, "..."), "");
    assert_eq!(Str::words("one two", 1, ""), "one");
}

#[test]
fn mask_replaces_characters_from_an_index() {
    assert_eq!(
        Str::mask("taylor@example.com", '*', 3, None),
        "tay***************"
    );
    assert_eq!(
        Str::mask("taylor@example.com", '*', -15, Some(3)),
        "tay***@example.com"
    );
    assert_eq!(
        Str::mask("taylor@example.com", '*', 0, Some(6)),
        "******@example.com"
    );
    assert_eq!(Str::mask("héllo wörld", '#', 1, Some(4)), "h#### wörld");
    assert_eq!(
        Str::mask("short", '*', 10, None),
        "short",
        "an index past the end masks nothing"
    );
    assert_eq!(
        Str::mask("abcdef", '*', 1, Some(-2)),
        "a***ef",
        "a negative length stops short of the end"
    );
}

#[test]
fn mask_with_a_huge_length_masks_to_the_end() {
    assert_eq!(
        Str::mask("taylor@example.com", '*', 3, Some(isize::MAX)),
        "tay***************"
    );
    assert_eq!(Str::mask("abc", '*', isize::MIN, Some(isize::MIN)), "abc");
    assert_eq!(Str::mask("abc", '*', isize::MAX, Some(isize::MAX)), "abc");
}

#[test]
fn limit_cuts_at_a_character_count() {
    assert_eq!(
        Str::limit("The quick brown fox jumps over the lazy dog", 20, "..."),
        "The quick brown fox..."
    );
    assert_eq!(Str::limit("short", 20, "..."), "short", "nothing to cut");
    assert_eq!(Str::limit("héllo wörld", 4, " (more)"), "héll (more)");
    assert_eq!(
        Str::limit_words("The quick brown fox", 12, "..."),
        "The quick..."
    );
    assert_eq!(
        Str::limit_words("The quick brown fox", 9, "..."),
        "The quick..."
    );
    assert_eq!(
        Str::limit_words("The quick brown fox", 30, "..."),
        "The quick brown fox"
    );
}

/// Laravel 13.34.0 gave each of these.
#[test]
fn limit_words_breaks_lines_and_spaces_as_laravel_does() {
    assert_eq!(
        Str::limit_words("The quick\r\nbrown fox jumps", 14, "..."),
        "The quick...",
        "a CRLF is one space"
    );
    assert_eq!(
        Str::limit_words("The quick\n\n\nbrown fox", 12, "..."),
        "The quick..."
    );
    assert_eq!(
        Str::limit_words("a\u{a0}bc d", 3, "..."),
        "a\u{a0}b...",
        "a no-break space is not a place to break"
    );
    assert_eq!(Str::limit_words("one\ttwo three", 6, "..."), "one...");
}

#[test]
fn excerpt_frames_the_first_match() {
    assert_eq!(
        Str::excerpt("This is my name", "my", 3, "...").as_deref(),
        Some("...is my na...")
    );
    assert_eq!(
        Str::excerpt("This is my name", "THIS", 4, "...").as_deref(),
        Some("This is...")
    );
    assert_eq!(
        Str::excerpt("This is my name", "name", 3, "(...)").as_deref(),
        Some("(...)my name")
    );
    assert_eq!(Str::excerpt("This is my name", "missing", 3, "..."), None);
    assert_eq!(
        Str::excerpt("ünïcödé wörds hère", "wörds", 2, "…").as_deref(),
        Some("…é wörds h…")
    );
}

#[test]
fn excerpt_trims_laravels_invisible_characters_at_cut_ends() {
    for character in [
        '\u{9}',
        '\u{20}',
        '\u{a0}',
        '\u{ad}',
        '\u{34f}',
        '\u{61c}',
        '\u{115f}',
        '\u{1160}',
        '\u{17b4}',
        '\u{17b5}',
        '\u{180e}',
        '\u{2000}',
        '\u{2001}',
        '\u{2002}',
        '\u{2003}',
        '\u{2004}',
        '\u{2005}',
        '\u{2006}',
        '\u{2007}',
        '\u{2008}',
        '\u{2009}',
        '\u{200a}',
        '\u{200b}',
        '\u{200c}',
        '\u{200d}',
        '\u{200e}',
        '\u{200f}',
        '\u{202f}',
        '\u{205f}',
        '\u{2060}',
        '\u{2061}',
        '\u{2062}',
        '\u{2063}',
        '\u{2064}',
        '\u{2065}',
        '\u{206a}',
        '\u{206b}',
        '\u{206c}',
        '\u{206d}',
        '\u{206e}',
        '\u{206f}',
        '\u{3000}',
        '\u{2800}',
        '\u{3164}',
        '\u{feff}',
        '\u{ffa0}',
        '\u{1d159}',
        '\u{1d173}',
        '\u{1d174}',
        '\u{1d175}',
        '\u{1d176}',
        '\u{1d177}',
        '\u{1d178}',
        '\u{1d179}',
        '\u{1d17a}',
        '\u{e0020}',
    ] {
        let text = format!("prefix{character}target{character}suffix");
        assert_eq!(
            Str::excerpt(&text, "target", 1, "...").as_deref(),
            Some("...target..."),
            "{character:?}"
        );
    }
    assert_eq!(
        Str::excerpt("\u{200b}target\u{200b}", "target", 8, "...").as_deref(),
        Some("...target...")
    );
    for character in ['\u{1680}', '\u{2028}', '\u{2029}', '\u{85}'] {
        let text = format!("prefix{character}target{character}suffix");
        assert_eq!(
            Str::excerpt(&text, "target", 1, "...").as_deref(),
            Some("...target...")
        );
    }
    assert_eq!(
        Str::excerpt("prefix\u{1c}target\u{1c}suffix", "target", 1, "...").as_deref(),
        Some("...\u{1c}target\u{1c}...")
    );
    assert_eq!(
        Str::excerpt("α\u{200b}target\u{200b}ω", "target", 2, "...").as_deref(),
        Some("α\u{200b}target\u{200b}ω"),
        "interior invisible characters stay"
    );
}

#[test]
fn excerpt_handles_literal_phrases_empty_values_and_extreme_radii() {
    assert_eq!(
        Str::excerpt("a [b] c [b]", "[b]", 0, "...").as_deref(),
        Some("...[b]...")
    );
    assert_eq!(
        Str::excerpt("first\nMATCH\nlast", "match", usize::MAX, "...").as_deref(),
        Some("first\nMATCH\nlast")
    );
    assert_eq!(Str::excerpt("", "", 0, "...").as_deref(), Some(""));
    assert_eq!(Str::excerpt("", "absent", usize::MAX, "..."), None);
    assert_eq!(Str::limit("世界です", 2, "…"), "世界…");
    assert_eq!(Str::limit("abc", 0, "…"), "…");
    assert_eq!(Str::mask("世界です", '★', -2, Some(1)), "世界★す");
    assert_eq!(Str::mask("abc", '*', 0, Some(0)), "abc");
}
