//! PAR-036: plural and singular by the language of the current locale.

use suprnova::Str;

#[test]
fn english_words_inflect_and_keep_their_case() {
    assert_eq!(Str::plural("car", 2), "cars");
    assert_eq!(Str::plural("child", 2), "children");
    assert_eq!(Str::plural("person", 2), "people");
    assert_eq!(Str::plural("sheep", 2), "sheep");
    assert_eq!(Str::plural("Car", 2), "Cars");
    assert_eq!(Str::plural("CAR", 2), "CARS");
    assert_eq!(Str::plural("Person", 3), "People");
    assert_eq!(
        Str::plural("cars", 1),
        "cars",
        "a count of one leaves the word"
    );
    assert_eq!(Str::plural("car", -1), "car");
    assert_eq!(Str::plural("car", 0), "cars");
    assert_eq!(Str::plural("quiz", 2), "quizzes");
    assert_eq!(Str::plural("status", 2), "statuses");
    assert_eq!(Str::singular("people"), "person");
    assert_eq!(Str::singular("Children"), "Child");
    assert_eq!(Str::singular("statuses"), "status");
    assert_eq!(Str::singular("cars"), "car");
}

/// doctrine/inflector 2.1.0, the release Laravel 13.34.0 installs.
#[test]
fn english_follows_the_inflector_laravel_installs() {
    assert_eq!(Str::plural("die", 2), "dice");
    assert_eq!(Str::plural("stadium", 2), "stadiums");
    assert_eq!(Str::plural("alga", 2), "algae");
    assert_eq!(Str::plural("nursery", 2), "nurseries");
    assert_eq!(Str::plural("medium", 2), "media");
    assert_eq!(Str::singular("dice"), "die");
    assert_eq!(Str::singular("algae"), "alga");
    assert_eq!(Str::singular("stadiums"), "stadium");
}

#[test]
fn plural_supports_studly_pascal_and_count_prefixes() {
    assert_eq!(Str::plural_studly("VerifiedHuman", 2), "VerifiedHumans");
    assert_eq!(Str::plural_pascal("VerifiedHuman", 2), "VerifiedHumans");
    assert_eq!(Str::plural_studly("UserPerson", 2), "UserPeople");
    assert_eq!(Str::plural_pascal("APIChild", 2), "APIChildren");
    assert_eq!(Str::plural_studly("VerifiedHuman", -1), "VerifiedHuman");
    assert_eq!(Str::plural_studly("car", 2), "cars");
    assert_eq!(Str::plural_pascal("", 2), "");
    assert_eq!(Str::plural_with_count("car", 3), "3 cars");
    assert_eq!(Str::plural_with_count("cars", 1), "1 cars");
    assert_eq!(Str::plural_with_count("car", -1), "-1 car");
    assert_eq!(Str::plural_with_count("car", 0), "0 cars");
}

#[test]
fn plural_preserves_word_cases_and_leaves_non_words() {
    assert_eq!(Str::plural("iPhone", 2), "iPhones");
    assert_eq!(Str::singular("iPhones"), "iPhone");
    assert_eq!(Str::plural("New Car", 2), "New Cars");
    assert_eq!(Str::singular("New Cars"), "New Car");
    assert_eq!(Str::plural("car!", 2), "car!");
    assert_eq!(Str::plural("recommended", 2), "recommended");
    assert_eq!(Str::plural("RELATED", 2), "RELATED");
    assert_eq!(Str::plural("", 2), "");
    assert_eq!(Str::singular(""), "");
    assert_eq!(Str::plural("car", i64::MIN), "cars");
    assert_eq!(Str::plural("car", i64::MAX), "cars");
}

#[cfg(feature = "localization")]
mod languages {
    use suprnova::{Lang, Locale, Str, scope_locale};

    async fn in_locale(locale: &str, word: &str) -> String {
        let word = word.to_owned();
        scope_locale(Locale::parse(locale).unwrap(), async move {
            Str::plural(&word, 2)
        })
        .await
    }

