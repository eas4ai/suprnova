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
    assert!(expanded.terms[0].required);
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

#[test]
fn correction_skips_long_tokens_and_limits_query_tokens() {
    let long = "a".repeat(65);
    let near = format!("{}b", "a".repeat(64));
    let vocabulary = Vocabulary {
        words: [(near, 2), ("house".into(), 1)].into_iter().collect(),
    };
    let expanded = expand(
        &QueryText::parse(&format!("{long} {}", vec!["hosue"; 33].join(" "))),
        &QuerySettings::default(),
        &vocabulary,
        64,
    )
    .unwrap();
    assert_eq!(expanded.terms[0].alternatives, [long]);
    assert!(expanded.terms[31].alternatives.contains(&"house".into()));
    assert_eq!(expanded.terms[33].alternatives, ["hosue"]);
}

#[test]
fn vocabulary_prunes_long_candidates_and_short_circuits_exact_matches() {
    let long = "a".repeat(65);
    let vocabulary = Vocabulary {
        words: [
            (long.clone(), 2),
            (format!("{}b", "a".repeat(64)), 3),
            ("z".repeat(100_000), 1),
            ("house".into(), 1),
        ]
        .into_iter()
        .collect(),
    };
    // Oversized tokens still retain their exact identity without distance work.
    assert_eq!(vocabulary.within(&long, 1), vec![(long, 2)]);
    assert_eq!(vocabulary.within("hosue", 1), vec![("house".into(), 1)]);
    let boundary = "a".repeat(64);
    let candidate = format!("{boundary}b");
    let boundary_vocabulary = Vocabulary {
        words: [(candidate.clone(), 1)].into_iter().collect(),
    };
    assert_eq!(
        boundary_vocabulary.within(&boundary, 1),
        vec![(candidate, 1)]
    );
    let huge = "a".repeat(100_000);
    let vocabulary = Vocabulary {
        words: [(huge.clone(), 1)].into_iter().collect(),
    };
    assert_eq!(vocabulary.within(&huge, 2), vec![(huge, 1)]);
}

#[test]
fn expansion_rejects_excessive_query_size() {
    assert!(
        expand(
            &QueryText::parse(&"x".repeat(65_537)),
            &QuerySettings::default(),
            &Vocabulary::default(),
            64
        )
        .is_err()
    );
}

#[test]
fn overflowing_numeric_tokens_do_not_receive_corrections() {
    let vocabulary = Vocabulary {
        words: [
            ("1e998".into(), 10),
            ("nanx".into(), 10),
            ("infx".into(), 10),
        ]
        .into_iter()
        .collect(),
    };
    let mut settings = QuerySettings::default();
    settings.typo_tolerance.one_edit_min_length = 1;
    for token in ["1e999", "NaN", "inf", "Infinity", "12345"] {
        let expanded = expand(&QueryText::parse(token), &settings, &vocabulary, 64).unwrap();
        assert_eq!(expanded.terms[0].alternatives, [token.to_lowercase()]);
    }
}

// Keep the old scan independent of the candidate index as an equivalence oracle.
fn scan(vocabulary: &Vocabulary, word: &str, budget: usize) -> Vec<(String, u64)> {
    let length = word.chars().take(65).count();
    let mut matches: Vec<_> = vocabulary
        .words
        .iter()
        .filter(|(candidate, df)| {
            if **df == 0 {
                return false;
            }
            if word == candidate.as_str() {
                return true;
            }
            if length > 64 || budget == 0 {
                return false;
            }
            let candidate_length = candidate
                .chars()
                .take(length.saturating_add(budget).saturating_add(1))
                .count();
            length.abs_diff(candidate_length) <= budget
                && strsim::damerau_levenshtein(word, candidate) <= budget
        })
        .map(|(word, df)| (word.clone(), *df))
        .collect();
    matches.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    matches
}

