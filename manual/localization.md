# Localization

Localization in Suprnova is one module with four faces: message catalogs
on the server, validation errors that arrive already translated, the
*same* catalog bytes handed to the browser, and locale-aware number,
date, and list formatting. The message format is
[Fluent](https://projectfluent.org) - Mozilla's `.ftl`, the one Firefox
ships - and the whole subsystem is on by default behind the
`localization` feature.

The shortest possible tour. Write a catalog:

```ftl
# lang/en/app.ftl
welcome = Welcome to { $app }!
```

```ftl
# lang/es/app.ftl
welcome = ¡Bienvenido a { $app }!
```

Use it from a handler:

```rust
use suprnova::{__, handler, HttpResponse, Request, Response};

#[handler]
pub async fn greet(_req: Request) -> Response {
    Ok(HttpResponse::text(__!("welcome", app: "Suprnova")))
}
```

A request with `Accept-Language: es` gets the Spanish string, because
`LocaleMiddleware` resolved the locale before your handler ran. Nothing
else in the handler changes - no locale parameter threaded through, no
`&Translator` in the signature.

## Why localization

Three reasons this is a framework concern rather than a crate you pick:

- **Validation messages are the framework's strings, not yours.** "The
  email field is required." is emitted deep inside `Rule::passes`, far
  from any code you own. Unless the framework carries a translation
  seam, a Spanish app ships English validation errors - or you wrap
  every rule by hand. Suprnova's built-in rules return *keyed* messages;
  you translate them by dropping a `.ftl` file in, and never touch the
  rules.
- **The browser needs the same strings.** An Inertia app renders half
  its text in Rust and half in Svelte/React/Vue. Two translation systems
  means two file formats, two review workflows, and two chances for the
  same sentence to drift. Suprnova serves the exact catalog the server
  resolved from `/_suprnova/lang/<locale>.ftl`, and the starter kits
  parse it with `@fluent/bundle` - one set of files, one source of truth.
- **Plurals and formats are CLDR data, not string concatenation.**
  English has two plural categories, Russian and Polish four, Arabic six.
  A number is `1,234.56` in `en-US` and `1.234,56` in `de-DE`. Fluent selects on
  CLDR plural categories and ICU4X does the formatting, so neither is
  something you hand-roll per locale.

Turning the feature off (`--no-default-features`) is supported: the
localization module doesn't compile, and validation renders its embedded
English fallback strings. Nothing else changes shape.

## File layout

Catalogs live under `lang/`, one directory per locale:

```
myapp/
├── lang/
│   ├── en/
│   │   ├── app.ftl
│   │   └── validation.ftl
│   └── es/
│       ├── app.ftl
│       └── validation.ftl
├── src/
└── frontend/
```

The rules:

- **A directory name is a BCP-47 locale** - `en`, `en-GB`, `pt-BR`,
  `zh-Hans`. A directory whose name doesn't parse is skipped with a
  `warn!` rather than failing boot.
- **Every `.ftl` in a locale directory merges into one catalog**, in
  sorted filename order. Split by feature (`auth.ftl`, `billing.ftl`,
  `emails.ftl`) as much as you like - message ids are global within the
  locale, so `auth.ftl` and `billing.ftl` must not define the same id.
- **The framework's own English validation catalog loads first**, into
  every locale's bundle. Your files load over it, and a later definition
  wins. That is the whole override mechanism: define `validation-min` in
  `lang/es/validation.ftl` and the Spanish bundle uses yours.
- **The root is `lang_path()`** - `<APP_BASE_PATH>/lang`. Set
  `APP_BASE_PATH` when the binary runs from somewhere other than the
  project root (a systemd unit, a container with a different
  `WorkingDirectory`), or call `use_lang_path("…")` to move only the
  `lang` directory. See [Environment Variables](env-vars.md).
- **A missing `lang/` directory is not an error.** A fresh app must
  boot, so the translator comes up with the embedded English catalog and
  nothing else. A *malformed* `.ftl` is a different story: parse errors
  fail boot, naming the file and what the parser objected to, because a
  silently half-loaded catalog is worse than a stopped process.
- **In `local` and `development`, catalogs hot-reload.** Each request
  stats `lang/` and reparses only when something actually changed, so
  editing a `.ftl` shows up on the next refresh. Production never
  re-stats; catalogs are read once at boot.

## FTL in five minutes

Fluent is a small format. This section is everything you need for a
typical app.

**Messages** are `id = value` pairs. Ids are kebab-case by convention
(the framework's own are), values run to end of line, and indented
continuation lines are joined:

```ftl
# A comment. Attached to the message below it.
sign-in = Sign in
password-hint =
    Use at least 12 characters. A passphrase of a few
    ordinary words beats a short string of symbols.
```

**Arguments** are `{ $name }` placeables. You supply them at call time;
missing arguments are an error, not an empty string (`Lang::get` then
falls through its chain - see [The `Lang` facade](#the-lang-facade)):

```ftl
greeting = Hello, { $name }!
invoice-line = { $qty } × { $item }
```

**Terms** start with `-`, are private to the catalog, and exist so a
brand name or a repeated phrase lives in one place:

```ftl
-product-name = Suprnova
about = About { -product-name }
footer = © 2026 { -product-name }. All rights reserved.
```

**Selectors** are Fluent's conditional. The selector value is matched
against variant keys; exactly one variant is marked default with `*`:

```ftl
cart-summary =
    { $count ->
        [0] Your cart is empty.
        [one] One item in your cart.
       *[other] { $count } items in your cart.
    }
```

`[0]` matches the literal number zero. `[one]` and `[other]` are **CLDR
plural categories**, resolved for the bundle's locale - which is where
Fluent earns its place. English has two categories; Russian has four,
and a Russian translator writes all four without you changing a line of
Rust:

```ftl
# lang/ru/app.ftl
unread-messages =
    { $count ->
        [one] У вас { $count } непрочитанное сообщение.
        [few] У вас { $count } непрочитанных сообщения.
        [many] У вас { $count } непрочитанных сообщений.
       *[other] У вас { $count } непрочитанного сообщения.
    }
```

CLDR assigns `1`, `21`, `31` to `one`; `2`-`4`, `22`-`24` to `few`;
`0`, `5`-`20`, `25`-`30` to `many`; and fractions to `other`. The same
`__!("unread-messages", count: 22)` call renders correctly in English,
Russian, Polish, and Arabic, because the category selection is data, not
code.

**Always put the `*` on `other`.** It is the one category CLDR defines
for every locale, so it is the only variant guaranteed to exist - and
the default is what an unmatched selector value falls through to,
including any non-integer count. Marking `*[many]` (or any other
category) as the default sends fractions to text written for whole
numbers.

> **Pass counts as numbers.** `__!("unread-messages", count: 3)` sends a
> JSON number and selects a plural category. `count: "3"` sends a
> string, which can only match a literal variant key - it will land on
> your `*[other]` default. This is the one FTL trap worth memorising.

**Functions** are called inside placeables. Two are registered:
`NUMBER()` (Fluent's builtin) and `DATETIME()` (Suprnova's):

```ftl
score = Your score is { NUMBER($points) } out of { NUMBER($total) }.
published = Published { DATETIME($when, dateStyle: "medium") }
```

See [Locale-aware formatting](#locale-aware-formatting) for both.

**One deliberate limitation:** Suprnova resolves flat message *values*
only. Fluent's attribute syntax (`login .placeholder = …`) parses but is
not addressable through `Lang::get`, so keep one id per string:
`login-placeholder`, not `login.placeholder`. Ids are a flat namespace
per locale - prefix them (`auth-login-title`, `billing-invoice-due`)
rather than reaching for a hierarchy the resolver doesn't have.

## The `Lang` facade

`Lang` is the server-side entry point. Every method reads the **current
locale**, which the middleware bound for this request.

| Method | Returns | Notes |
|---|---|---|
| `Lang::get(key)` | `String` | Infallible. Runs the fallback chain, then returns the key itself |
| `Lang::get_with(key, args)` | `String` | Same, with arguments |
| `Lang::try_get(key)` | `Result<String, FrameworkError>` | Errors instead of degrading |
| `Lang::try_get_with(key, args)` | `Result<String, FrameworkError>` | Same, with arguments |
| `Lang::has(key)` | `bool` | Whether the key resolves for the current locale, or anywhere along its fallback chain |
| `Lang::locale()` | `Locale` | The current locale |
| `Lang::set_locale(locale)` | `()` | Change it for the rest of this request |
| `Lang::available_locales()` | `Vec<Locale>` | Every locale with a loaded catalog |

```rust
use suprnova::{Lang, Locale, TranslateArgs};

let subject = Lang::get("password-reset-subject");

let mut args = TranslateArgs::new();
args.insert("name".into(), serde_json::json!("Ada"));
args.insert("count".into(), serde_json::json!(3));
let body = Lang::get_with("unread-messages", args);

if Lang::has("beta-banner") {
    // Only some locales ship the banner copy.
}

let locales: Vec<String> = Lang::available_locales()
    .iter()
    .map(Locale::as_str)
    .collect();
```

`TranslateArgs` is an ordered map of `String` to `serde_json::Value`,
both re-exported from the crate root. Fluent arguments are strings and
numbers; other JSON shapes are stringified.

### The fallback chain

`Lang::get` never fails, and it never returns an empty string. In order:

1. The **current locale's** catalog.
2. Its **configured fallback parents** (see [Fallback
   chains](#fallback-chains)), walked transitively, if any are
   configured - `pt-PT` before `pt-BR` before whatever `pt-BR` itself
   names as a parent, and so on.
3. The **fallback locale's** catalog (`APP_FALLBACK_LOCALE`, default
   `en`), unless it already appeared earlier in this chain.
4. The **key itself**, plus one `tracing::warn!` per missing
   `(locale, key)` pair - once, not once per request, so a missing key
   in a hot path doesn't drown your logs.

Step 4 is why a missing translation renders `checkout-submit` in the
button instead of a blank button: a visibly wrong string is a bug report
waiting to happen, while an empty one is a mystery.

When you'd rather know than degrade, use the `try_*` siblings. They run
steps 1 through 3 and return `Err` instead of doing step 4:

```rust
use suprnova::Lang;

// A missing key here means a broken email - fail the job, don't send
// a message with a raw key in the subject line.
let subject = Lang::try_get("invoice-paid-subject")?;
```

### The `__!` macro

`__!` is the Laravel-muscle-memory shorthand. With no arguments it calls
`Lang::get`; with named arguments it builds a `TranslateArgs` and calls
`Lang::get_with`:

```rust
use suprnova::__;

let plain = __!("welcome-back");
let greeted = __!("greeting", name: "Ada");
let counted = __!("unread-messages", name: "Ada", count: 3);
```

Argument values are anything that converts into a
`serde_json::Value` - `&str`, `String`, integers, floats, `bool`. The
macro is exported at the crate root, so `suprnova::__!("welcome-back")`
works without the import when you'd rather not bring `__` into scope.

## Fallback chains

`APP_FALLBACK_LOCALE` is one global net under every locale. Sometimes
that's not enough: European Portuguese and Brazilian Portuguese share
nearly everything and diverge on a handful of words
(`ficheiro`/`arquivo`, `utilizador`/`usuário`, `tu`/`você`), and
maintaining two complete catalogs means every new string has to be
written twice. A **fallback parent** lets `pt-PT` inherit from `pt-BR`
before `pt-BR` falls further back to the global `fallback_locale` - so
`lang/pt-PT/` only has to hold the strings that are actually different.

### Configuring parents

One environment variable, comma-separated `child=parent` pairs:

```env
APP_LOCALE_PARENTS=pt-PT=pt-BR
```

Or the builder, one call per pair, chainable:

```rust
use suprnova::{Config, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .parent(
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

Both paths feed the same map (`LocalizationConfig::parents`), and both
are validated at boot, not at request time:

- A pair with no `=`, or an empty child or parent, is a malformed
  `APP_LOCALE_PARENTS` entry - boot fails naming the bad segment.
- A locale invalid as BCP-47 on either side of the pair fails the same
  way.
- Naming the same child twice is ambiguous config, not last-wins - boot
  fails naming the duplicate child.
- **A cycle fails boot.** The error spells out the cycle: two locales
  naming each other (`pt-PT=pt-BR,pt-BR=pt-PT`) produces
  `` `pt-PT` -> `pt-BR` -> `pt-PT` ``. A locale naming itself as its own
  parent (`pt-PT=pt-PT`) is the same case in miniature -
  `` `pt-PT` -> `pt-PT` ``. (Two code paths raise this error: parsing
  `APP_LOCALE_PARENTS` - so any app whose config goes through
  `LocalizationConfig::from_env()` fails at config load - and
  `FluentTranslator`'s catalog load, which catches a cyclic map built
  programmatically with `.parent(...)`. Only an app that builds its
  config entirely by hand *and* binds its own custom `Translator` in
  `bootstrap_fn` skips both; `Lang`'s walk is guarded independently and
  still terminates safely there, it just won't get the loud boot-time
  error.)

The builder's `.parent(child, parent)` is last-write-wins for a repeated
child - a later call overriding an earlier one is just a later
override, not the ambiguous-input case `APP_LOCALE_PARENTS` guards
against.

### Resolution order

A chain can be more than one hop long: `pt-PT` names `pt-BR` as its
parent, and `pt-BR` can in turn name a parent of its own.
`Lang::get` / `try_get` / `get_with` / `try_get_with` / `has` all walk
the whole thing, current locale first:

1. The **current locale's** catalog.
2. Its **configured parent**, then *that* locale's configured parent,
   transitively, until a locale with no configured parent is reached.
3. The global **`fallback_locale`** (`APP_FALLBACK_LOCALE`), unless it
   already appeared earlier in the chain - including the common case
   where it's just the current locale itself (the `en`/`en` default).

`Lang::get` / `Lang::get_with` fall through to the key itself if
nothing in the chain resolves it, exactly as [The fallback
chain](#the-fallback-chain) describes; `Lang::try_get` /
`Lang::try_get_with` return `Err`, and `Lang::has` returns `false`. This
walk runs inside the `Lang` facade itself, so it works for **any**
`Translator` - the bundled `FluentTranslator`, or a driver you write.

### A runnable example

```
myapp/
├── lang/
│   ├── pt-BR/
│   │   ├── app.ftl
│   │   └── validation.ftl
│   └── pt-PT/
│       └── app.ftl
├── src/
└── frontend/
```

```ftl
# lang/pt-BR/app.ftl
welcome = Bem-vindo ao { $app }!
file-label = Arquivo
```

```ftl
# lang/pt-PT/app.ftl
file-label = Ficheiro
```

```rust
use suprnova::__;

// A request that resolved to `pt-PT`.
assert_eq!(__!("file-label"), "Ficheiro");                    // pt-PT's own override
assert_eq!(
    __!("welcome", app: "Suprnova"),
    "Bem-vindo ao Suprnova!"                                  // inherited from pt-BR
);
```

`lang/pt-PT/` never defines `welcome` - it doesn't need to. `file-label`
is a genuine one-word difference between the two catalogs, so it's the
only id that gets a file.

### Served catalogs are flattened

The `/_suprnova/lang/pt-PT.ftl` endpoint (see [The catalog
endpoint](#the-catalog-endpoint)) never asks the browser to know that
`pt-BR` exists. `FluentTranslator` pre-merges the whole chain into one
resource per locale at load time - the embedded framework catalog at
the bottom for `en`/`en-*` locales, then the configured parent chain,
then the locale's own files - and serves *that*, already flattened.
Fetch `pt-PT.ftl` and the response carries `welcome` and `file-label`
both, in one request, with no client-side chain logic. `?v=<hash>`
still names one immutable resource; the hash simply now covers strings
pulled in from `pt-BR` too.

**Flattening covers configured parents only** - it never reaches past
them to `fallback_locale`. `pt-PT`'s served catalog includes `pt-BR`'s
strings because `pt-BR` is a *configured parent*; it does not include
`en`'s strings just because `en` happens to be the global fallback.
`LocaleShare`'s `fallback` field always names the terminal
`fallback_locale`, unaffected by any of this - it tells the frontend
where `Lang`'s facade-level walk would eventually land, not what's
already in the file it just fetched.

### Delta-file merge rules

A child catalog merges over its parent **at the Fluent AST level**, not
by textual concatenation and not by whole-message shadowing. The
override unit is the *pattern*, so:

- **A child value replaces the parent's value**, in the parent's
  position in the file.
- **A child entry with attributes but no value keeps the parent's
  value.** Retranslating `.placeholder` doesn't require repeating the
  message's own text.
- **Attributes merge by name.** A same-named child attribute replaces
  the parent's, in place; a child-only attribute appends after the
  parent's own. **Attributes the child doesn't mention survive from the
  parent** - overriding a message's value never silently drops its
  `.placeholder` or `.aria-label`.
- **Select expressions replace whole, never variant-by-variant.** A
  selector's variants are keyed to one locale's CLDR plural categories;
  because those categories are locale-dependent, splicing one variant
  from the parent and another from the child could produce a selector
  with no single locale's grammar behind it. A child that overrides a
  selector at all must supply every variant it wants.
- **Comments on an overridden entry stay the parent's.** The comment
  documents the id, and the override unit is the pattern, not the
  comment.
- **Child-only entries append at the end**, in the child's own order,
  comments included - an id `pt-BR` never defined is not an "override"
  of anything.

Terms (`-brand`) follow the identical rule, with one narrowing: a
term's value is never optional in Fluent syntax, so the
"attributes-but-no-value keeps the parent's value" case above applies
to messages only - a child term always supplies a value, and that
value always wins. Attribute merge-by-name, whole-pattern replacement
for the value, and parent-wins comments all apply to terms exactly as
to messages. Terms are tracked in their own namespace - overriding
`-brand` can never shadow a message also named `brand`.

### Why Suprnova diverges

Laravel 13 has exactly one fallback: the single global `fallback_locale`
config value, consulted when the current locale's array is missing a
key. There is no concept of one locale inheriting from a sibling locale -
`pt_PT.php` and `pt_BR.php` are two unrelated arrays, and a `pt_PT`
app either duplicates everything `pt_BR` already has translated, or
ships without it.

Suprnova's parent chains are the Rust-side extension: an intermediate
step between "this locale" and "the global fallback," configured
per-locale rather than once globally. The tradeoff we didn't want to
make is pushing that complexity onto the browser - a chain-aware
frontend would need to fetch `pt-PT.ftl`, discover it's incomplete,
fetch `pt-BR.ftl` too, and merge them client-side in JavaScript, using
rules that would have to exactly match the server's. Flattening at load
time instead means the served catalog is always one complete,
self-contained file - the same contract the frontend already had before
parent chains existed, so `@fluent/bundle` and the kit wrappers needed
zero changes to support this feature.

## Locale detection

`LocaleMiddleware` resolves one locale per request and binds it for the
duration of the handler. The chain is config-driven and **first hit
wins**:

1. **Session** - the `locale` key in the session, if
   [session middleware](session.md) ran and the value names an available
   locale. This is where "user picked Español in settings" lives.
2. **Cookie** - the `locale` cookie. Survives logout, so a language
   choice made before signing in isn't lost.
3. **`Accept-Language`** - negotiated against `available_locales()` with
   `fluent-langneg`, honouring q-values. `fr-CH, es;q=0.8, en;q=0.5`
   against catalogs `en` + `es` resolves to `es`.
4. **`APP_LOCALE`** - the configured default, when nothing above hit.

A candidate that doesn't parse, or names a locale with no catalog, is
**skipped, not rejected**. A user with a stale `locale=zz` cookie sees
the default language, not a 500. A garbage `Accept-Language` header does
the same. Attacker-controlled input reaches this chain on every request;
it must never be able to do more than pick a language.

Wire it up in `bootstrap.rs`, **after** the session middleware, since
step 1 reads the session:

```rust
use std::sync::Arc;
use suprnova::{
    global_middleware, App, LocaleMiddleware, LocaleShare, SessionConfig, SessionMiddleware,
};

pub async fn register() {
    global_middleware!(SessionMiddleware::install(SessionConfig::from_env()).await);

    // Resolves the locale and binds it for the request.
    global_middleware!(LocaleMiddleware::from_env().expect("locale config"));

    // Hands the frontend its locale + catalog URL on every Inertia page.
    App::register_inertia_shared(Arc::new(LocaleShare));
}
```

`LocaleMiddleware::from_env()` reads `LocalizationConfig::from_env()`;
`LocaleMiddleware::new(config)` takes one you built yourself. A
scaffolded app has both lines already.

Register it **before** `Inertia::install` as well, if the app names an
[Inertia error page](frontend-inertia-responses.md#error-pages). That
page is rendered by a middleware on the way out, after everything
registered inside it has returned - so a locale scope opened inside the
Inertia layer is already gone by then, and every error page would render
in the default locale. Session outside locale outside Inertia is the
order the scaffold uses.

### Changing the locale mid-request

`Lang::set_locale` is Laravel's `App::setLocale` - it rewrites the
current request's locale from that point on:

```rust
use suprnova::session::session_mut;
use suprnova::{FrameworkError, Lang, Locale};

/// The user just switched languages in a settings form.
pub fn switch_language(choice: &str) -> Result<(), FrameworkError> {
    let locale = Locale::parse(choice)?;
    Lang::set_locale(locale);                       // this request
    session_mut(|s| s.put("locale", choice));       // every request after
    Ok(())
}
```

Note the two halves: `set_locale` affects *this* request (so the
redirect's flash message is already in Spanish), and the session write
is what the detection chain reads on the *next* one.

### Outside a request

Console commands, queue workers, and scheduled tasks have no request and
no middleware. There, `Lang::set_locale` writes a process-global
override that `Lang::locale()` consults before falling back to
`APP_LOCALE`:

```rust
use suprnova::{command, FrameworkError, Lang, Locale, Mail};

use crate::mail::Digest;
use crate::models::user::User;

#[command(name = "mail:digest", description = "Send the weekly digest")]
pub async fn send_digest(_args: Vec<String>) -> Result<(), FrameworkError> {
    for user in User::query().get().await? {
        // Each user's stored preference, for the duration of their email.
        Lang::set_locale(Locale::parse(&user.locale)?);
        Mail::to(&user.email).send(Digest::for_user(&user)).await?;
    }
    Ok(())
}
```

Because that override is process-wide rather than task-local, set it at
the top of each unit of work as above - don't rely on it being unchanged
across an `.await` that another task could interleave with.

## Configuration

Three environment variables. `APP_LOCALE` and `APP_FALLBACK_LOCALE` both
default to `en`; `APP_LOCALE_PARENTS` defaults to empty - no per-locale
overrides, only `fallback_locale` applies:

```env
APP_LOCALE=en
APP_FALLBACK_LOCALE=en
# APP_LOCALE_PARENTS=pt-PT=pt-BR
```

Everything else is code, on `LocalizationConfig`. It registers like every
other typed config - in your `config::register_all`, which runs before
boot:

```rust
// src/config/mod.rs
use suprnova::{Config, Detect, Locale, LocalizationConfig};

pub fn register_all() {
    let localization = LocalizationConfig::from_env()
        .expect("APP_LOCALE / APP_FALLBACK_LOCALE must be valid BCP-47")
        .default_locale(Locale::parse("es").expect("valid locale"))
        .use_isolating(true)                                // see the divergence note
        .detection(vec![Detect::Session, Detect::Header])   // ignore the cookie
        .session_key("preferred_locale")
        .cookie_name("lang")
        .parent(                                            // see Fallback chains
            Locale::parse("pt-PT").expect("valid locale"),
            Locale::parse("pt-BR").expect("valid locale"),
        );

    Config::register(localization);
}
```

- `default_locale` / `fallback_locale` - override `APP_LOCALE` and
  `APP_FALLBACK_LOCALE` from code. A malformed value in either place
  fails boot rather than silently becoming `en`.
- `use_isolating` - Unicode isolation marks around interpolations. Off
  by default; turn it on when you ship an RTL locale.
- `detection` - the chain, in order. Dropping `Detect::Cookie` means a
  language choice only lives in the session; dropping `Detect::Header`
  means the browser's preference is ignored entirely.
- `session_key` / `cookie_name` - rename the two lookups.
- `parents` - per-locale fallback parents (`child -> parent`), walked
  before `fallback_locale` when a key is missing from the child's
  catalog; same shape as `APP_LOCALE_PARENTS`. Add one with
  `.parent(child, parent)` - chainable, last write wins for a repeated
  child. See [Fallback chains](#fallback-chains) for the full contract
  (boot-time validation, resolution order, served-catalog flattening).

Boot binds an `Arc<dyn Translator>` in the container. If your app has
already bound one, the framework leaves it alone - which is how you
substitute a translator of your own without forking anything:

```rust
// src/bootstrap.rs
use std::sync::Arc;
use suprnova::{App, FluentTranslator, LocalizationConfig, Translator};

pub async fn register() {
    let config = LocalizationConfig::from_env().expect("locale config");
    let translator =
        FluentTranslator::from_dir("./catalogs", &config).expect("load catalogs");
    App::bind::<dyn Translator>(Arc::new(translator));
}
```

`Translator` is the extension seam: `translate`, `has`,
`available_locales`, `catalog`, `reload`. One driver ships
(`FluentTranslator`), and a new backend is a new driver - not a fork of
the surface.

## Translated validation messages

Every built-in rule returns a **keyed** message: a catalog key, the
arguments the message needs, and an English fallback. Translation
happens once, at the serialization boundary - `ValidationErrors::to_json`
and the Inertia error bag - never inside the rule. Rules stay pure, and
the whole subsystem compiles out.

The keys follow one convention:

| Shape | Example | Used for |
|---|---|---|
| `validation-<rule>` | `validation-min`, `validation-required-if` | One per built-in rule, kebab-cased |
| `field-<name>` | `field-email` | A human name for a field |
| `validation-invalid-data` | - | The top-level "The given data was invalid." banner |

To translate them, define the ids you care about in any `.ftl` file
under the target locale:

```ftl
# lang/es/validation.ftl
validation-invalid-data = Los datos proporcionados no son válidos.
validation-required = El campo { $field } es obligatorio.
validation-email = El campo { $field } debe ser una dirección de correo válida.
validation-min = El campo { $field } debe tener al menos { $min } caracteres.
validation-confirmed = La confirmación del campo { $field } no coincide.
```

`$field` is always available. Every rule's own parameters are passed
under the names they carry in the framework's English catalog -
`$min`, `$max`, `$other`, `$value` - and
`framework/src/localization/catalogs/en/validation.ftl` is the canonical
list of ids and arguments. Copy the ids you need out of it; you never
have to override all of them.

Overriding works per locale and per key. Defining `validation-min` in
`lang/en/validation.ftl` replaces the framework's English wording for
that one rule and leaves the rest alone.

### Field names

Interpolating a raw column name produces "The email_address field is
required." The `field-<name>` convention fixes that:

```ftl
# lang/en/validation.ftl
field-email_address = email address
field-dob = date of birth
```

Before rendering, the translator looks up `field-<name>` for the current
locale. A hit is passed as `$field`; a miss falls back to the field name
with underscores turned into spaces. So the file above is only needed
for the names that humanize badly.

### Custom rules

`Rule::passes` returns `Result<(), ValidationMessage>`. A keyed message
participates in translation:

```rust
use suprnova::{Rule, ValidationMessage};

pub struct StartsWith(pub &'static str);

impl Rule for StartsWith {
    fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
        if value.starts_with(self.0) {
            Ok(())
        } else {
            Err(ValidationMessage::keyed("validation-starts-with")
                .arg("prefix", self.0)
                .fallback(format!("must start with {}", self.0)))
        }
    }
}
```

```ftl
# lang/en/validation.ftl
validation-starts-with = The { $field } field must start with { $prefix }.
```

A plain string still works, and is the right answer for a message that
will only ever exist in one language:

```rust
Err("must start with acct_".into())   // keyless: rendered verbatim
```

Keyless messages skip translation entirely, which is what keeps existing
custom rules compiling and behaving exactly as before.

### The derive flow

`#[derive(Validate)]` errors are keyed too. The `validator` crate's
error code becomes `validation-<code>` with underscores turned into
dashes, and every param the validator attaches becomes a message
argument - with two reserved exceptions, `value` and `other`, which are
always dropped. Both carry a field's actual *value* rather than
metadata about the rule: `value` is the echoed input under test, and
`other` (set by `must_match`, the canonical password-confirmation rule)
is the sibling field's value. Neither is ever handed to the catalog, so
no `.ftl` override - however it phrases `validation-must-match` - can
interpolate a submitted secret into a 422 response body. So a
`#[validate(email)]` failure resolves `validation-email` like the
hand-written rule does, and a locale that translates one translates
both.

## The frontend

The browser gets the same bytes the server resolved. Nothing is
re-translated, re-exported, or kept in sync by hand.

### The catalog endpoint

```
GET /_suprnova/lang/es.ftl              → 200 text/plain, ETag: "<hash>"
GET /_suprnova/lang/es.ftl?v=<hash>     → 200 + Cache-Control: public,
                                          max-age=31536000, immutable
GET /_suprnova/lang/es.ftl              → 304 when If-None-Match matches
GET /_suprnova/lang/zz.ftl              → 404 (no such catalog)
```

The body is the merged catalog for that locale - framework messages
first, then its configured fallback parent chain if any (see [Fallback
chains](#fallback-chains)), then your files in load order. `ETag` is the content hash. Ask
for a specific hash with `?v=` and the response is immutable-cacheable
forever, because that URL can only ever mean one thing; ask without it
and you get revalidation instead. Like `/_suprnova/health`, the path is
exempt from the middleware chain: it must answer before a locale has
been resolved, and it carries no user data.

### The shared prop

`LocaleShare` is an `InertiaSharedData` the framework ships. Registered
in `bootstrap.rs` (see [Locale detection](#locale-detection)), it adds
one prop to every Inertia page:

```json
{
  "lang": {
    "locale": "es",
    "fallback": "en",
    "catalog": {
      "url": "/_suprnova/lang/es.ftl?v=9f2c1ae4",
      "hash": "9f2c1ae4"
    }
  }
}
```

`catalog` is `null` when no translator is bound - the share never fails
a page render.

### The kit wrappers

Each starter kit ships a ~100-line wrapper that reads that prop, fetches
the catalog once, builds a `@fluent/bundle` bundle, and exposes `t()`.
Call `initLang` once in your Inertia entry point (scaffolded apps
already do):

```ts
// frontend/src/main.ts
import { createInertiaApp } from '@inertiajs/svelte'
import { mount } from 'svelte'
import { initLang } from './lib/lang.svelte'

createInertiaApp({
  resolve: (name) => { /* … unchanged … */ },
  async setup({ el, App, props }) {
    await initLang(props.initialPage)
    mount(App, { target: el!, props })
  },
})
```

Then, in components:

```svelte
<!-- Svelte 5 -->
<script lang="ts">
  import { t, currentLocale } from '../lib/lang.svelte'
</script>

<h1>{t('welcome', { app: 'Suprnova' })}</h1>
<p>{currentLocale()}</p>
```

```tsx
// React 19
import { useLang } from '../lib/lang'

export default function Home() {
  const { t, locale } = useLang()
  return <h1>{t('welcome', { app: 'Suprnova' })}</h1>
}
```

```vue
<!-- Vue 3.5 -->
<script setup lang="ts">
import { useLang } from '../lib/lang'
const { t, locale } = useLang()
</script>

<template>
  <h1>{{ t('welcome', { app: 'Suprnova' }) }}</h1>
</template>
```

Number and date formatting on the client uses the browser's built-in
`Intl` - no ICU data is shipped to the browser.

### Typed message keys

`suprnova generate-types` parses `lang/<default locale>/*.ftl` and emits
a union of every message id alongside the page-props types:

```ts
// frontend/src/types/lang-keys.ts
// Generated by `suprnova generate-types` - do not edit.
export type MessageKey =
  | "validation-min"
  | "welcome"
```

The wrappers type `t(key: MessageKey, …)`, so this is the same promise
as [`inertia-props.ts`](frontend-typescript-types.md): rename a message
in Rust, regenerate, and the TypeScript compiler points at every call
site that still uses the old id. `suprnova serve` watches `lang/`
alongside `src/`, so the file regenerates as you edit catalogs.

A project with no `lang/` directory and no message ids gets **no
file** - an app that isn't localized sees no new artifact appear.

## Locale-aware formatting

Seven functions on `Lang`, all ICU4X-backed, all reading the current
locale, all with `try_*` siblings that return
`Result<String, FrameworkError>` instead of degrading:

```rust
use suprnova::chrono::NaiveDate;
use suprnova::{DateStyle, Lang, ListStyle, RelativeUnit, TimeStyle};

let dt = NaiveDate::from_ymd_opt(2026, 8, 1)
    .and_then(|d| d.and_hms_opt(14, 30, 0))
    .expect("valid datetime");

Lang::number(1_234_567.89);                          // en-US → 1,234,567.89
                                                     // de-DE → 1.234.567,89
Lang::currency(19.99, "USD");                        // en-US → $19.99
Lang::date(&dt, DateStyle::Long);                    // en-US → August 1, 2026
Lang::time(&dt, TimeStyle::Short);                   // en-US → 2:30 PM
Lang::datetime(&dt, DateStyle::Medium, TimeStyle::Short);
Lang::list(&["Ada", "Grace", "Alan"], ListStyle::And); // → Ada, Grace, and Alan
Lang::relative(-3, RelativeUnit::Day);               // → 3 days ago
```

The style enums: `DateStyle { Full, Long, Medium, Short }`,
`TimeStyle { Medium, Short }`, `ListStyle { And, Or, Unit }`,
`RelativeUnit { Second, Minute, Hour, Day, Week, Month, Year }`.
`Lang::relative` takes a signed amount - negative is the past
("3 days ago"), positive the future ("in 3 days").

> Exact output comes from the CLDR data baked into ICU4X and can change
> across an ICU upgrade, particularly for dates and currency. In your own
> tests, assert on shape and locale-distinctness (`de != en`, contains
> `2026`) rather than on exact bytes.

### Formatting inside a message

Two functions are callable from FTL:

```ftl
order-total = Your total is { NUMBER($amount, maximumFractionDigits: 2) }.
published = Published { DATETIME($when, dateStyle: "medium", timeStyle: "short") }
```

```rust
use suprnova::__;

let line = __!("published", when: "2026-08-01T14:30:00");
```

`NUMBER()` is Fluent's builtin, registered explicitly, and gives you
fraction-digit control inside the message. `DATETIME()` is Suprnova's:
`$value` accepts an ISO-8601 string or epoch milliseconds, and
`dateStyle` / `timeStyle` take the same names as the Rust enums, lower
case. A value it cannot parse passes through verbatim with a `warn!` -
a Fluent function cannot return an error, and a rendered page with one
odd-looking date beats a 500.

When you want ICU4X's full formatting rather than what a Fluent function
exposes, format in Rust and pass the finished string in:

```rust
use suprnova::{__, Lang};

let total = __!("order-total-text", amount: Lang::currency(19.99, "USD"));
```

## Testing your translations

Two helpers do the work: `use_lang_path` points the loader at a fixture
directory, and `scope_locale` pins the current locale for the duration
of a future.

The hermetic form - build a translator over a fixture directory and bind
it in a test-scoped container - is what the framework's own tests use,
because it touches no process-global state and survives parallel test
execution:

```rust
use std::sync::Arc;
use suprnova::testing::TestContainer;
use suprnova::{scope_locale, FluentTranslator, Lang, Locale, LocalizationConfig, Translator};

#[tokio::test]
async fn spanish_greeting_comes_from_the_catalog() {
    let _guard = TestContainer::fake();

    let config = LocalizationConfig::from_env().expect("locale config");
    let translator = FluentTranslator::from_dir("tests/fixtures/lang", &config)
        .expect("load catalogs");
    TestContainer::bind::<dyn Translator>(Arc::new(translator));

    scope_locale(Locale::parse("es").expect("locale"), async {
        assert_eq!(Lang::get("welcome"), "¡Bienvenido!");
        assert_eq!(Lang::locale().as_str(), "es");
    })
    .await;
}
```

`use_lang_path` is the right tool when the test boots the real
application and you want the *whole* app pointed at fixtures:

```rust
use suprnova::use_lang_path;

#[tokio::test]
async fn app_boots_against_fixture_catalogs() {
    use_lang_path("tests/fixtures/lang");
    // … boot the app; `lang_path("")` now resolves to the fixture dir.
}
```

It writes a process-global path override, so treat it as a per-binary
setting rather than something two parallel tests can disagree about.

Detection itself - the session/cookie/`Accept-Language` chain - is worth
testing through the real pipeline rather than by calling the middleware
directly, because the interesting cases are about header parsing and
about which source wins. Mount a route whose handler returns
`__!("welcome")`, register `LocaleMiddleware` in the
`MiddlewareRegistry`, and drive it with the loopback harness from
[HTTP Tests](http-tests.md), sending `Accept-Language: fr, es;q=0.8` and
asserting on the Spanish body. The cases worth pinning: a header
negotiates, a cookie beats a header, an unavailable locale is skipped
rather than erroring, and a malformed header still returns 200.

See [Testing](testing.md) for `TestContainer::scope` when your test runs
on a multi-threaded runtime - the thread-local `fake()` guard above does
not survive a future migrating between workers.

### Why Suprnova diverges

**FTL files, not PHP arrays.** Laravel has two formats - nested arrays
in `lang/en/messages.php`, plus flat JSON in `lang/en.json` for
string-keyed translations - and neither is loadable by a browser, nor
expresses plural selection in the file: that lives in `trans_choice`'s
pipe-and-range convention inside the string. Fluent gives us one format that the server and
the client both parse, which is what makes "the frontend shows the same
string the validator produced" a property of the design rather than a
convention you maintain. It costs you a new syntax to learn (this
chapter is most of it) and a tooling change: Poedit can't edit `.ftl`,
while Crowdin, Weblate, Lokalise, and Pontoon can. It also costs
dotted namespacing - `trans('messages.welcome')` has no equivalent,
because ids are a flat namespace per locale. Prefix instead.

**No `trans_choice`.** Laravel selects a plural form with pipe-separated
strings and explicit ranges:

```php
// Laravel
trans_choice('{1} plik|[2,4] pliki|[5,*] plików', $count);
```

Now count to 22 in Polish. CLDR puts 22 in the `few` category - `22
pliki` - but `[5,*]` swallows it and produces `22 plików`. The same
break happens at 32, 42, 102, and in Russian, Arabic, Czech, Lithuanian,
and Welsh, each in its own places. Integer ranges cannot express plural
rules, because plural rules are not about ranges; they're about the last
digit, the last two digits, and in some languages whether the value is
an integer at all. Fluent selects on the CLDR category directly, so
`$count` is an ordinary argument and the *translator* - the person who
knows the language - writes all four of Polish's categories:

```ftl
files =
    { $count ->
        [one] { $count } plik
        [few] { $count } pliki
        [many] { $count } plików
       *[other] { $count } pliku
    }
```

`one` is 1; `few` is 2-4, 22-24, 32-34, 102-104; `many` is 0, 5-21,
25-31; `other` catches the fractions (`1,5 pliku`) and carries the
default marker, per the rule above.

Laravel's rangeless form (`plik|pliki|plików`) does better - it consults
a per-language index and picks the *n*th segment - but that index is a
hand-maintained table rather than CLDR data, it offers Polish three
segments where CLDR defines four categories, the segments are positional
with no category names to review, and it can only ever select on the
count.

Which is the second benefit, falling out for free: a Fluent selector can
switch on *any* argument, not just a count. Gender, plan tier, and connection
state select the same way, and none of them needed a new facade method.

**Isolation marks are off by default.** Fluent normally wraps every
interpolation in U+2068 (FIRST STRONG ISOLATE) and U+2069 (POP
DIRECTIONAL ISOLATE), so that a right-to-left value embedded in a
left-to-right sentence renders in the right order. Correct - and
invisible, which means every `assert_eq!("Hello Ada", …)` in an
English-only app fails with two characters nobody can see in the diff.
We default them off and make turning them on one call:

```rust
let config = LocalizationConfig::from_env()?.use_isolating(true);
```

**Turn them on when you ship an RTL locale** - Arabic, Hebrew, Persian,
Urdu - or any locale where user-supplied values mix scripts inside a
sentence. Then update your assertions to compare against strings that
carry the marks, or strip them in the assertion helper. The default
optimises for the common case; the correct case is one line away and
this paragraph is the reminder to take it.

## Next

- [Validation](validation.md) - rules, the `validate!` macro, and where
  `ValidationMessage` comes from
- [TypeScript Types](frontend-typescript-types.md) - `generate-types`,
  `inertia-props.ts`, and `lang-keys.ts`
- [Middleware](middleware.md) - ordering `LocaleMiddleware` against the
  rest of the global chain
- [Session](session.md) - the store the first detection step reads
- [Environment Variables](env-vars.md) - `APP_LOCALE`,
  `APP_FALLBACK_LOCALE`, `APP_LOCALE_PARENTS`, `APP_BASE_PATH`
- [Testing](testing.md) - `TestContainer`, `#[suprnova_test]`, and
  hermetic DI overrides
