# Precognition

Precognition lets you validate a draft with your server's rules before
you submit it. You use the same route for live validation and the real
request. You keep validation in [form requests](requests.md) and show
its errors in your frontend.

## Opt in to Precognition

Add `Precognitive` to a route or a route group. A route without this
middleware ignores the `Precognition` header and handles a real request.
This example validates an email and redirects to the form on the real
submit. Your kit's bootstrap supplies the Inertia middleware:

```rust
use suprnova::{
    InertiaResponse, Precognitive, Redirect, Request, Response,
    get, handler, post, request, routes,
};

#[request]
pub struct CreateUser {
    #[validate(email(message = "Enter a valid email address."))]
    pub email: String,
}

#[handler]
pub async fn create(req: Request) -> Response {
    Ok(InertiaResponse::new("Users/Create").resolve(&req).await?)
}

#[handler]
pub async fn store(_form: CreateUser) -> Response {
    Ok(Redirect::to("/users/create"))
}

routes! {
    get!("/users/create", create),
    post!("/users", store).middleware(Precognitive),
}
```

Send `Precognition: true` to ask for live validation. The middleware marks
the request before the later middleware and extractors run. Route
parameters and model bindings resolve, and form requests validate. The
handler body never runs, whether it is a closure or a controller method.

Every response from a route carrying `Precognitive` includes
`Precognition` in `Vary`. An existing `Vary` value is kept. Every response
to a marked request also carries `Precognition: true`, including a `401`,
`403`, `404`, `409` or `423` from later middleware or an extractor.
A passing live validation answers `204` with `Precognition-Success: true`.
A failing validation answers `422` with `{ message, errors }`.

Try the route with a running application:

```sh
curl -i http://127.0.0.1:8765/users \
  -H 'Content-Type: application/json' \
  -H 'Precognition: true' \
  -H 'Precognition-Validate-Only: email' \
  --data '{"email":"ada@example.com"}'
```

You receive `204` and the two Precognition headers. Send an invalid email
to see the `422` error bag. Omit `Precognition` to run `store`.

## Live validation with Inertia 3.8

Inertia 3.8 ships the Precognition client. If you use a Suprnova starter
kit, you install nothing extra. Import `useForm` from your Inertia adapter,
call `.withPrecognition('post', '/users')`, and call
`form.validate('email')` when the input loses focus. Submit normally with
`form.post('/users')`. Put the component in
`frontend/src/pages/Users/Create.vue`, `Create.tsx` or `Create.svelte`
for your adapter. Visit `/users/create`. Each example below posts to the
route above.

### Vue

```vue
<script setup lang="ts">
import { useForm } from '@inertiajs/vue3'

const form = useForm({ email: '' }).withPrecognition('post', '/users')
</script>

<template>
  <form @submit.prevent="form.post('/users')">
    <label for="email">Email</label>
    <input
      id="email"
      type="email"
      v-model="form.email"
      @blur="form.validate('email')"
    />
    <p v-if="form.errors.email">{{ form.errors.email }}</p>
    <p v-if="form.validating">Validating...</p>
    <button type="submit" :disabled="form.processing">Submit</button>
  </form>
</template>
```

### React

```tsx
import { useForm } from '@inertiajs/react'

export default function CreateUser() {
  const form = useForm({ email: '' }).withPrecognition('post', '/users')

  return (
    <form onSubmit={(event) => {
      event.preventDefault()
      form.post('/users')
    }}>
      <label htmlFor="email">Email</label>
      <input
        id="email"
        type="email"
        value={form.data.email}
        onChange={(event) => form.setData('email', event.target.value)}
        onBlur={() => form.validate('email')}
      />
      {form.errors.email && <p>{form.errors.email}</p>}
      {form.validating && <p>Validating...</p>}
      <button type="submit" disabled={form.processing}>Submit</button>
    </form>
  )
}
```

### Svelte

```svelte
<script lang="ts">
  import { useForm } from '@inertiajs/svelte'

  const form = useForm({ email: '' }).withPrecognition('post', '/users')
</script>

<form onsubmit={(event) => {
  event.preventDefault()
  form.post('/users')
}}>
  <label for="email">Email</label>
  <input
    id="email"
    type="email"
    bind:value={form.email}
    onblur={() => form.validate('email')}
  />
  {#if form.errors.email}<p>{form.errors.email}</p>{/if}
  {#if form.validating}<p>Validating...</p>{/if}
  <button type="submit" disabled={form.processing}>Submit</button>
</form>
```

See [Inertia responses](frontend-inertia-responses.md) for pages and
redirects after the real submit.

## Client configuration

The client sends `Precognition: true`, `Accept: application/json` and
`Precognition-Validate-Only` with the field names it wants to validate,
joined with commas. The list can be empty when no field is touched.
The client debounces validation with a default timeout of 1500 ms.
Set your own timeout on any of the three forms:

```js
form.setValidationTimeout(500)
```

The client checks `Precognition: true` on success and error responses.
Without it, the client throws `Did not receive a Precognition response`.
Check that your target route carries `Precognitive`. This check happens
before status callbacks such as forbidden or not-found handling.

