//! PAR-035: slug, mask, limit and excerpt.

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
