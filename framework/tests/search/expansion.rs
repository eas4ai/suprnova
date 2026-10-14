use suprnova::search::{
    expansion::{Vocabulary, expand, tokenize},
    query::QueryText,
    settings::{QuerySettings, Synonym},
};

fn vocabulary() -> Vocabulary {
    Vocabulary {
        words: [
            ("house", 20),
            ("horse", 12),
            ("hose", 9),
            ("hous", 8),
            ("framework", 5),
            ("frmaework", 2),
            ("2025", 20),
        ]
        .into_iter()
        .map(|(word, df)| (word.into(), df))
        .collect(),
    }
}

#[test]
fn typography_folds_case_and_diacritics() {
    assert_eq!(tokenize("CAFÉ, naïve rust!"), ["cafe", "naive", "rust"]);
}

#[test]
fn typo_budget_obeys_five_and_nine_character_thresholds() {
    let settings = QuerySettings::default();
    let four = expand(&QueryText::parse("hous"), &settings, &vocabulary(), 64).unwrap();
    assert_eq!(four.terms[0].alternatives, ["hous"]);
    let five = expand(&QueryText::parse("hosue"), &settings, &vocabulary(), 64).unwrap();
    assert!(five.terms[0].alternatives.contains(&"house".into()));
    let nine = expand(&QueryText::parse("frmaework"), &settings, &vocabulary(), 64).unwrap();
    assert!(nine.terms[0].alternatives.contains(&"framework".into()));
}

#[test]
fn numerical_and_disabled_words_are_never_corrected() {
    let mut settings = QuerySettings::default();
    settings
        .typo_tolerance
        .disable_on_words
        .insert("hosue".into());
    let result = expand(
        &QueryText::parse("2026 hosue"),
        &settings,
        &vocabulary(),
        64,
    )
    .unwrap();
    assert_eq!(result.terms[0].alternatives, ["2026"]);
    assert_eq!(result.terms[1].alternatives, ["hosue"]);
}

#[test]
fn expansions_keep_original_and_cap_corrections_by_document_frequency() {
    let result = expand(
        &QueryText::parse("house"),
        &QuerySettings::default(),
        &vocabulary(),
        2,
    )
    .unwrap();
    assert_eq!(result.terms[0].alternatives, ["house", "horse"]);
    assert!(
        expand(
            &QueryText::parse("house"),
            &QuerySettings::default(),
            &vocabulary(),
            0
        )
        .is_err()
    );
}

#[test]
fn synonyms_support_direction_and_multi_word_source_and_target() {
    let settings = QuerySettings {
        synonyms: vec![
            Synonym {
                terms: vec!["new york".into(), "nyc".into()],
                one_way: false,
            },
            Synonym {
                terms: vec!["car".into(), "motor vehicle".into()],
                one_way: true,
            },
        ],
        ..QuerySettings::default()
    };
    let forward = expand(
        &QueryText::parse("new york car"),
        &settings,
        &Vocabulary::default(),
        64,
    )
    .unwrap();
    assert_eq!(forward.terms[0].original, "new york");
    assert!(forward.terms[0].alternatives.contains(&"nyc".into()));
    assert!(
        forward.terms[1]
            .alternatives
            .contains(&"motor vehicle".into())
    );
    let reverse = expand(
        &QueryText::parse("motor vehicle"),
        &settings,
        &Vocabulary::default(),
        64,
    )
    .unwrap();
    assert!(
        !reverse
            .terms
            .iter()
            .flat_map(|term| &term.alternatives)
            .any(|term| term == "car")
    );
}

#[test]
fn prefix_phrases_exclusions_and_stop_words_keep_their_roles() {
    let mut settings = QuerySettings::default();
    settings.stop_words.insert("the".into());
    let result = expand(
        &QueryText::parse("the house rust \"red fox\" -dog"),
        &settings,
        &vocabulary(),
        64,
    )
    .unwrap();
    assert_eq!(result.terms.len(), 2);
    assert!(!result.terms[0].prefix);
    assert!(result.terms[1].prefix);
    assert_eq!(
        result.phrases,
        vec![vec!["red".to_string(), "fox".to_string()]]
    );
    assert_eq!(result.negatives, ["dog"]);
}

