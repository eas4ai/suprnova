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
}