    #[tokio::test]
    async fn each_language_uses_its_own_rules() {
        assert_eq!(in_locale("fr", "cheval").await, "chevaux");
        assert_eq!(in_locale("fr-CA", "journal").await, "journaux");
        assert_eq!(in_locale("es", "ciudad").await, "ciudades");
        assert_eq!(in_locale("es-MX", "lápiz").await, "lápices");
        assert_eq!(in_locale("pt-BR", "cão").await, "cães");
        assert_eq!(in_locale("pt", "avião").await, "aviões");
        assert_eq!(in_locale("nb", "bil").await, "biler");
        assert_eq!(in_locale("tr", "kitap").await, "kitaplar");
        assert_eq!(in_locale("tr", "ev").await, "evler");
        assert_eq!(in_locale("it", "gatto").await, "gatti");
        assert_eq!(in_locale("eo", "hundo").await, "hundoj");
        assert_eq!(in_locale("en-GB", "child").await, "children");
        assert_eq!(
            in_locale("de", "car").await,
            "cars",
            "a language with no rules uses English"
        );
    }

    #[tokio::test]
    async fn singular_follows_the_locale_too() {
        let singular = scope_locale(Locale::parse("fr").unwrap(), async {
            assert_eq!(Lang::locale().as_str(), "fr");
            Str::singular("chevaux")
        })
        .await;
        assert_eq!(singular, "cheval");
    }

    #[tokio::test]
    async fn french_follows_the_inflector_laravel_installs() {
        let singulars = scope_locale(Locale::parse("fr").unwrap(), async {
            ["locaux", "bois", "mas"].map(Str::singular)
        })
        .await;
        assert_eq!(singulars, ["local", "bois", "mas"]);
        assert_eq!(in_locale("fr", "bois").await, "bois");
        assert_eq!(in_locale("fr", "CHEVAL").await, "CHEVAUX");
    }

    #[tokio::test]
    async fn italian_uses_irregular_uninflected_and_ordered_transformation_rules() {
        scope_locale(Locale::parse("it").unwrap(), async {
            for (singular, plural) in [
                ("uomo", "uomini"),
                ("uovo", "uova"),
                ("bue", "buoi"),
                ("amico", "amici"),
                ("braccio", "braccia"),
                ("valigia", "valigie"),
                ("mille", "mila"),
                ("studio", "studi"),
                ("auto", "auto"),
                ("crisi", "crisi"),
                ("virtù", "virtù"),
                ("film", "film"),
            ] {
                assert_eq!(Str::plural(singular, 2), plural, "{singular}");
                assert_eq!(Str::singular(plural), singular, "{plural}");
            }
            assert_eq!(Str::plural("fascia", 2), "fasce");
            assert_eq!(Str::singular("fasce"), "fascia");
            assert_eq!(Str::plural("lago", 2), "laghi");
            assert_eq!(Str::singular("laghi"), "lago");
            assert_eq!(Str::plural("GATTO", 2), "GATTI");
            assert_eq!(Str::plural("Gatto", 2), "Gatti");
            assert_eq!(Str::plural_studly("VerifiedGatto", 2), "VerifiedGatti");
        })
        .await;
    }

    #[tokio::test]
    async fn esperanto_inflects_only_the_doctrine_noun_ending() {
        scope_locale(Locale::parse("eo").unwrap(), async {
            assert_eq!(Str::plural("Hundo", 2), "Hundoj");
            assert_eq!(Str::plural("HUNDO", 2), "HUNDOJ");
            assert_eq!(Str::singular("HUNDOJ"), "HUNDO");
            assert_eq!(Str::plural("hundoj", 2), "hundoj");
            assert_eq!(Str::plural("bela", 2), "bela");
            assert_eq!(Str::singular("hundo"), "hundo");
            assert_eq!(Str::plural("hundo", -1), "hundo");
            assert_eq!(Str::plural_pascal("VerifiedHundo", 2), "VerifiedHundoj");
        })
        .await;
    }

    #[tokio::test]
    async fn count_prefix_formats_the_exact_integer_in_the_current_locale() {
        scope_locale(Locale::parse("de").unwrap(), async {
            assert_eq!(Str::plural_with_count("car", 1234), "1.234 cars");
            assert_eq!(
                Str::plural_with_count("car", i64::MAX),
                "9.223.372.036.854.775.807 cars"
            );
            assert_eq!(
                Str::plural_with_count("car", i64::MIN),
                "-9.223.372.036.854.775.808 cars"
            );
        })
        .await;
        scope_locale(Locale::parse("it").unwrap(), async {
            assert_eq!(Str::plural_with_count("gatto", 3), "3 gatti");
        })
        .await;
    }
}