#[test]
fn vocabulary_drops_zero_frequencies_and_recognizes_transpositions() {
    let vocabulary = Vocabulary {
        words: [("house".into(), 2), ("hosue".into(), 0)]
            .into_iter()
            .collect(),
    };
    assert_eq!(vocabulary.within("hosue", 1), vec![("house".into(), 2)]);
}

#[test]
fn attribute_disables_retain_correction_provenance_without_disabling_synonyms() {
    let mut settings = QuerySettings::default();
    settings
        .typo_tolerance
        .disable_on_attributes
        .insert("sku".into());
    settings.synonyms.push(Synonym {
        terms: vec!["house".into(), "horse".into()],
        one_way: false,
    });
    let result = expand(&QueryText::parse("house"), &settings, &vocabulary(), 64).unwrap();
    assert!(result.typo_disabled_attributes.contains("sku"));
    assert!(result.terms[0].typo_alternatives.contains(&"hose".into()));
    assert!(!result.terms[0].typo_alternatives.contains(&"horse".into()));
    assert!(!result.terms[0].typo_alternatives.contains(&"house".into()));
}

#[test]
fn quoted_synonym_sources_remain_one_phrase_or_group_and_respect_direction() {
    let mut settings = QuerySettings::default();
    settings.synonyms.push(Synonym {
        terms: vec!["new york".into(), "nyc".into()],
        one_way: true,
    });
    let expanded = expand(
        &QueryText::parse("\"new york\""),
        &settings,
        &Vocabulary::default(),
        64,
    )
    .unwrap();
    assert!(expanded.phrases.is_empty());
    assert_eq!(expanded.terms[0].alternatives, ["new york", "nyc"]);
    assert!(!expanded.terms[0].prefix);
    let reverse = expand(
        &QueryText::parse("\"nyc\""),
        &settings,
        &Vocabulary::default(),
        64,
    )
    .unwrap();
    assert!(reverse.terms.is_empty());
    assert_eq!(reverse.phrases, vec![vec!["nyc".to_string()]]);
    let capped = expand(
        &QueryText::parse("\"new york\""),
        &settings,
        &Vocabulary::default(),
        1,
    )
    .unwrap();
    assert_eq!(capped.terms[0].alternatives, ["new york"]);
}

#[test]
fn unicode_transliterations_produce_idempotent_tokens_across_scripts() {
    for text in ["中国 北京", "한국 대한민국", "中国한국", "ℹ", "¼"] {
        let words = tokenize(text);
        assert_eq!(tokenize(&words.join(" ")), words, "{text}");
    }
    assert_eq!(tokenize("ℹ"), ["information", "source"]);
    assert_eq!(tokenize("¼"), ["1", "4"]);
}

#[test]
fn numbers_disable_typos_while_alphanumeric_words_retain_their_budget() {
    let vocabulary = Vocabulary {
        words: [("part8".into(), 4), ("12346".into(), 4)]
            .into_iter()
            .collect(),
    };
    let expanded = expand(
        &QueryText::parse("part7 12345"),
        &QuerySettings::default(),
        &vocabulary,
        64,
    )
    .unwrap();
    assert!(expanded.terms[0].alternatives.contains(&"part8".into()));
    assert_eq!(expanded.terms[1].alternatives, ["12345"]);
}

#[test]
fn typo_thresholds_count_source_characters_before_diacritic_expansion() {
    let vocabulary = Vocabulary {
        words: [("assast".into(), 4)].into_iter().collect(),
    };
    let expanded = expand(
        &QueryText::parse("äßäß rust"),
        &QuerySettings::default(),
        &vocabulary,
        64,
    )
    .unwrap();
    assert_eq!(expanded.terms[0].original, "assass");
    assert_eq!(expanded.terms[0].alternatives, ["assass"]);
    assert!(expanded.terms[0].typo_alternatives.is_empty());
}
