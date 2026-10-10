# Strings

`Str` holds the string helpers worth carrying over from Laravel: slugs,
masks, limits, excerpts, plural and singular forms, and Markdown.
`Str::of` wraps a value in a `Stringable` that chains them. For case
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
assert_eq!(Str::slug_in("Ärger", "-", "de"), "aerger");
```

The ASCII spelling is Laravel's own: the replacement map of
voku/portable-ascii, the package Laravel's `Str::ascii` uses, so a title
gets the slug it gets in Laravel. That map spells Latin, Greek, Cyrillic,
Arabic, and many other scripts, and drops what it can't spell, such as
Han characters and emoji: `Str::slug("北京 city", "-")` is `city`.
You choose the language's spelling with
`Str::slug_in(title, separator, language)`. With German, you get `ae` for
`ä`; with Esperanto, you get `cx` for `ĉ`. You pass an empty language to
keep Unicode letters instead of transliterating them.

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

You keep the first `words` space-separated runs with
`Str::words(value, words, end)` and add `end` when you cut. You count
markup as part of a word and keep whitespace between words. You pass a
zero word limit to leave the value as it is:

```rust
use suprnova::Str;

assert_eq!(
    Str::words("Perfectly balanced, as all things should be.", 3, " >>>"),
    "Perfectly balanced, as >>>"
);
assert_eq!(Str::words("<b>bold</b> text here", 2, "..."), "<b>bold</b> text...");
```

`Str::excerpt(text, phrase, radius, omission)` frames the first match of
a phrase, ignoring case, with up to `radius` characters on each side,
in text of any number of lines. It returns `None` when the phrase is not
there. You trim Laravel's invisible characters, including zero-width
spaces, from the ends you cut:

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
Esperanto, French, Italian, Norwegian Bokmål, Portuguese, Spanish, and
Turkish. Any other language uses the English rules. A count of 1 or -1
leaves the word as it is, and the result keeps the word's case:

```rust
use suprnova::Str;

assert_eq!(Str::plural("child", 2), "children");
assert_eq!(Str::plural("Person", 3), "People");
assert_eq!(Str::plural("comment", 1), "comment");
assert_eq!(Str::singular("people"), "person");
assert_eq!(Str::plural_studly("VerifiedHuman", 2), "VerifiedHumans");
assert_eq!(Str::plural_pascal("VerifiedHuman", 2), "VerifiedHumans");
assert_eq!(Str::plural_with_count("car", 3), "3 cars");
```

In a request whose locale is `fr`, `Str::plural("cheval", 2)` is
`chevaux`; in `es`, `Str::plural("ciudad", 2)` is `ciudades`. Without the
`localization` feature, the rules are English.

You inflect the last word of the value with `Str::plural_studly` or
`Str::plural_pascal`. You put the count before the inflected word with
`Str::plural_with_count(word, count)`, using the current locale's integer
format. You keep all digits even for the largest and smallest `i64`
values. Without `localization`, you get plain decimal digits.

You vary a page that [RenderCache](render-cache.md) stores by its locale
when you call these inflection helpers: add
`.vary(VarianceDimension::Locale)` to the route's policy. Without it,
RenderCache declines to store the page.

To choose the form a reader sees in a translated message, use the plural
categories of a Fluent message instead; see
[Localization](localization.md). `Str::plural` is for words in your own
code, such as a table or a label built from a model's name.

## Markdown

`Str::markdown(value, &renderer)` renders a whole Markdown document to
HTML, and `Str::inline_markdown(value, &renderer)` renders the inline
syntax only: emphasis, code, links and line breaks, with no paragraph
around it and block markers such as `#`, `>` or `-` kept as text. You
pass the `MarkdownRenderer` (from `suprnova::content`) that decides what
raw HTML and links may do:

```rust
use suprnova::Str;
use suprnova::content::MarkdownRenderer;

let renderer = MarkdownRenderer::default();
let title = Str::inline_markdown("**Laravel** _rocks_", &renderer)?;
assert_eq!(title, "<strong>Laravel</strong> <em>rocks</em>");
assert_eq!(Str::inline_markdown("# Title", &renderer)?, "# Title");
let page = Str::markdown("# Laravel\n\nA *framework*.", &renderer)?;
assert!(page.contains("<p>A <em>framework</em>.</p>"));
# Ok::<(), suprnova::content::ContentError>(())
```