#[test]
fn indexed_candidates_equal_scan_for_random_unicode_vocabularies() {
    use rand::{RngExt, SeedableRng, rngs::StdRng};
    let mut rng = StdRng::seed_from_u64(0x5c1fac7);
    let alphabet = ['a', 'b', 'c', 'd', 'é', '界'];
    for _ in 0..16 {
        let mut vocabulary = Vocabulary::default();
        for _ in 0..128 {
            let word = (0..rng.random_range(0..14))
                .map(|_| alphabet[rng.random_range(0..alphabet.len())])
                .collect::<String>();
            vocabulary.words.insert(word, rng.random_range(0..8));
        }
        let mut queries = vocabulary.words.keys().cloned().collect::<Vec<_>>();
        for source in vocabulary.words.keys() {
            let mut chars: Vec<_> = source.chars().collect();
            if chars.len() >= 2 {
                let at = rng.random_range(0..chars.len() - 1);
                chars.swap(at, at + 1);
            }
            // A transposition plus an insertion exercises unrestricted Damerau distance.
            chars.insert(
                rng.random_range(0..=chars.len()),
                alphabet[rng.random_range(0..alphabet.len())],
            );
            queries.push(chars.into_iter().collect());
        }
        for _ in 0..64 {
            queries.push(
                (0..rng.random_range(0..14))
                    .map(|_| alphabet[rng.random_range(0..alphabet.len())])
                    .collect(),
            );
        }
        for query in queries {
            for budget in 0..=2 {
                assert_eq!(
                    vocabulary.within(&query, budget),
                    scan(&vocabulary, &query, budget),
                    "query {query:?}, budget {budget}"
                );
            }
        }
    }
}

#[test]
fn vocabulary_cache_survives_clone_serde_and_map_mutations() {
    let mut original = vocabulary();
    original.within("hosue", 2);
    let clone = original.clone();
    original.words.remove("house");
    original.words.insert("hogue".into(), 40);
    original.words.insert("horse".into(), 0);
    for snapshot in [&original, &clone] {
        assert_eq!(snapshot.within("hosue", 2), scan(snapshot, "hosue", 2));
        let serialized = serde_json::to_value(snapshot).unwrap();
        assert_eq!(serialized, serde_json::json!({"words": &snapshot.words}));
        let decoded: Vocabulary = serde_json::from_value(serialized).unwrap();
        assert_eq!(&decoded, snapshot);
        assert_eq!(decoded.within("hosue", 2), scan(snapshot, "hosue", 2));
    }
}

#[test]
fn indexed_candidates_preserve_repeated_scalars_and_correction_boundaries() {
    let vocabulary = Vocabulary {
        words: [
            ("ca".into(), 1),
            ("abc".into(), 2),
            ("aabbaa".into(), 3),
            ("界".repeat(63), 4),
            ("界".repeat(64), 5),
            ("界".repeat(65), 6),
            ("界".repeat(66), 7),
            ("界".repeat(67), 8),
            (String::new(), 9),
        ]
        .into_iter()
        .collect(),
    };
    for query in [
        "ca".to_owned(),
        "abc".into(),
        "ababaa".into(),
        "界".repeat(64),
        "界".repeat(65),
        String::new(),
    ] {
        for budget in 0..=3 {
            assert_eq!(
                vocabulary.within(&query, budget),
                scan(&vocabulary, &query, budget)
            );
        }
    }
}

#[cfg(all(feature = "testing", target_os = "linux"))]
fn resident_kib() -> Result<usize, suprnova::FrameworkError> {
    let status = std::fs::read_to_string("/proc/self/status")
        .map_err(|error| suprnova::FrameworkError::internal(error.to_string()))?;
    status
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse().ok())
        })
        .ok_or_else(|| suprnova::FrameworkError::internal("missing VmRSS"))
}

#[test]
#[cfg(feature = "testing")]
#[ignore = "manual SciFact cold index and warm expansion measurement"]
fn scifact_expansion_measurement() -> Result<(), suprnova::FrameworkError> {
    use std::{collections::BTreeMap, path::PathBuf, time::Instant};
    use suprnova::search::baseline::Fixture;
    let fixture =
        Fixture::load(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/search/fixtures"))?;
    let mut words = BTreeMap::new();
    for document in &fixture.corpus {
        let unique = tokenize(&format!("{} {}", document.title, document.text))
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        for word in unique {
            *words.entry(word).or_insert(0) += 1;
        }
    }
    let vocabulary = Vocabulary {
        words: words.into(),
    };
    #[cfg(target_os = "linux")]
    let before_kib = resident_kib()?;
    let started = Instant::now();
    vocabulary.within("framework", 2);
    let cold = started.elapsed();
    #[cfg(target_os = "linux")]
    eprintln!(
        "SciFact index RSS growth: {} KiB",
        resident_kib()?.saturating_sub(before_kib)
    );
    let started = Instant::now();
    for query in &fixture.queries {
        expand(
            &QueryText::parse(&query.text),
            &QuerySettings::default(),
            &vocabulary,
            64,
        )?;
    }
    let warm = started.elapsed();
    eprintln!(
        "SciFact: {} words, cold index {:?}, 300 warm expansions {:?}",
        vocabulary.words.len(),
        cold,
        warm
    );
    assert!(
        warm.as_secs_f64() < 10.0,
        "300 expansions exceeded ten seconds: {warm:?}"
    );
    Ok(())
}
