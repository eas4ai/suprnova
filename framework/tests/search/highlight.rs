use suprnova::search::{
    expansion::{Vocabulary, expand},
    highlight::{highlight, validate_tags},
    query::QueryText,
    settings::{QuerySettings, Synonym},
};

fn mark_settings() -> QuerySettings {
    QuerySettings {
        highlight_pre_tag: "<mark>".into(),
        highlight_post_tag: "</mark>".into(),
        ..QuerySettings::default()
    }
}

#[test]
fn document_markup_is_escaped_before_matches_receive_trusted_tags() {
    let settings = mark_settings();
    let expanded = expand(
        &QueryText::parse("script"),
        &settings,
        &Vocabulary::default(),
        64,
    )
    .unwrap();
    let formatted = highlight("<script>alert('x')</script>", &expanded, &settings, None).unwrap();
    assert!(!formatted.contains("<script>"));
    assert!(formatted.contains("&lt;<mark>script</mark>&gt;"));
    assert!(formatted.contains("&#x27;x&#x27;"));
}

#[test]
fn typo_prefix_synonym_and_phrase_matches_keep_original_spelling() {
    let mut settings = mark_settings();
    settings.synonyms.push(Synonym {
        terms: vec!["car".into(), "motor vehicle".into()],
        one_way: false,
    });
    let vocabulary = Vocabulary {
        words: [("house".into(), 10)].into_iter().collect(),
    };
    let expanded = expand(
        &QueryText::parse("hosue car \"red fox\" caf"),
        &settings,
        &vocabulary,
        64,
    )
    .unwrap();
    let formatted = highlight(
        "House motor vehicle red fox café",
        &expanded,
        &settings,
        None,
    )
    .unwrap();
    assert_eq!(
        formatted,
        "<mark>House</mark> <mark>motor vehicle</mark> <mark>red fox</mark> <mark>café</mark>"
    );
}

#[test]
fn cropping_centers_first_match_and_never_splits_a_tag() {
    let settings = mark_settings();
    let expanded = expand(
        &QueryText::parse("extraordinary"),
        &settings,
        &Vocabulary::default(),
        64,
    )
    .unwrap();
    let formatted = highlight(
        "before before extraordinary after after",
        &expanded,
        &settings,
        Some(12),
    )
    .unwrap();
    assert!(formatted.contains("<mark>extraordinary</mark>"));
    assert!(formatted.starts_with(&settings.crop_marker));
    assert!(formatted.ends_with(&settings.crop_marker));
    assert!(highlight("text", &expanded, &settings, Some(0)).is_err());
}

#[test]
fn only_allowlisted_matching_tags_and_a_single_class_attribute_are_accepted() {
    for tag in ["mark", "em", "strong", "b", "span"] {
        validate_tags(&format!("<{tag} class=\"hit bold\">"), &format!("</{tag}>")).unwrap();
    }
    for (pre, post) in [
        ("<script>", "</script>"),
        ("<mark onclick=\"alert(1)\">", "</mark>"),
        ("<mark class=\"a\" id=\"x\">", "</mark>"),
        ("<mark>", "</em>"),
        ("<mark class=\"a&b\">", "</mark>"),
    ] {
        assert!(validate_tags(pre, post).is_err(), "{pre}");
    }
}

#[test]
fn crop_marker_is_text_and_unicode_boundaries_remain_intact() {
    let mut settings = mark_settings();
    settings.crop_marker = "<more>".into();
    let expanded = expand(
        &QueryText::parse("cafe"),
        &settings,
        &Vocabulary::default(),
        64,
    )
    .unwrap();
    let formatted = highlight("éééé café éééé", &expanded, &settings, Some(5)).unwrap();
    assert!(formatted.contains("<mark>café</mark>"));
    assert!(formatted.contains("&lt;more&gt;"));
}

#[test]
fn transliteration_tokens_highlight_their_entire_original_unicode_span() {
    let settings = mark_settings();
    for (query, text, expected) in [
        ("information", "ℹ", "<mark>ℹ</mark>"),
        ("\"information source\"", "ℹ", "<mark>ℹ</mark>"),
        ("bei", "北京", "<mark>北</mark>京"),
        ("hangug", "한국", "<mark>한국</mark>"),
    ] {
        let expanded = expand(
            &QueryText::parse(query),
            &settings,
            &Vocabulary::default(),
            64,
        )
        .unwrap();
        assert_eq!(
            highlight(text, &expanded, &settings, None).unwrap(),
            expected
        );
    }
}

#[test]
fn tag_validation_uses_html_whitespace_and_rejects_leftover_attributes() {
    for pre in [
        "<em\u{00a0}class=\"hit\">",
        "<em class=\"hit\"\u{00a0}>",
        "<em\u{000b}class=\"hit\">",
        "<em class=\"hit\"\u{000b}>",
    ] {
        assert!(validate_tags(pre, "</em>").is_err(), "{pre:?}");
    }
    for space in [' ', '\t', '\n', '\r', '\u{000c}'] {
        validate_tags(&format!("<em{space}class=\"hit\"{space}>"), "</em>").unwrap();
    }
}

#[test]
fn native_script_prefixes_keep_source_highlights_and_crop_focus() {
    use suprnova::search::{ExpandedQuery, ExpandedTerm};
    let query = ExpandedQuery {
        terms: vec![ExpandedTerm {
            original: "косм".into(),
            alternatives: vec!["косм".into()],
            prefix: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let rendered = highlight(
        "before before космос after after",
        &query,
        &mark_settings(),
        Some(10),
    )
    .unwrap();
    assert!(rendered.contains("<mark>космос</mark>"), "{rendered}");
    assert!(rendered.starts_with("..."));
}

#[test]
fn native_script_phrases_keep_source_highlights_and_crop_focus() {
    use suprnova::search::ExpandedQuery;
    let query = ExpandedQuery {
        phrases: vec![vec!["космос".into(), "планета".into()]],
        ..Default::default()
    };
    let rendered = highlight(
        "before before космос планета after after",
        &query,
        &mark_settings(),
        Some(10),
    )
    .unwrap();
    assert!(
        rendered.contains("<mark>космос планета</mark>"),
        "{rendered}"
    );
    assert!(rendered.starts_with("..."));
}