The default renderer drops raw HTML and sanitizes its output, so
`<script>` never reaches the page. You choose otherwise with the
renderer's builder calls, which hold Laravel's CommonMark options:

```rust
use suprnova::Str;
use suprnova::content::{HtmlInput, MarkdownRenderer};

let renderer = MarkdownRenderer::default()
    .html_input(HtmlInput::Strip)
    .allow_unsafe_links(false)
    .autolink(true);
let html = Str::inline_markdown("Inject: <script>alert(\"XSS\");</script>", &renderer)?;
assert_eq!(html, "Inject: alert(&quot;XSS&quot;);");
# Ok::<(), suprnova::content::ContentError>(())
```

- `html_input(HtmlInput::Sanitize)` keeps raw HTML and passes the output
  through the sanitizer, which keeps safe tags such as `<b>` and removes
  scripts, event handlers and unsafe URLs. `HtmlInput::Strip` removes raw
  HTML tags and keeps the text between them. `HtmlInput::Escape` shows raw
  HTML as text. `HtmlInput::Allow` passes it through: use it only for
  Markdown you trust.
- `allow_unsafe_links(true)` lets a Markdown link point at a
  `javascript:`, `vbscript:`, `file:` or `data:` URL. The sanitizer still
  removes such URLs, so it takes effect with `Strip`, `Escape` or `Allow`.
- `autolink(true)` turns bare URLs and email addresses into links.

A renderer that calls none of them renders as it always has: sanitized,
unless `MarkdownOptions::unsafe_html` is on.

## Fluent strings

`Str::of(value)` returns a `Stringable`, whose methods are the `Str`
helpers, so you chain them as Laravel's `Str::of($value)->...` chains:

```rust
use suprnova::Str;

let slug = Str::of("Blog Post").limit(4, "").slug("-");
assert_eq!(slug.to_string(), "blog");
let label: String = Str::of("comment").plural(3).into();
assert_eq!(label, "comments");
```

A `Stringable` converts from and into `String`, compares with `&str`, and
displays as its value. Next to the `Str` helpers it has `markdown`,
`inline_markdown`, and encryption through [`Crypt`](encryption.md):
`encrypt(purpose)`, `encrypt_for(purpose, context)`, `decrypt(purpose)`
and `decrypt_for(purpose, context)`. Every encryption names a
`CryptPurpose`, and a value sealed for one purpose does not open under
another:

```rust
use suprnova::{CryptPurpose, Str};

let sealed = Str::of("ssn-123").encrypt(CryptPurpose::Cast)?;
let opened = sealed.clone().decrypt(CryptPurpose::Cast)?;
assert_eq!(opened, "ssn-123");
assert!(sealed.decrypt(CryptPurpose::Cursor).is_err());
# Ok::<(), suprnova::FrameworkError>(())
```

`CryptPurpose::Cookie` is refused by `encrypt` and `decrypt`: a cookie
value is bound to its cookie's name, so you encrypt it with
`encrypt_for(CryptPurpose::Cookie, name)`.

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
- **A subset.** `Stringable` chains the helpers `Str` keeps, not every
  method of Laravel's `Stringable`; case conversions come from `heck`.
- **Markdown is sanitized by default.** Laravel's `Str::markdown` lets raw
  HTML through unless you pass `html_input`; here the default renderer
  drops it and sanitizes the output, and you opt into raw HTML with
  `HtmlInput::Allow`. The inline form keeps each line break of the text.
- **Every encryption names a purpose.** Laravel's `Stringable::encrypt`
  takes none; here `encrypt` takes a `CryptPurpose`, so a value encrypted
  for one use cannot be decrypted as another.

## Next

- [Localization](localization.md) - percentages, abbreviated numbers, and
  the rest of the locale-aware formatting
- [Dates](dates.md) - reading a date or a time from text
- [Validation](validation.md) - rules for the strings your users send