## Validating arrays and wildcards

`Precognition-Validate-Only` names fields exactly. A `*` matches one
non-empty path segment. `tags.*` covers `tags.3`, but `tags` does not.
`users.*.email` covers `users.2.email`, but not `users.2.profile.email`.
Use the input's name, including any serde rename:

```js
form.validate('users.*.email')
```

On a marked request, only the listed fields' rules run. The list narrows
the rules before validation, through the derived rules, the synchronous
hook and the asynchronous hook. An unlisted field that fails or does not
parse cannot prevent a listed field's database check from running.
Without the header, you validate all fields. An empty header validates
no fields and answers `204` when no hook adds an error.

Errors an after-validation hook adds are never filtered. Any such error
answers `422`, even if its field is outside the list. What the list
narrows is the checks a hook runs, as Laravel removes the rules of
unlisted fields before its validator runs.

## Database rules in hooks

While the after-validation hooks of a form request, a data object or a
multipart form run for a marked request that lists fields,
`Precognition::should_validate(field)` answers whether the field is
listed, by the same exact and wildcard match. Outside such a request it
answers `true`, so a real submit runs every check.

The built-in database rules ask it before they query. `check_async` (on
`Unique`, `Exists` and `Password`), `Exists::check_value` and
`Exists::check_each` skip a field the request did not list: no query
runs and no error is added. `check_each` is selected under the key
`<field>.*`, the key Laravel gives an array rule, so `tag_ids.*` in the
list runs it and `tag_ids` alone does not:

```rust
use suprnova::{AsyncRule, Exists, FormRequest, Precognition, Unique, ValidationErrors, async_trait};

#[derive(serde::Deserialize, validator::Validate, suprnova::FormRequestDerive)]
#[form_request(custom_hooks)]
pub struct Signup {
    #[validate(email)]
    pub email: String,
    pub tag_ids: Vec<i64>,
}

#[async_trait]
impl FormRequest for Signup {
    async fn after_validation_async(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        // Skipped when the request lists fields and `email` is not one.
        Unique::new("users", "email")
            .check_async(&self.email, &mut errors, "email")
            .await;
        // Runs for `tag_ids.*`, not for `tag_ids`.
        Exists::new("tags", "id")
            .check_each(&self.tag_ids, &mut errors, "tag_ids")
            .await;
        // Your own per-field work asks the same question.
        if Precognition::should_validate("email") && self.email.ends_with("@example.invalid") {
            errors.add("email", "Use a real address.");
        }
        errors.into_result()
    }
}
```

A field is matched both as you name it in Rust and under the input name
serde renames it to. `Request::should_validate(field)` keeps its meaning
for middleware and custom rules that hold the request. See
[Validation](validation.md) for rule objects and cross-field hooks.

## Validation outside a form request

`Precognition::after_validation(&request, errors)` turns an error bag
you built yourself into the Precognition answer, as Laravel's
`Precognition::afterValidationHook` does for any validator. On a marked
request an empty bag answers `204` with `Precognition-Success: true` and
a non-empty one `422` with the bag. On another request an empty bag
returns `Ok(())` and the request goes on, and a non-empty one is the
ordinary validation error: a redirect back with the errors for an HTML
form, `422` otherwise.

```rust
use suprnova::{Middleware, Next, Precognition, Request, Response, ValidationErrors, async_trait};

pub struct RequireTeam;

#[async_trait]
impl Middleware for RequireTeam {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let mut errors = ValidationErrors::new();
        if request.should_validate("team") && request.header("X-Team").is_none() {
            errors.add("team", "Choose a team.");
        }
        Precognition::after_validation(&request, errors)?;
        next(request).await
    }
}
```

Register it after `Precognitive`, so the request is marked. The bag is
used as you give it: check a field only when `request.should_validate`
answers `true` for it.

## Customizing rules for live validation

Use `Request::is_precognitive()` to ask whether the middleware marked
the request. Use `Request::is_attempting_precognition()` to ask whether
the header alone requests Precognition. On a route without `Precognitive`,
the second can be true while the first is false. Use the marked state
when you decide whether to skip a rule or a side effect.

`Request::validate_only()` exposes the parsed list on a marked request.
Outside a marked request, the selection is absent and every field is included.
`Request::should_validate(field)` applies the same exact and wildcard
match as the validator. Use it in middleware or a custom rule before
you run a field's own check. A handler can use it on the real request;
its body never runs on a marked one.

The client leaves files out of live validation by default. Declare a file
as optional in your form request, then require it on the real submit.
For example, this custom check requires an avatar only on a real request:

```rust
use suprnova::{ImageFile, Request, UploadedFile, ValidationErrors};

fn require_avatar(
    req: &Request,
    avatar: Option<&UploadedFile<ImageFile>>,
) -> Result<(), ValidationErrors> {
    let mut errors = ValidationErrors::new();
    if !req.is_precognitive() && avatar.is_none() {
        errors.add("avatar", "Choose an avatar.");
    }
    errors.into_result()
}
```

Pass the request and the parsed optional file to your custom check where
you validate uploads. Keep the file's image and size checks for any file
the client actually sends. A missing file during live validation does
not make the real submit's required-file rule optional.

