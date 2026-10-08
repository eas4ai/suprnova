# Strings

`Str` holds the string helpers worth carrying over from Laravel: slugs,
masks, limits, excerpts, and plural and singular forms. For case
conversions, use the `heck` crate; for everything else, `std::str` and
`regex`. Every count here is in characters, never bytes, so a multibyte
value is never cut inside a character.

```rust
use suprnova::Str;

let slug = Str::slug("Œuvre d'art @home", "-");          // oeuvre-dart-at-home
let card = Str::mask("4111 1111 1111 1234", '*', 0, Some(-4)); // ***************1234
let teaser = Str::limit("The quick brown fox jumps", 15, "..."); // The quick brown...
let label = Str::plural("comment", 2);                  // comments
```

## Slugs

`Str::slug(title, separator)` spells the title in ASCII, lower-cases it,
writes `@` as `at`, and joins each run of other characters between
letters and digits with the separator:

```rust
use suprnova::Str;

assert_eq!(Str::slug("Laravel 5 Framework", "-"), "laravel-5-framework");
assert_eq!(Str::slug("Ünïcödé Straße", "-"), "unicode-strasse");
assert_eq!(Str::slug("Привет мир", "-"), "privet-mir");
assert_eq!(Str::slug("foo bar", "_"), "foo_bar");
```

The ASCII spelling is Laravel's own: the replacement map of
voku/portable-ascii, the package Laravel's `Str::ascii` uses, so a title
gets the slug it gets in Laravel. That map spells Latin, Greek, Cyrillic,
Arabic, and many other scripts, and drops what it can't spell, such as
Han characters and emoji: `Str::slug("北京 city", "-")` is `city`.

## Masks

`Str::mask(value, character, index, length)` replaces the characters from
`index` with `character`: `length` of them, or all to the end with `None`.
A negative index counts from the end, and a negative length stops that
many short of the end:

```rust
use suprnova::Str;

assert_eq!(Str::mask("taylor@example.com", '*', 3, None), "tay***************");
assert_eq!(Str::mask("taylor@example.com", '*', -15, Some(3)), "tay***@example.com");
```

## Limits and excerpts

`Str::limit(value, limit, end)` keeps the first `limit` characters and
adds `end` when it cut. `Str::limit_words` cuts at the last space within
the limit instead, so no word is broken:

```rust
use suprnova::Str;

assert_eq!(Str::limit("The quick brown fox", 12, "..."), "The quick br...");
assert_eq!(Str::limit_words("The quick brown fox", 12, "..."), "The quick...");
```

`Str::excerpt(text, phrase, radius, omission)` frames the first match of
a phrase, ignoring case, with up to `radius` characters on each side,
in text of any number of lines. It returns `None` when the phrase is not
there:

```rust
use suprnova::Str;

assert_eq!(
    Str::excerpt("This is my name", "my", 3, "...").as_deref(),
    Some("...is my na...")
);
```

## Plural and singular

`Str::plural(word, count)` and `Str::singular(word)` inflect by the rules
of the current locale's language, the ones Laravel's `Pluralizer` uses
from doctrine/inflector 2.1.0, the release Laravel 13 installs: English,
French, Norwegian Bokmål, Portuguese, Spanish, and Turkish. Any other
language uses the English rules. A count of 1 or -1 leaves the word as it
is, and the result keeps the word's case:

```rust
use suprnova::Str;

assert_eq!(Str::plural("child", 2), "children");
assert_eq!(Str::plural("Person", 3), "People");
assert_eq!(Str::plural("comment", 1), "comment");
assert_eq!(Str::singular("people"), "person");
```

In a request whose locale is `fr`, `Str::plural("cheval", 2)` is
`chevaux`; in `es`, `Str::plural("ciudad", 2)` is `ciudades`. Without the
`localization` feature, the rules are English.

Both read the current locale, so a page that [RenderCache](render-cache.md)
stores and that calls them must vary by it: add
`.vary(VarianceDimension::Locale)` to the route's policy. Without it,
RenderCache declines to store the page.

To choose the form a reader sees in a translated message, use the plural
categories of a Fluent message instead; see
[Localization](localization.md). `Str::plural` is for words in your own
code, such as a table or a label built from a model's name.

### Why Suprnova diverges

- **The language follows the locale.** Laravel sets the plural language
  for the whole process with `Pluralizer::useLanguage`; here the current
  request's locale chooses it, for `Str::singular` as for `Str::plural`.
- **An excerpt can span lines.** Laravel's pattern stops at a line break,
  so `Str::excerpt` returns `null` for text with a line break inside it;
  here it finds the phrase on any line.
- **Counts are characters.** Laravel's `limit` counts display width, two
  for a wide East Asian character, and its word-preserving form also
  strips HTML tags.
- **A subset.** There is no fluent `Stringable`; case conversions come
  from `heck`.

## Next

- [Localization](localization.md) - percentages, abbreviated numbers, and
  the rest of the locale-aware formatting
- [Validation](validation.md) - rules for the strings your users send
