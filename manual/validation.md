# Validation

Suprnova validates request input on two complementary tracks:

1. **Derive validation** - `#[validate(...)]` attributes on a `FormRequest`
   struct, run automatically by `extract()`. This is the everyday path and
   is covered in [Requests](requests.md). It handles per-field
   rules (`email`, `length`, `range`, …) declaratively.
2. **Rule objects + the `validate!` macro** - plain values implementing
   [`Rule`](#rule-objects) / `ContextualRule` / `AsyncRule`, composed
   imperatively. Reach for these when you need cross-field logic, rules
   that touch the database, or rules you want to store and pass around.

The two tracks accumulate into the same
[`ValidationErrors`](error-model.md) bag and render the same
Laravel/Inertia `{ "message", "errors": { field: [...] } }` shape (HTTP
422).

## Rule objects

A rule is a value implementing one of four traits:

| Trait | Shape | Use |
|-------|-------|-----|
| `Rule` | `passes(&self, value: &str)` | pure check on one value |
| `ValueRule` | `passes(&self, value: &serde_json::Value)` | check on a JSON-shaped value (array/object) |
| `ContextualRule` | `passes(&self, value, ctx)` | check that reads sibling fields |
| `AsyncRule` | `async passes(&self, value)` | check that `.await`s (DB, HTTP) |

Built-in `Rule`s: `Required`, `Email`, `Min`, `Max`, `Between`, `In`,
`NotIn`, `InArray`, `Integer`, `Numeric`, `Boolean`, `Alpha`, `AlphaNum`,
`AlphaDash`, `Url`, `UrlProtocols`, `HttpUrl`, `Uuid`. Built-in
`ValueRule`s: `ArrayKeys`, `Distinct`, `Contains`, `DoesntContain`.
Built-in `ContextualRule`s: `RequiredIf`, `RequiredWith`,
`RequiredUnless`, `Same`, `Different`, `Confirmed`, `Gt`, `Gte`, `Lt`,
`Lte`. Built-in `AsyncRule`:
[`Unique`](#the-unique-rule).

```rust
use suprnova::{Rule, rules::Email};

Email.passes("user@example.com")?; // Ok(())
```

> **Note:** `Numeric` accepts a **finite** number - `NaN`, `inf`, and magnitudes that
> overflow to infinity are rejected, even though Rust's parser would accept
> the strings.

### URL schemes

`Url` accepts a value that parses as a URL, whose scheme is on Laravel's
allowlist - the same list `Illuminate\Support\Str::isUrl` uses - is
followed by `://`, **and** is followed in turn by a non-empty host,
matching Laravel's `^(PROTOCOLS)://HOST` pattern in shape (Laravel's
host group has no `?` - an absent or empty host never matches). The
scheme list and the `://`-plus-host requirement are Laravel's verbatim;
the host is parsed by the `url` crate rather than Laravel's regex, so an
out-of-range port is rejected here that Laravel would accept. All three
have to hold: `mailto:`, `tel:`, and `data:` are on the allowlist by name
but carry no authority component at all, so `Url` rejects them; and
`file:///etc/passwd` fails for the third reason - it has `://`, but
nothing sits between the third and fourth `/`, and nothing isn't a host.
`javascript:` and `vbscript:` are rejected outright; they aren't on the
allowlist at all.

`ftp://host/x` and `ssh://host` - real hosts, just not web schemes - still
pass, so `Url` is not a "this is a web page" check, and it says nothing
about where the URL resolves. Rejecting `javascript:` makes a validated
value safe to put in an `href`, not safe to fetch. A webhook or callback
target still needs `HttpUrl` (or your own scheme + SSRF checks); `Url`
alone doesn't cover that.

For a narrower set, name the schemes you want:

```rust
use suprnova::{Rule, rules::Url};

// Laravel's `url:http,https`
Url::protocols(&["https"]).passes("https://example.com")?;   // Ok
Url::protocols(&["https"]).passes("http://example.com");     // Err

// The same thing, under a name
use suprnova::rules::HttpUrl;
HttpUrl.passes("https://example.com")?;
```

`Url::protocols(...)` **replaces** the allowlist rather than narrowing it,
so an app can accept its own deep-link scheme (`myapp://…`) without the
framework having an opinion about it - the `://`-plus-host requirement
still applies to that custom scheme too. Use `HttpUrl` (or
`Url::protocols(&["https"])`) for callback, webhook, and avatar inputs -
a webhook target that resolves to `ftp://internal-host/` still parses as
a `Url`, and an `ftp:` target is not a webhook target.

### Writing your own rule

A custom rule is a unit (or data-carrying) struct with one impl. The
trait gives you `check()` for free - it pushes any failure message onto
a `ValidationErrors` bag under the named field - so the rule plugs
into `validate!` and the `after_validation` hooks unchanged:

```rust
use suprnova::{Rule, ValidationMessage};

pub struct StartsWith(pub &'static str);

impl Rule for StartsWith {
    fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
        if value.starts_with(self.0) {
            Ok(())
        } else {
            Err(format!("must start with {}", self.0).into())
        }
    }
}

// Now usable everywhere:
StartsWith("acct_").passes("acct_1234")?;
// or, in a validate! row:
//   stripe_id => Required, StartsWith("acct_");
```

A `String` converts into a `ValidationMessage` that renders verbatim,
which is all a single-language app needs. To have the message translated
per locale, return a *keyed* message instead -
`ValidationMessage::keyed("validation-starts-with").arg("prefix", self.0).fallback(…)` -
and define the id in `lang/<locale>/validation.ftl`. See
[Localization](localization.md), which also covers overriding the
built-in rules' messages and the `field-<name>` naming convention.

For cross-field logic, implement [`ContextualRule`] instead - the
`passes` method gets a `&FormContext` (a `HashMap<String, String>` of
sibling field values) alongside the value under test. For
database-backed checks, implement [`AsyncRule`] and use it from
`after_validation_async`.

### Value-shaped rules

`Rule` only ever sees `&str`. Two built-ins need more structure than a
string carries, so they implement `ValueRule` instead, over
`&serde_json::Value`:

```rust
use suprnova::{ValueRule, rules::{ArrayKeys, Distinct}};

// Laravel's array:keys - reject keys outside the allowed set. Listed
// keys need not all be present; an empty allowed list is a programming
// error, reported as a keyless message.
ArrayKeys(&["name", "email"]).passes(&serde_json::json!({"name": "Ada"}))?;

// Laravel's distinct / distinct:ignore_case / distinct:strict.
Distinct { ignore_case: false, strict: false }
    .passes(&serde_json::json!(["a", "b", "c"]))?;
```

A field validated by a `ValueRule` must hold `serde_json::Value` itself
(or `Option<serde_json::Value>` for a `?:`/`?=>` row) - typically a
request field pulled straight from the JSON body. `validate!` rows
accept `Rule`s and `ValueRule`s in the same field list; which trait runs
is resolved by which one the rule's type implements, not by anything
you write in the row.

### Membership rules

Three rules answer "is this value in that list?", each over the shape it
needs:

```rust
use suprnova::{Rule, ValueRule, rules::{Contains, DoesntContain, InArray}};

// Laravel's in_array:allowed_roles.* - the value must appear in another
// field's list. Pass the list itself: a Vec<String> field and a &[&str]
// literal both work.
InArray(&form.allowed_roles).passes(&form.role)?;

// Laravel's contains:rust,web - the array must hold every listed value.
Contains(&["rust", "web"]).passes(&form.tags)?;

// Laravel's doesnt_contain:banned - the array must hold none of them.
DoesntContain(&["banned"]).passes(&form.tags)?;
```

Every comparison is exact. `InArray` compares strings with `==`, and
`Contains` and `DoesntContain` match a parameter against a JSON string
element only, so `["1"]` contains `"1"` and `[1]` does not. A value that
is not an array fails `Contains` and `DoesntContain` outright.

`Contains` and `DoesntContain` reject an empty parameter list as a keyless
construction error, the same way `ArrayKeys` does - a list with nothing in
it constrains nothing. An empty `InArray` haystack is different: a sibling
field can legitimately be empty at run time, so the value simply fails.

`InArray`'s failure message names no values, because its list comes out of
the request and a validation message is rendered into a response body.

### Comparison rules

`Gt`, `Gte`, `Lt`, and `Lte` compare a field against a number or against
another field. `CompareWith` names the operand and the measure together:

```rust
use suprnova::{ContextualRule, FormContext, rules::{CompareWith, Gt, Lte}};

let mut ctx = FormContext::new();
ctx.insert("max_price".to_string(), form.max_price.clone());

// Laravel's gt:0 - a literal operand, compared numerically.
Gt(CompareWith::Number(0.0)).passes(&form.price, &ctx)?;

// Laravel's lte:max_price - a sibling field, compared numerically.
Lte(CompareWith::NumericField("max_price")).passes(&form.price, &ctx)?;

// Laravel's gt:summary on two string fields - compared by character count.
Gt(CompareWith::LengthField("summary")).passes(&form.body, &ctx)?;
```

All four read sibling fields, so they are `ContextualRule`s and every
`validate!` row carries `=> with ctx` - including a row whose only operand
is a literal, where the context goes unread. Pass an empty `FormContext`
there.

Anything the rule cannot measure fails the field: a value that is not a
finite number under a numeric comparison, a sibling the form never sent, a
sibling that is not a number, or a non-finite literal such as `f64::NAN`.
None of those panics, and none of them passes.

### Why Suprnova diverges

Laravel's `distinct:strict` leans on PHP's coercing `==`. JSON values are
already typed, so Suprnova's `strict` only changes whether two *numbers*
with different internal representations (`1` vs `1.0`) count as equal -
it never makes a string and a number "the same," in either mode.

Laravel writes the other field into a rule string - `in_array:allowed_roles.*` -
and the validator globs it out of the request data at run time. Suprnova
has no rule-string parser: you hand `InArray` the list directly, and the
compiler checks the field exists.

Laravel 13.27 tightened `in`, `in_array`, and `doesnt_contain` to strict
comparison because PHP's `==` turned `"1abc"`, `true`, and `"0x1"` into
matches. Suprnova never had that hole - `In` and `NotIn` compare `&str`
with `==` - and the new rules match JSON values variant by variant. Laravel's
`contains` stayed loose; Suprnova's does not. The cost is that these rules
cannot check a numeric array: `Contains(&["1"])` does not match `[1]`.

Laravel's `gt` family picks its measure at run time: the number itself for
numerics, `count()` for arrays, kilobytes for files, and character length
for everything else, with the numeric branch gated on whether the field
also carries `numeric` or `integer`. Suprnova writes the measure into the
rule instead, because a rule here cannot see the other rules on its field
and sniffing the value's shape is the coercion habit these rules exist to
avoid. Two of Laravel's four measures have no counterpart at all: a rule
only ever receives a string, so an array-valued sibling cannot be read,
and uploads never reach the rule surface - the multipart parser caps their
size before a handler sees them.

## The `validate!` macro

`validate!` runs a chain of rules over the fields of a struct, accumulating
every failure into one `ValidationErrors`. It's the idiomatic home for the
synchronous cross-field hook, [`after_validation`](#cross-field-hooks).

```rust
use suprnova::{validate, ValidationErrors, rules::{Required, Email, Min, Max, RequiredIf}};

fn after_validation(&self) -> Result<(), ValidationErrors> {
    // Contextual rules read sibling values from a `FormContext` you build
    // - a map of field name to its string value.
    let mut ctx = std::collections::HashMap::new();
    ctx.insert("billing_type".to_string(), self.billing_type.clone());
    validate! { self =>
        email       => Required, Email;          // required-shape row
        bio         ?: Min(10), Max(500);        // optional: validate only if Some
        card_number ?=> RequiredIf {             // conditional-presence (see below)
            other: "billing_type",
            value: "card",
        } => with ctx;
    }
}
```

Each row is one of three shapes:

- **`field => Rule1, Rule2;`** - required-shape. Rules run on `&self.field`
  directly (for `String`, `i64`, or anything that derefs to the rule's
  expected borrow) - or, for a `ValueRule`, on a `serde_json::Value`
  field directly. Which trait each rule uses is inferred automatically.
- **`field ?: Rule1, Rule2;`** - optional. The field is `Option<T>`; rules
  run only when it is `Some`, and are **skipped entirely on `None`**. This
  is Laravel's "if present, validate" (`sometimes`) semantics.
- **`field ?=> Rule1, Rule2;`** - conditional-presence. Also for an
  `Option<String>` field, but rules run **even when `None`** (absence is
  treated as the empty string). This is the row for presence-conditional
  rules like `RequiredIf` that must be able to *fail an absent field* -
  the case `?:` cannot express because it skips on `None`.

A contextual rule is followed by `=> with $ctx` (an
`&HashMap<String, String>` of sibling values). The macro is **synchronous** -
for async rules use the [hook](#async-rules-in-requests) below.

> **Warning:** A common trap: writing `card_number ?: RequiredIf {...} => with ctx;`. On
> a `?:` row, `None` skips all rules, so `RequiredIf` can never fail an
> absent field. Use `?=>` for any rule that must fire on absence.

## Cross-field hooks

`FormRequest` runs two cross-field hooks after the derived per-field rules,
both in the normal and Precognition flows. `extract()` runs the stages in
order - derived `validate()`, then `after_validation`, then
`after_validation_async` - and **bails at the first failing stage**.

```rust
use suprnova::{FormRequest, ValidationErrors};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct UpdatePassword {
    #[validate(length(min = 8))]
    pub new_password: String,
    pub confirmation: String,
}

impl FormRequest for UpdatePassword {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        if self.new_password != self.confirmation {
            errs.add("confirmation", "passwords do not match");
        }
        errs.into_result()
    }
}
```

> **Note:** Override hooks need a hand-written `impl FormRequest` - the `#[request]`
> attribute and `#[derive(FormRequest)]` generate their own (empty) impl, so
> they're for the common no-override case only.

### Async rules in requests

The `validate!` macro can't weave in `.await`, so database-backed rules run
in `after_validation_async` - the final validation stage, which `extract()`
calls automatically. This is where [`Unique`](#the-unique-rule) and any
custom `AsyncRule` participate in automatic request validation; no
per-handler plumbing required.

```rust
use suprnova::{FormRequest, ValidationErrors, Unique, async_trait};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct CreateUser {
    #[validate(email)]
    pub email: String,
}

#[async_trait]
impl FormRequest for CreateUser {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errs = ValidationErrors::new();
        Unique::new("users", "email")
            .check_async(&self.email, &mut errs, "email")
            .await;
        errs.into_result()
    }
}
```

Because the async stage runs only after the synchronous stages pass, a
malformed value (a syntactically invalid email) never reaches the database
`Unique` query.

## The `Unique` rule

`Unique` checks that a value does not already exist in a table. Build it
with `Unique::new(table, column)` and refine with the fluent API:

```rust
use suprnova::Unique;

// email must be unique, ignoring the row currently being edited
Unique::new("users", "email").ignore(current_user_id)

// email unique *per tenant*, compared case-insensitively
Unique::new("users", "email")
    .where_eq("tenant_id", tenant_id)
    .case_insensitive()
```

| Builder method | Effect |
|----------------|--------|
| `.ignore(id)` | exclude the row whose `id` equals `id` (edit-self case) |
| `.ignore_with_column(col, id)` | exclude on a non-`id` key column |
| `.where_eq(col, value)` | scope the check to rows where `col = value`; multiple calls AND together |
| `.case_insensitive()` | compare with `LOWER(col) = LOWER(?)` |

Table, column, the exclusion key, and every `where_eq` column are validated
against an identifier allowlist before they reach the SQL string; the value
under test and all scope values are bound parameters.

### Unique is advisory - the database constraint is the guarantee

`Unique` runs a `SELECT COUNT(*)` **before** the write, so it carries an
unavoidable time-of-check/time-of-use race: two concurrent requests can
both pass the check and then both insert. Laravel's `unique` rule has the
identical property. The **only** real guarantee is a `UNIQUE` constraint
(or unique index) on the column in your migration.

Use the three together:

1. **The advisory rule** - a fast, friendly "that email is taken" message
   before submit (and so Precognition can validate the field).
2. **The `UNIQUE` constraint** - the authoritative guard against the race.
3. **`FrameworkError::from_unique_violation`** - at the write site, map the
   constraint violation the loser of a race receives back to the same clean
   422, instead of leaking a 500:

```rust
use suprnova::FrameworkError;

// `users.email` has a UNIQUE constraint in the migration.
let user = new_user
    .insert(db)
    .await
    .map_err(|e| FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    ))?;
```

`from_unique_violation` returns a 422 `Validation` error when the database
error is a unique-constraint violation, and passes any other error through
unchanged (MySQL, Postgres, and SQLite are all recognized).

## Async authorization

`FormRequest::authorize(&Request) -> bool` runs **before** the body is
parsed, so it can reject unauthorized requests without reading the payload.
It is synchronous by design: at that point the request still holds the
streaming body, so the hook cannot `.await`. Authorization that needs to
hit the database or an async policy belongs in one of these places, not in
`authorize`:

- **Middleware** - runs before `extract()`, is `async`, and short-circuits
  by returning `Err(response)` (see [Middleware](middleware.md)).
  The right place for "is this user allowed to reach this route at all".
- **The Gate** - call `Gate::allows_async` / `Gate::authorize_async` in the
  handler once you have the authenticated user and the resource (see
  [Authorization](authorization.md)).
- **`after_validation_async`** - for an authorization check that depends on
  the parsed request body, run it in the async hook alongside your other
  async rules.

## Inertia form submissions

A validation failure answers two audiences differently. A REST client
gets the `422` with `{ message, errors }`. An Inertia visit gets a `303`
back to the form page with the errors flashed into the session, because
the Inertia client shows an error modal for any response it does not
recognise as an Inertia response - a `422` would never populate
`form.errors`.

Nothing in the handler changes. On the destination page each field
carries its first message as a string:

```svelte
{#if errors?.email}
  <p class="text-red-600">{errors.email}</p>
{/if}
```

See [Inertia responses](frontend-inertia-responses.md#validation-failures)
for error bags, `with_all_errors`, and where the redirect points.

## Design notes

- **Partial validation.** A `FormRequest` deserializes into a typed struct
  before validation runs, so the struct *is* the schema: a field that may
  be absent must be `Option<T>`. This is also what lets Precognition
  validate a partial payload - make the fields a draft can omit optional.
- **Rule messages.** Built-in rules return keyed messages
  (`validation-min` plus its arguments and an English fallback), resolved
  through the catalog at the serialization boundary. Translate or reword
  any of them by defining the same id in `lang/<locale>/validation.ftl` -
  no rule wrapping. See [Localization](localization.md).
- **`Min` / `Max` / `Between`** are string-length rules (counted in Unicode
  scalar values). For numeric bounds, validate with `#[validate(range(...))]`
  on the derive or a custom rule - the length rules are not value
  comparisons.

## Summary

| Task | API |
|------|-----|
| Per-field rules | `#[validate(...)]` on the `FormRequest` (see Requests) |
| Composed / cross-field rules | `validate! { self => ... }` |
| JSON-shaped rule (array/object) | `field => ArrayKeys(&[...]);` / `field => Distinct { .. };` |
| Optional "if present" | `field ?: Rule;` |
| Conditionally-required optional | `field ?=> Rule => with ctx;` |
| Async / DB-backed rule | `after_validation_async` + `AsyncRule::check_async` |
| Uniqueness | `Unique::new(t, c)` + `UNIQUE` constraint + `from_unique_violation` |
| Async authorization | middleware / `Gate::*_async` / `after_validation_async` |

## Next

- [Requests](requests.md) - the `#[request]` / `#[derive(FormRequest)]`
  surface, the everyday derived-validation path
- [Data Objects](data.md) - `#[derive(Data, Validate)]` for one struct
  that's both an inbound request and an outbound DTO
- [Error Model](error-model.md) - how `ValidationErrors` becomes the
  422 JSON body, alongside every other error path
- [Localization](localization.md) - translating rule messages, the
  `field-<name>` convention, and keyed `ValidationMessage`s
- [Authorization](authorization.md) - `Gate`, `Policy`, and where
  authorization belongs relative to validation
- [Middleware](middleware.md) - the right place for "is this request
  even allowed through" checks that need `.await`