## File uploads and query data

Call `form.validateFiles()` to include files in live validation. A
precognitive multipart request validates both its files and its fields
through the same form-request path as a JSON body. Your upload validators
still check any listed file. See [Requests](requests.md) for
`Option<UploadedFile<ImageFile>>`, file lists and size limits.

A precognitive `GET` or `DELETE` validates query parameters. A `key[]`
parameter reads as a list. A JSON-encoded object reads as an object.
For example, a marked `GET /users?email=` fails a required email rule;
the absence of a JSON body does not make it pass.

For inline validation, use `Request::validate::<T>()` with your form
request type. On a marked request it applies the same narrowing and
answers `204` or `422` with the Precognition headers. It does not return
the typed value and continue into a write. Put inline validation in
middleware when it must run on a marked request, since the handler body
is skipped.

## Managing side effects

Put writes, queued mail and other submit work in the handler body. The
framework runs extractors and stops before that body on a marked request.
Middleware still runs. After `Precognitive` marks the request, check
`is_precognitive()` in any later middleware before you count an interaction
or perform another side effect. Middleware outside it has not seen the mark.

Use `Precognition::precognitive` around a pre-check closure when you need
the same choice outside dispatch. The helper runs the closure. If the
closure bails with a response, it uses the precognition-specific response
on a marked request and the default response otherwise. If the closure
does not bail, it answers `204` with `Precognition-Success: true` on a
marked request and returns the closure's value on a real one.

A marked request does not save the session. It neither ages nor consumes
flash data and writes no session row. It also does not become the
previous URL, in either the session middleware or Inertia's previous-URL
store. Your next page still receives its toast, and redirecting back
still leads to the page you visited. See [Session](session.md).

## The 422 body

A failing live validation and a failing real JSON request use the same
error bag. `errors` maps each field to a list of messages. `message`
starts with the first error's message. It adds ` (and 1 more error)` for
one remaining error, or ` (and N more errors)` for more than one:

```json
{
  "message": "Enter a valid email address. (and 2 more errors)",
  "errors": {
    "email": ["Enter a valid email address."],
    "name": ["Enter your name."],
    "password": ["Use at least eight characters."]
  }
}
```

You localize the count suffix with `validation-summary-more` in your
request's locale catalog, with an English fallback when the message is missing.

With only one error, `message` has no count. Bind field errors to their
exact keys, including dotted array paths.

## Testing

Use `with_precognition()` on a test request to send `Precognition: true`.
Use `assert_successful_precognition()` on its response to assert both
`204` and `Precognition-Success: true`. This complete test also proves
the handler body does not run:

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use suprnova::testing::TestClient;
use suprnova::{
    MiddlewareRegistry, Precognitive, Response, handler, json_response,
    post, request, routes,
};

#[request]
pub struct EmailDraft {
    #[validate(email)]
    pub email: String,
}

static HANDLER_RUNS: AtomicUsize = AtomicUsize::new(0);

#[handler]
pub async fn store(form: EmailDraft) -> Response {
    HANDLER_RUNS.fetch_add(1, Ordering::SeqCst);
    json_response!({ "email": form.email })
}

routes! {
    post!("/users", store).middleware(Precognitive),
}

#[tokio::test]
async fn validates_without_running_the_handler() {
    let client = TestClient::new(register(), MiddlewareRegistry::new());

    client.post("/users")
        .with_precognition()
        .header("Precognition-Validate-Only", "email")
        .json(&serde_json::json!({ "email": "ada@example.com" }))
        .send().await
        .assert_successful_precognition();

    assert_eq!(HANDLER_RUNS.load(Ordering::SeqCst), 0);
}
```

See [HTTP tests](http-tests.md) for the client and its other assertions.

### Why Suprnova diverges

- **Header values ignore case.** You can send `Precognition: TRUE` or
  `true`. Laravel matches `true` exactly. This keeps the header read
  consistent with Suprnova's existing clients.
- **Field names are trimmed.** You can send
  `Precognition-Validate-Only: email, name`. Suprnova trims each name;
  Laravel splits on commas without trimming. Spaces around a listed
  name do not change the field you asked to validate.
- **Hooks narrow their database rules, not their errors.** Laravel
  removes the rules of unlisted fields from the rule set before its
  validator runs. Suprnova runs database rules from the after-validation
  hooks, so the hooks see the selection through
  `Precognition::should_validate` and the built-in database rules skip
  unlisted fields there. An error a hook adds itself is never filtered.
- **Form requests are typed.** The struct is built only for the real
  request. During live validation, the listed fields parse and validate
  without requiring values for every other field. A parse failure on an
  unlisted field does not block the listed fields' answer. Your real
  submit still needs every required field to parse before you receive
  the typed struct. During live validation, a field whose type parses a
  string itself (a date, an address) answers its own parse error when its
  value does not parse, even when you leave it unlisted.

## Next

- [Validation](validation.md) - rule objects and validation hooks
- [Requests](requests.md) - typed form requests, query data and uploads
- [Inertia responses](frontend-inertia-responses.md) - pages, redirects
  and errors after the real submit
