# Upgrading to 4.0

This chapter moves an application from Suprnova 3.2.1 to 4.0.0. Each
section matches one entry of the 4.0.0 section of the
[changelog](../CHANGELOG.md), which carries the full detail of every change.

## Who this chapter is for

- A person upgrading an application from 3.2.1 to 4.0.0.
- An agent doing the same upgrade. The headings are exact strings, so a
  search for a changelog entry's bold lead sentence lands on its section.

## How to use this chapter

1. Find the bold lead sentence of a changelog entry, such as
   **Typed factory counts.**
2. Search this chapter for it. It heads a `###` section, word for word.
3. Read the three parts of the section and make the edit:
   - **Before:** what your application saw on 3.2.1.
   - **Now:** what it sees on 4.0.0.
   - **Do:** the edit that keeps it working, or "Nothing to do." when
     none applies.

Sections are grouped by area, in the order of the
[documentation index](documentation.md). Every `### Changed` entry has a
section; the 4.0.0 changelog has no `### Removed` heading, and its removals
are `### Changed` entries. After the areas,
[Fixed entries that change behaviour](#fixed-entries-that-change-behaviour)
covers the fixes that change what an application observes,
[Added entries that change behaviour](#added-entries-that-change-behaviour)
covers additions that change an existing path, and
[Public types with new fields or variants](#public-types-with-new-fields-or-variants)
covers types that gained fields without an entry of their own.

## Upgrade in this order

1. Bump the pin. Point every `suprnova` dependency in `Cargo.toml` at the
   release, in `[dependencies]` and `[dev-dependencies]` alike, then update
   the lock file and the CLI:

   ```toml
   suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.0.0" }
   ```

   ```bash
   cargo update -p suprnova
   cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v4.0.0 suprnova-cli
   ```

2. Build. Run `cargo check --all-targets`. The compiler names each type or
   method whose shape changed; search this chapter for that name.
3. Follow the compile errors until the build is clean. Each one has a
   **Do:** line with the edit.
4. Then work through the behaviour changes for the areas your application
   uses. Start with
   [Configuration and environment](#configuration-and-environment),
   [Cache](#cache) and [Migrations and schema](#migrations-and-schema):
   an unset `APP_ENV`, the Redis cache prefix, unsigned keys and the MySQL
   collation change how a deployment boots and what it finds in its stores.
5. Run `suprnova migrate`, then your test suite.

## Configuration and environment

### An unset `APP_ENV` is production.

**Before:** An unset `APP_ENV` was `local`: `.env.local` loaded and none of
the production checks ran.

**Now:** An unset `APP_ENV` is production. Debug is off, `APP_KEY` is
required, the production refusals apply, and `db:seed`, `migrate:fresh` and
`suprnova migrate:fresh` ask for `--force`. `.env.local` still loads; no
other `.env.<environment>` file does.

**Do:** Set `APP_ENV` in every environment, test runs and CI included. For
development, use `local`, as the scaffold's `.env` does:

```env
APP_ENV=local
```

## The service container

### `#[service(impl = ...)]` parses.

**Before:** `#[service(impl = ...)]` failed to parse: `impl` was refused as
a keyword.

**Now:** The named form registers the implementation.

**Do:** Nothing to do.

## Routing and controllers

### A wrong method answers 405.

**Before:** A path that only a route of another method matched answered
`404`, an `OPTIONS` request included.

**Now:** It answers `405` with `Allow` and Laravel's message, and an
`OPTIONS` request answers `200` with `Allow`. Both pass through the global
middleware. A path no route matches keeps the `404` or the fallback.

**Do:** Nothing to do. A client or test that expects `404` for a wrong
method expects `405`.

### `RouteBinding` is the one binding trait

**Before:** A type implemented `AutoRouteBinding` by hand, and an older
`RouteBinding` trait with `param_name` existed beside it.

**Now:** `RouteBinding` is the one trait, shaped like Laravel's
`UrlRoutable`. `AutoRouteBinding` is a second name for it, and
`<T as AutoRouteBinding>::from_route_param(value)` keeps working. The old
trait and the unused `route_binding!` macro are gone.

**Do:** Rewrite a hand-written `impl AutoRouteBinding` as
`impl RouteBinding` with its methods, as [Routing](routing.md) shows, and
remove any call of the removed macro. Every `#[suprnova::model]` implements
it already.

### A `#[handler]` inside an `impl` block names its type

**Before:** A `#[handler]` method inside an `impl` block compiled without
naming its type.

**Now:** It must name the type. Without it, the build fails with an error
that names the fix.

**Do:** Add the type to each such attribute, as
[Controllers](controllers.md) shows:

```rust
impl Posts {
    #[handler(Self = Posts)] // was #[handler]
    pub async fn show(post: Post) -> Response {
        json_response!({ "id": post.id })
    }
}
```

### A dotted resource name nests.

**Before:** `Router::resource("users.posts", ...)` registered
`/users.posts/{users.post}`.

**Now:** It registers `/users/{user}/posts/{post}`, as Laravel's
`Route::resource` does.

**Do:** Move clients to the nested paths, and read the `post` parameter
where handlers read `users.post`.

### A route key that does not parse answers 404.

**Before:** A route key that did not parse, or broke a `unique_id` key's
format, answered `400` and repeated the value.

**Now:** It gets the `404` body a missing row gets, naming the model and
never the value.

**Do:** Nothing to do. A client or test that expects the `400` expects
`404`.

### The bare `x::Model` form no longer binds a soft-deleted row

**Before:** The bare `x::Model` form bound a soft-deleted row of a
soft-deleting `#[model]`'s table.

**Now:** It no longer binds such a row unless the route calls
`with_trashed()`.

**Do:** Call `with_trashed()` on each route that must bind trashed rows.

### Bound arguments are bound before every other argument

**Before:** Arguments resolved in declaration order, so a form request
declared before a bound model validated first.

**Now:** Bound arguments resolve first, after the route's middleware, so a
missing row answers `404` before a form request is validated.

**Do:** Nothing to do. A test that expects a `422` for a request whose
bound row is missing expects `404`.

### The router refuses a route it cannot run, at startup.

**Before:** These mistakes surfaced at request time, and two body readers
were a compile error.

**Now:** A handler that reads an undeclared parameter, reads the body twice,
names a binding field that is not a parseable column, scopes a child through
an undeclared relation, or uses a binder that cannot run is an error from
the boot path. Two body readers are a startup error, except two `Request`
arguments, still a compile error.

**Do:** Boot the application once after the upgrade and fix each route the
error names.

## Middleware

### A default middleware priority.

**Before:** Middleware ran in the order a route or the global list gave it.

**Now:** The priority list starts as `Precognitive`, `SessionMiddleware`,
`AuthMiddleware` and `ThrottleRequestsMiddleware`, so a route listing
`AuthMiddleware` before `SessionMiddleware` loads the session first, and a
route-listed `Precognitive` runs before a global `SessionMiddleware`.

**Do:** Nothing to do. To change the order, edit the priority list, as
[Middleware](middleware.md) describes.

## CSRF protection

### CSRF checks every method but GET, HEAD and OPTIONS.

**Before:** `CsrfMiddleware` let `QUERY` and extension methods such as
`PROPFIND` through without a token.

**Now:** It checks them as it checks `POST`; a `QUERY` request without a
token gets `419`.

**Do:** Send the CSRF token with `QUERY` and extension-method requests.

### Regenerating the session issues a new CSRF token.

**Before:** `regenerate_session_id()` kept the CSRF token.

**Now:** It sets a new CSRF token beside the new id, so a token read before
the regeneration gets `419` after it.

**Do:** Read the token again after a sign-in or any other regeneration, as a
page rendered after it does.

### The CSRF field turns autocomplete off.

**Before:** `csrf_field()` rendered no `autocomplete` attribute.

**Now:** It renders `autocomplete="off"`, so a browser does not restore a
stale token into it.

**Do:** Nothing to do.

## Requests

### `Request::ajax()` compares exactly.

**Before:** `Request::ajax()` was true for other spellings of
`XMLHttpRequest` too.

**Now:** It is true only for `X-Requested-With: XMLHttpRequest`.

**Do:** Nothing to do. A client that sends another spelling sends
`XMLHttpRequest`.

### `Request::host()` reads the host as Symfony does.

**Before:** `Request::host()` returned the host as sent.

**Now:** It lowercases the host, strips only a numeric port, and returns
`None` for a host with a character no host has, so `http_host()`,
`scheme_and_http_host()` and `url()` never carry such a host.

**Do:** Nothing to do. Code that compares hosts compares them in lower
case.

## Responses and redirects

### File and disk responses answer byte ranges.

**Before:** File and disk responses ignored `Range` and sent the whole file.

**Now:** A `GET` range gets `206` with the selected bytes and
`Content-Range`, and an unsatisfiable range gets `416`. Responses carry
`Accept-Ranges: bytes` and `Last-Modified`.

**Do:** Nothing to do.

### A streamed file is one blocking task, not one per chunk.

**Before:** A file above 1 MiB took one blocking-pool task per 64 KiB chunk.

**Now:** One blocking task reads the whole file in 256 KiB chunks. The bytes
and `Content-Length` are unchanged.

**Do:** Nothing to do.

### `Redirect::intended` ignores a stored network-path reference.

**Before:** A stored intended URL that starts with `//`, `/\` or `\`, or
holds a control character, was followed.

**Now:** It is ignored in favour of the fallback.

**Do:** Store an external destination as an absolute URL.

### `Redirect::guest` stores a page to come back to.

**Before:** `Redirect::guest` stored the request's own URL for every
request, so a sign-in after a `POST` or a JSON call went back to it.

**Now:** It stores the request's URL only for a `GET` that does not expect
JSON, and the session's previous URL for any other request. It never reads
the `Referer`.

**Do:** Nothing to do.

### Deletion cookies follow the session configuration.

**Before:** The deletion cookie did not take `Path`, `Domain` and `SameSite`
from the session configuration, so it could miss the cookie it deleted.

**Now:** `Cookie::forget`, `without_cookie` and `without_cookies` take them
from `SESSION_PATH`, `SESSION_DOMAIN` and `SESSION_SAME_SITE`. An explicit
path or domain given to `Cookie::forget_with` still wins.

**Do:** Nothing to do. For a cookie set with another path or domain, use
`Cookie::forget_with`.

## URL generation

### Some string-pair `route()` calls need `.as_str()`.

**Before:** `route()` and `try_route()` took a slice of string pairs, so the
compiler coerced many pair forms.

**Now:** They take bound values as well as strings, and a few forms no
longer compile: inferred names or values (`s.as_ref()`,
`Default::default()`), more than one dereference (`&&&str`,
`&Box<String>`), arrays mixing `&String` and `&str`, arrays of more than 32
`&String` pairs, and `route` used as a function pointer.

**Do:** Write `.as_str()`, `&*` or `.as_slice()`, or a closure in place of
the pointer:

```rust
route("posts.show", &[("post", slug.as_str())])
```

### URLs carry the public root.

**Before:** `route()`, `try_route`, `route_with_params` and
`try_route_with_params` returned the path without the public root.

**Now:** They put the public root (the path in `APP_URL`, or a trusted
prefix) in front. Signed URLs cover the root, `LiveAssetCatalog::url`
includes it, and `RenderKeyInput` has a public `root` field.

**Do:** Register routes without the `APP_URL` path, so the root appears once,
as [URL Generation](urls.md) describes. Sign again any link signed before the
application gained a root. A `RenderKeyInput` struct literal names `root`.

### `url::current` is the absolute URL without the query.

**Before:** `url::current` returned the path and the query, such as
`/posts?page=2`.

**Now:** It returns the absolute URL without the query: the `APP_URL`
origin, the public root and the path.

**Do:** Use `url::full` where you need the query, and compare
`url::current` with an absolute URL.

## Session and cookies

### Cookies follow the root.

**Before:** `SessionConfig.cookie_path` and the XSRF cookie path defaulted
to `/`.

**Now:** Both default to unset, which means the public root of each request.
When a request carries two cookies of one name, the framework reads the
first.

**Do:** Nothing to do. To keep `/` under a path prefix, set it:

```env
SESSION_PATH=/
```

## Validation

### Merged validation hook errors.

**Before:** Form and multipart requests did not return the errors of both
after-validation hooks together with the field rules' errors.

**Now:** Both hooks run after the field rules, and the response carries
their combined errors.

**Do:** Nothing to do.

### Classic form validation redirects.

**Before:** A failed form that prefers HTML got the validation error
response.

**Now:** It redirects back, with the errors and the old text input flashed
to the session.

**Do:** Render the session's errors and old input in your HTML forms.

### An image wildcard leaves SVG out.

**Before:** `image/*` in a `MimeType` allowlist admitted an SVG document.

**Now:** A `type/*` wildcard admits every subtype except `image/svg+xml`.

**Do:** To accept SVG, name `image/svg+xml` in the allowlist.

### The numeric max message.

**Before:** `#[validate(range(max = 10))]` failed with another message.

**Now:** It reads `must not be greater than 10`, Laravel's wording.

**Do:** Update tests that assert the old message.

### `ImageFile` accepts Laravel's image types.

**Before:** `ImageFile` also accepted other image types, such as TIFF, PSD,
ICO and JPEG XL.

**Now:** It accepts JPEG, PNG, GIF, BMP, WebP, AVIF, HEIC and HEIF, and
refuses every other type.

**Do:** Accept any other type through a `MimeType` allowlist that names it.

## Precognition

### Precognition requires the route middleware.

**Before:** Every form request answered the `Precognition` header.

**Now:** Only a route that carries `Precognitive` does. Elsewhere the header
is ignored and the request runs for real.

**Do:** Add `Precognitive` to each route or group that serves live
validation, as [Precognition](precognition.md) shows:

```rust
post!("/users", store).middleware(Precognitive),
```

### Database rules in hooks follow the Precognition selection.

**Before:** `AsyncRule::check_async`, `Exists::check_value` and
`Exists::check_each` ran for every field of a precognitive request.

**Now:** They skip a field the request did not list, so they neither query
nor fail. Errors a hook adds stay unfiltered.

**Do:** Nothing to do.

## Error handling

### Production browser failures show a minimal HTML view.

**Before:** With debug off, a browser got the same 5xx body as any other
client.

**Now:** A browser's 5xx page shows its status and no error details.
Inertia visits keep the application error page, and JSON clients keep their
JSON response.

**Do:** Nothing to do.

### Debug frames show source and separate error traces.

**Before:** Frames showed no source lines, and recorded source errors shared
one trace.

**Now:** Application frames include nearby source lines when the file is
readable, and each recorded source error has its own trace.

**Do:** Nothing to do.

### Errors are reported through `Exceptions`.

**Before:** A 5xx response, a failed queued job attempt and a console
command's error reached the `framework error` log line without your
reportable callbacks.

**Now:** They run your reportable callbacks first, and a console command's
error is logged as well as printed.

**Do:** Check that your `Exceptions` callbacks expect these reports.

### The frame that created an error shows where.

**Before:** In a build without debug info, the innermost application frame
had no location.

**Now:** It gets the error's creation site as its location.

**Do:** Nothing to do.

## Logging

### Daily logs keep seven files by default.

**Before:** The `daily` channel kept 14 files by default.

**Now:** The `daily` channel and `LogChannel::daily` keep 7 files unless
`LOG_DAILY_DAYS` or `.days(...)` sets another count; 0 keeps every file.

**Do:** To keep 14 files:

```env
LOG_DAILY_DAYS=14
```

### Syslog names the application.

**Before:** Syslog datagrams did not use `APP_NAME` as their ident.

**Now:** They use `APP_NAME`, with `suprnova` when it is unset, and keep each
record's facility and severity.

**Do:** Update syslog filters that match the old ident.

### File and stream log channels keep `LOG_LEVEL`'s bare level.

**Before:** The file and stream channels did not keep to `LOG_LEVEL`'s bare
level.

**Now:** `single`, `daily`, `monthly`, `stderr`, `errorlog` and `syslog` keep
only records at or above it, `debug` when it names none, and a bare level
that is no level fails the boot.

**Do:** Make sure `LOG_LEVEL` names a real level, or leave it unset.

## Console

### The console binary ends with the command's exit code.

**Before:** A new project's `src/bin/console.rs` ended with
`Err(_) => ExitCode::FAILURE`, so every failed command exited with 1.

**Now:** The scaffold ends with the code of `FrameworkError::exit`.

**Do:** In `src/bin/console.rs`, replace the error arm:

```rust
match result {
    Ok(()) => ExitCode::SUCCESS,
    Err(error) => ExitCode::from(error.exit_code()), // was Err(_) => ExitCode::FAILURE
}
```

### `down` prints Laravel's lines.

**Before:** `down` printed its own wording.

**Now:** It prints Laravel's lines: `Maintenance mode options updated.` when
already down, `You may bypass maintenance mode via [<APP_URL>/<secret>].`,
and `Failed to enter maintenance mode: <error>.` on a failure.

**Do:** Update scripts that read the output.

## Cache

### `CacheConfig` gained `connection` and `lock_connection`.

**Before:** `CacheConfig` had no `connection` or `lock_connection` field.

**Now:** It has both.

**Do:** A struct literal names both fields, or spreads the default:

```rust
let config = CacheConfig {
    prefix: "app_cache:".to_string(),
    ..CacheConfig::default()
};
```

### The Redis cache moves to the `cache` connection and `CACHE_PREFIX`.

**Before:** The Redis cache used the default connection, and `REDIS_PREFIX`
(`suprnova_cache:` by default) was its key prefix.

**Now:** It runs on the `cache` connection (`REDIS_CACHE_DB`, 1 by default)
under `REDIS_PREFIX`, then `CACHE_PREFIX` (the slug of `APP_NAME` and
`-cache-`), then the key. `Cache::flush` deletes only keys under both
prefixes. An upgraded deployment finds its Redis cache empty.

**Do:** Let the cache refill, or keep the old keys. If you had set
`REDIS_PREFIX`, put that value in `CACHE_PREFIX` instead:

```env
REDIS_CACHE_CONNECTION=default
REDIS_PREFIX=
CACHE_PREFIX=suprnova_cache:
```

## Events

### Unique queued listeners take the lock.

**Before:** A `QueuedListener` whose job declares `unique_id` pushed a job
for every event, while the job's lock was held too.

**Now:** It pushes through `Queue::push_unique`, so a second event under the
lock pushes nothing and dispatches `UniqueJobSkipped`.

**Do:** Nothing to do.

## File storage

### `Storage::purge` takes the disk name.

**Before:** `Storage::purge()` dropped every disk.

**Now:** `purge(name)` drops one disk and reports whether it was registered;
`purge_all()` drops every disk.

**Do:**

```rust
Storage::purge_all(); // was Storage::purge()
```

### The implicit default disk is local.

**Before:** With no default disk named in code or `FILESYSTEM_DISK`,
`Storage::default_disk()` returned an error.

**Now:** It returns `local`, or an error naming `local` and
`FILESYSTEM_DISK` when no `local` disk is registered.

**Do:** Nothing to do.

### S3 accepts Laravel's AWS variables.

**Before:** S3 disks read their own variable names only.

**Now:** When an S3 name is unset, the bucket, region, credentials, public
URL, endpoint and path-style settings read Laravel's AWS names. Each explicit
S3 value wins.

**Do:** Nothing to do. Check that a stray `AWS_*` variable does not now
configure a disk you left to its defaults.

### `make_directory` takes a path without its trailing slash.

**Before:** `make_directory("a/b")` failed on every disk.

**Now:** It creates the directory as `"a/b/"` does. An empty path and a path
that ends in `/` reach the backend unchanged.

**Do:** Nothing to do.

## HTTP client

### The HTTP fake records a request as it is sent.

**Before:** The fake recorded a request before its body's `Content-Type`,
the user agent, the global middleware's headers, the base URL, URL
parameters and query were applied.

**Now:** It records the request as it is sent.

**Do:** Update assertions on recorded headers or URLs that expected the
request before those steps.

## Images

### The built-in image driver's PNG, BMP and WebP codecs come from the eas4ai forks

**Before:** The built-in driver used the upstream `oxideav-png`,
`oxideav-bmp` and `oxideav-webp` crates.

**Now:** It uses the eas4ai forks. Output changes in three cases: pixels
decoded from a lossy WebP (by up to 16 levels), a PNG with more than 1 MiB
of filtered data (its bytes can differ), and lossless WebP with
transparency (written in the simple lossless layout).

**Do:** Nothing to do. Refresh test fixtures that compare encoded bytes or
decoded lossy WebP pixels exactly.

### Images are upright and carry no metadata.

**Before:** Images kept their orientation tag and metadata, and rotation
corners and flattened transparency were black in JPEG and GIF.

**Now:** Images are oriented by their EXIF tag on decode, and processed
output keeps only the ICC profile. Those corners are white, and GIF output
is converted to sRGB.

**Do:** To keep the pixels as the sensor wrote them:

```env
IMAGE_AUTO_ORIENT=false
```

### The image types changed shape.

**Before:** `Transformation` and `ImageConfig` were exhaustive,
`Transformation::Rotate(f32)` took a bare angle, and
`Image::store(disk, path)` stored at a path you gave.

**Now:** Both are `#[non_exhaustive]`, `Rotate` is
`Rotate { degrees, background }`, `store(directory, disk)` stores under a
generated name and returns the path, and `store_as(directory, name, disk)`
takes a name.

**Do:** Build `ImageConfig` from `default()` or `from_env()`, give a match on
`Transformation` a wildcard arm, and store with a directory and a name:

```rust
// was: image.store("public", "avatars/1.jpg").await?
let path = image.store_as("avatars", "1.jpg", Some("public")).await?;
```

### No image driver receives a zero side.

**Before:** A side of 0 reached the driver, and the `magick` driver failed
on the `0x0` geometry.

**Now:** `Image::resize`, `cover`, `resize_width` and `resize_height` record
a side of 0 as 1.

**Do:** Nothing to do.

### `ImagePipeline` carries the settings of its custom steps.

**Before:** `ImagePipeline` had no `settings` field.

**Now:** `settings` holds what `Image::transform_with` recorded, and a
custom driver runs a custom step with
`CustomTransformation::apply_with(pixels, &pipeline.settings)`.

**Do:** A struct literal names `settings` or ends with the default:

```rust
let pipeline = ImagePipeline {
    transformations,
    ..ImagePipeline::default()
};
```

## Localization

### `lang/vendor` is not a locale.

**Before:** The translator read `lang/vendor` as a locale directory.

**Now:** It skips it; `lang/vendor` holds overrides of namespaced catalogs.

**Do:** Nothing to do. Rename a locale you named `vendor`.

## Mail and notifications

### One attachment per name and bytes.

**Before:** A message sent every attachment, duplicates included.

**Now:** It drops an attachment whose name and bytes equal an earlier one's;
two attachments with one name and different bytes are both sent.

**Do:** Nothing to do.

### Notification mail without a subject.

**Before:** An empty `MailRendering` subject was sent empty.

**Now:** It is the notification's name in title case (`InvoicePaid` gives
`Invoice Paid`).

**Do:** Set a subject where you want another one.

## Processes

### Processes remove inherited environment variables.

**Before:** `env` could only set an inherited variable.

**Now:** Passing `None` to `env` removes the variable before the program
starts.

**Do:** Nothing to do.

### TTY processes require terminal input and output.

**Before:** A TTY process started whatever its input and output were.

**Now:** It requires terminal input and output, fails with an error naming
the redirected stream, and captures no output.

**Do:** Run TTY processes from a terminal only, and read output from a
process run without TTY.

### Process diagnostics quote command arguments.

**Before:** Results and errors showed the arguments joined by spaces.

**Now:** They use POSIX shell quoting. Fake patterns still match the
arguments joined by spaces.

**Do:** Update assertions on the diagnostic text.

### Stopping a process accepts a signal.

**Before:** `stop(grace)` took the grace period alone.

**Now:** `stop(grace, signal)` also takes a signal. `None` sends terminate
and waits 10 seconds before killing the process and its descendants.

**Do:**

```rust
let result = process.stop(Duration::from_secs(5), None).await?; // was stop(Duration::from_secs(5))
```

### Process fake handlers accept closures.

**Before:** A fake handler was a fixed result.

**Now:** It can be a closure that reads the pending command, path,
environment and input before it returns a result.

**Do:** Nothing to do.

### Described process fakes replay mixed streams in order.

**Before:** A described fake did not replay its output and error-output
lines in the order described.

**Now:** It replays mixed streams in order, and each line ends with one
newline.

**Do:** Update assertions on a described fake's exact output.

### Process run-count assertions default to once.

**Before:** `assert_ran_times(command, times)` took a count.

**Now:** `assert_ran_times(command)` checks one run, and
`assert_ran_count(command, times)` takes a count.

**Do:**

```rust
fake.assert_ran_count("git push", 3); // was fake.assert_ran_times("git push", 3)
```

### A started fake process reveals its output a line at a time.

**Before:** A started fake process answered its whole output on the first
read.

**Now:** Each `output()` or `latest_output()` call reveals the next line:
`output()` answers every line so far and `latest_output()` the new one. The
error output works the same way, and `wait()` still returns the whole
output.

**Do:** Read a started fake's whole output from `wait()`.

## Strings and numbers

### Excerpts trim invisible characters.

**Before:** Cut ends kept invisible characters such as zero-width spaces.

**Now:** They remove Laravel's invisible character set, and characters
inside the excerpt stay.

**Do:** Nothing to do.

### Percentages round decimal ties half up.

**Before:** `0.12345` at four fraction digits formatted as `0.1234%`.

**Now:** Decimal ties round half up, so it formats as `0.1235%`.

**Do:** Nothing to do.

### Special number spellings.

**Before:** Number formats had no spelling for values without a finite
decimal representation.

**Now:** They use `∞` and `NaN`.

**Do:** Nothing to do.

## Queues

### Queue counts and clearing accept a queue name.

**Before:** `Queue::size()`, `pending_size()`, `delayed_size()`,
`reserved_size()` and `clear()` took no argument.

**Now:** Each takes an `Option<&str>` naming a queue, on every driver. SQS
uses `SQS_QUEUE` when you pass `None`.

**Do:**

```rust
let waiting = Queue::size(None).await?; // was Queue::size()
Queue::clear(Some("emails")).await?;
```

### SQS accepts FIFO queues.

**Before:** SQS sends carried no message group or deduplication ID.

**Now:** Jobs carry a message group and a deduplication ID, `default` and a
payload digest by default, and sends omit delays, so FIFO queues work.

**Do:** Nothing to do.

### SQS overflow supports named cache stores.

**Before:** SQS overflow payloads went to a disk.

**Now:** `SQS_OVERFLOW_STORE` names a cache store and takes precedence over
the disk, and clearing a queue keeps other queues' payloads.

**Do:** Nothing to do.

### Workers check `failed_jobs` before they start.

**Before:** `run_worker` returned nothing, and a worker started on a
failed-jobs table it could not write.

**Now:** `run_worker` returns `Result<(), FrameworkError>`. A worker whose
failed-jobs store is the database refuses to start on a table it cannot
write, and `queue:work` exits non-zero. Each failed job gets a fresh `uuid`,
and its `connection` is `suprnova:<connection>`, so Laravel's `queue:retry`
refuses it.

**Do:** Run `suprnova migrate` first, and handle the result:

```rust
run_worker(driver, config, shutdown).await?;
```

### The after-commit decision follows an order.

**Before:** A push waited for the commit when the job's `after_commit` or
`QUEUE_AFTER_COMMIT` said so.

**Now:** The first of the push's own override, the job's
`after_commit_choice`, the connection's setting and `QUEUE_AFTER_COMMIT`
decides, for `push`, `push_unique` and `bulk` alike.

**Do:** Nothing to do. To push a job before the commit while its connection
waits, return `Some(false)` from `after_commit_choice`.

## Rate limiting

### The in-memory rate limiter keeps answering while it sweeps.

**Before:** Each sweep of the in-memory limiter held one lock over every
bucket and stalled every throttled request.

**Now:** The buckets sit in a sharded map, and a sweep locks one shard at a
time. Answers are unchanged, and no request fails with "rate limiter
poisoned".

**Do:** Nothing to do.

### Rate-limiter keys are cleaned as Laravel cleans them.

**Before:** `RateLimiter::clean_rate_limiter_key` did not apply PHP's
`htmlentities`.

**Now:** It applies `htmlentities` before it strips entity markers, so
`café` and `cafe` count in one bucket and `a&b` is stored as `aab`.

**Do:** Nothing to do. Counts under such keys start again.

### Named limiters rename shared keys.

**Before:** Limits that shared a key were counted under re-keyed
`suprnova:throttle:<hex>:...` keys.

**Now:** `RateLimiter::limiter` gives each limit that shares a key its
fallback key, and `throttle:<name>` counts each limit under `<name>:<key>`,
so the middleware and a direct caller read the same buckets.

**Do:** Nothing to do. The old keys are not read, so these buckets start at
zero after the upgrade.

### One rate-limit header pair.

**Before:** The throttle middleware wrote its headers without regard to the
headers a handler had set.

**Now:** It writes one `X-RateLimit-Limit` and `X-RateLimit-Remaining` pair,
for the limit with the fewest attempts left, and none when the handler's
response carries an equal or lower `X-RateLimit-Remaining`.

**Do:** Nothing to do.

### Refused requests are not counted.

**Before:** A refused request still counted in the buckets.

**Now:** A refused request leaves every bucket's count where it was.

**Do:** Nothing to do.

### `throttle:60,1` counts signed-in users by user.

**Before:** The inline throttle counted every request by address.

**Now:** It counts a signed-in user by the user's identifier and a guest by
address, each per path.

**Do:** Nothing to do. Counts start again after the upgrade.

## Redis

### Redis prefixes typed keys.

**Before:** Typed methods, scripts, blocking commands and typed pipeline
methods used keys as given.

**Now:** They apply `REDIS_PREFIX`, by default the slug of `APP_NAME`
followed by `-database-`. Raw commands keep their arguments.

**Do:** To read keys written by 3.2.1, keep them unprefixed:

```env
REDIS_PREFIX=
```

### Redis events reach application listeners.

**Before:** Enabled Redis command and failure events reached the Redis
listeners only.

**Now:** They also dispatch through `EventFacade`. Pipeline and transaction
commands stay outside these events.

**Do:** Nothing to do.

## Task scheduling

### Single-server tasks need a cache store.

**Before:** An `on_one_server()` task ran on every server when no cache
store was bound.

**Now:** A schedule with such a task and no cache store refuses to start, in
every environment, and a tick with no store skips the task with an error
log.

**Do:** Bind a shared cache store in your bootstrap before the scheduler
starts.

### Day helpers keep the schedule.

**Before:** A day helper after a frequency, time or `cron` call reset the
schedule, so `.hourly().mondays()` ran at midnight on Mondays.

**Now:** `days`, `weekdays`, `weekends` and `sundays` to `saturdays` replace
only the day-of-week field, so `.hourly().mondays()` is `0 * * * 1`. Alone,
they still run at midnight.

**Do:** Check chains that put a day helper after a frequency; they keep the
frequency.

### Hour-only and seconds times.

**Before:** `at`, `try_at`, `daily_at` and `try_daily_at` did not read `9`
or `09:30:00`.

**Now:** They read `9` as 09:00 and `09:30:00` as 09:30.

**Do:** Nothing to do.

## WebSockets

### A WebSocket upgrade no longer waits behind the shutdown drain.

**Before:** An upgrade in flight during shutdown waited out the drain, up to
5 s, before its handler was registered.

**Now:** It no longer waits. The drain still aborts what runs after 5 s.

**Do:** Nothing to do.

## Live

### `live:add --manifest` installs only from a signed library tree.

**Before:** `live:add --manifest` installed a manifest without a signature,
a manifest held a `version`, and `.suprnova-installed.json` keyed its
digests in an earlier form.

**Now:** A third-party manifest sits in a library tree beside the
`library.json` that names its namespace and key, and installs only with a
valid signature. A manifest holds no `version`. `.suprnova-installed.json`
keys digests by the path from the project root, and `live:add` rewrites an
earlier record.

**Do:** Install third-party components from a signed library tree, as
[Live Component Libraries](live-libraries.md) describes, and remove
`version` from your own manifests.

### Scaffolds ignore `live:add`'s lock and journal.

**Before:** A new project's `.gitignore` did not list `live:add`'s lock and
journal.

**Now:** It lists `/.suprnova-live.lock` and `/.suprnova-live-journal.json`.

**Do:** Add both lines to your `.gitignore`:

```bash
printf '%s\n' '/.suprnova-live.lock' '/.suprnova-live-journal.json' >> .gitignore
```

### A component's script can no longer replace a built-in.

**Before:** The `live:add` scan admitted a write to a built-in's member,
such as `Object.keys = f` or `Math.random = f`.

**Now:** A write to a member of a built-in object or function is refused
(`script-builtin`) in every form.

**Do:** If `live:add` refuses a component it admitted on 3.2.1, change the
component as the refusal says.

## Authentication

### Suprnova never writes `users.remember_token`.

**Before:** The scaffold's `User` carried `update_remember_token`, read `id`
as `i64` and its timestamps as `DateTime<Utc>`.

**Now:** Suprnova never writes `users.remember_token`. The scaffold's `User`
drops `update_remember_token`, reads `id` as `u64` and its timestamps as
`Option<DateTime<Utc>>`.

**Do:** Nothing to do. Your `User` keeps compiling. Keep `id` as `i64` while
your `users.id` column is signed.

### The verified gate answers each request.

**Before:** `EnsureEmailVerifiedMiddleware::redirect_to` redirected every
unverified request, JSON clients included.

**Now:** A request that expects JSON gets `403`
`Your email address is not verified.`, and any other request goes through
`Redirect::guest`, so `Redirect::intended` returns the user after
verification. `new()` still answers `403` to every caller.

**Do:** Redirect with `Redirect::intended` after verification to return the
user.

### Verification refusals tell the two cases apart.

**Before:** `EmailVerification::verify` answered `400` for every refused
link.

**Now:** It answers `403` `This action is unauthorized.` for a live link of
another account or one mailed to an address the account no longer has, and
keeps `400` for an unknown, expired or used token.

**Do:** Update clients and tests that expect `400` in those two cases.

### Verifying a verified account keeps its time.

**Before:** Verifying a verified account stamped a new time and fired
`EmailVerified`.

**Now:** `verify` consumes the token and returns the user ID, with no new
stamp and no `EmailVerified`.

**Do:** Nothing to do.

### Checking a password writes nothing.

**Before:** `validate_credentials` on `EloquentUserProvider` and
`DatabaseUserProvider` rewrote an outdated hash, so `Auth::validate` could
write to the database.

**Now:** It writes nothing. The sign-in (`attempt`, `once` and
`Auth::logout_other_devices`) rewrites the hash through
`UserProvider::rehash_password_if_required`, and a rewrite that fails still
fails the sign-in.

**Do:** Nothing to do. A custom `UserProvider` that rehashed in
`validate_credentials` moves that work to `rehash_password_if_required`.

## Authorization

### Nullable-user policies authorize guests.

**Before:** A guest got `401` before any policy ran.

**Now:** A policy accepting `Option<&User>` receives `None` for a guest and
decides whether the handler runs. A policy that requires a user keeps the
`401`.

**Do:** Check that each `Option<&User>` policy refuses guests where it
should.

### Handler authorization accepts enum abilities.

**Before:** Handler authorization took string abilities only.

**Now:** An enum variant that implements `Into<String>` reaches the same
async gate.

**Do:** Nothing to do.

### Denial codes survive `authorize`.

**Before:** `GateResponse::authorize` turned a denial built with `with_code`
into a domain error and dropped the code.

**Now:** It returns `FrameworkError::Denial`, and `FrameworkError::code()`
reads the code after `?`. A denial without a code maps as before.

**Do:** Where code matched the domain error for a coded denial, match
`FrameworkError::Denial` or read `code()`.

## Hashing

### Laravel's `$2y$` hashes verify

**Before:** Laravel's `$2y$` hashes of passwords of 72 bytes and longer did
not verify.

**Now:** They verify through `Auth::attempt` and Magnetar, judged on their
first 72 bytes as PHP does.

**Do:** Nothing to do.

### Password hashing runs under one process-wide limit.

**Before:** Each async hash and verify took a blocking-pool thread with no
limit.

**Now:** They share one limit, `HASH_MAX_CONCURRENCY`, which defaults to the
host's available parallelism. `0` or a value that is not a whole number is
refused when the hashing configuration loads.

**Do:** Nothing to do. If `HASH_MAX_CONCURRENCY` is `0` or not a whole
number, set a whole number or remove it.

## Query builder and pagination

### Query conditions follow SQL precedence.

**Before:** An `or_` helper folded into the condition before it, so
`where(a).or_where(b).where(c)` was `(a OR b) AND c`.

**Now:** Every `or_` helper adds a flat `OR`, so the same chain is
`a OR b AND c`. The grouped helpers keep their parentheses, and model scopes
still constrain the whole condition list.

**Do:** Review every chain with an `or_` helper after another condition.
Where you mean the grouped form, use a grouped helper such as `where_any`,
or `where_raw` with parentheses, as [Query Builder](queries.md) shows.

### Null equality values compile to `IS NULL`.

**Before:** A null `where` or `or_where` comparison was bound after `=`,
which matches no row.

**Now:** Both builders compile it to `IS NULL`.

**Do:** Nothing to do. Such a query returns the rows it names.

### Joined builders update and delete matching rows.

**Before:** An update or delete on a joined builder did not keep the join.

**Now:** Both builders keep the join when they write the base table, and
model deletion still honours soft deletes. The model builder also has
`update` and `delete`.

**Do:** Check joined updates and deletes: they touch only the rows the join
matches.

### Empty results have page one.

**Before:** Length-aware pagination with no matching rows reported a last
page of `0` and left the page-one link out.

**Now:** It reports a last page of one and includes the page-one link.

**Do:** Read the total, not the last page, to tell that a result is empty.

## Migrations and schema

### Unsigned MySQL IDs by default.

**Before:** `id()` and `foreign_id()` created signed `BIGINT` columns on
MySQL and MariaDB.

**Now:** They create `BIGINT UNSIGNED`, as Laravel does; Postgres and SQLite
keep signed columns. On a database with signed tables, a new table whose
`foreign_id` references an old signed key fails at `migrate` (MySQL errno
150), and an `i64` model field cannot read an unsigned column.

**Do:** Keep signed IDs in every migration your binary runs. Without
`#[suprnova::main]`, call `Schema::set_unsigned_ids(false)` before the
first migration. Or move to unsigned keys: convert the old keys, declare
keys as `u64` in models, and use `foreign_id` or `unsigned_foreign_id` for
the columns that reference them. [Migrations](migrations.md) has the detail.

```toml
[package.metadata.suprnova.schema]
unsigned_ids = false
```

### Nullable default timestamps.

**Before:** `timestamps()` created `NOT NULL` `created_at` and `updated_at`.

**Now:** It creates them nullable on every database. The scaffold documents
the schema defaults and carries the signed-ID opt-out and the model cast
setting commented out.

**Do:** Nothing to do. Existing tables keep their columns.

### Laravel boolean storage.

**Before:** Boolean columns took SeaQuery's type for each backend.

**Now:** MySQL and SQLite boolean columns are `tinyint(1)` and read back as
false and true.

**Do:** Nothing to do. Existing columns keep their type.

### Default float precision.

**Before:** Float columns were single precision on MySQL and Postgres.

**Now:** They use 53 binary digits, and the column modifier sets another
precision.

**Do:** Nothing to do. Existing columns keep their type.

### Whole-second zoned timestamps.

**Before:** Zoned timestamps kept fractional seconds on Postgres, and SQLite
declared another type for them.

**Now:** Postgres uses `timestamp(0) with time zone` by default, and SQLite
declares `datetime`.

**Do:** Nothing to do. Existing columns keep their type.

### Schema-qualified table names.

**Before:** A table name that holds a `.` was not read as a schema and a
table.

**Now:** On Postgres, `Schema::create`, `table`, `drop`, `drop_if_exists`
and `rename` read `schema.table` as that table in that schema, and a `.`
becomes `_` in index and foreign key names. On MySQL and SQLite, a
qualified name is refused with an error naming it.

**Do:** On MySQL and SQLite, give tables names without a `.`.

### MySQL tables default to `utf8mb4_unicode_ci`.

**Before:** New MySQL tables took the server's default collation.

**Now:** They default to `utf8mb4_unicode_ci`, and MySQL refuses to compare
string columns of two collations, so a query across an old table and a new
one can fail.

**Do:** Keep the server's collation for new tables. This is MySQL 8's
default; on MariaDB, use your server's default:

```env
DB_COLLATION=utf8mb4_0900_ai_ci
```

### `migrate:status` prints its list.

**Before:** `migrate:status` wrote its list to the log.

**Now:** It writes one line per migration, `Applied` or `Pending`, to the
standard output.

**Do:** Update scripts that read the list from the log.

## Seeding

### Production seeding requires force.

**Before:** `db:seed` ran in production without asking.

**Now:** It returns an error in production unless you pass `--force`.

**Do:** Pass `--force` to `db:seed` in production deploy scripts, to the
console binary or as `suprnova db:seed --force`, as [Seeding](seeding.md)
shows.

## Running on a Laravel database

### The shipped migrations upgrade an existing application in place.

**Before:** `failed_jobs`, `jobs`, `job_batches`, `sessions`,
`notifications`, `features`, `roles` and `permissions` had Suprnova's own
layouts.

**Now:** The shipped migrations move them into Laravel's layouts in place,
with every row. A queued, delayed or reserved job runs once, a failed job
keeps its id, a batch keeps its counts, and no user is signed out. A
`migrate` that stops part way resumes on the next run.

**Do:** Run the migrations. For an earlier scaffold's `users`, use the
migration in [Running on a Laravel Database](laravel-database.md).

```bash
suprnova migrate
```

### The RBAC, feature and notification entities follow Laravel's layouts.

**Before:** `rbac::entity::Role` and `Permission` had `i64` ids and
`display_name`, the pivot entities and the
`session::driver::database::sessions` entity existed, and the
`FeatureRow` and `StoredNotification` timestamps were not `Option`.

**Now:** `Role` and `Permission` have `u64` ids and no `display_name`, those
entities are gone, and the timestamps are `Option`. Rolling back
`CreateNotificationsTable`, `CreateFeaturesTable` or `CreateRbacTables`
leaves the shared tables.

**Do:** Read the ids as `u64` and the timestamps as `Option`, drop
`display_name`, and query `sessions` with the query builder.

## Eloquent

### Typed factory counts.

**Before:** `count(n)` left `make` and `create` returning one model;
`make_many` and `create_many` built the counted set.

**Now:** After `count` or `times`, `make` returns a vector and `create`
returns `Result<Vec<M>, FrameworkError>`. Without a count, both return one
model.

**Do:** Where code read one model from `.count(n).create()`, drop the count
or take the model from the vector.

### Collection metadata belongs to the collection.

**Before:** A JSON:API collection merged the first item's metadata into its
root.

**Now:** A collection keeps its own document metadata at the root.

**Do:** Nothing to do. A client that read item metadata from the
collection's root reads it from the item.

### Model ordering follows timestamp declarations.

**Before:** `oldest` and `latest` ordered by `created_at`.

**Now:** They order by the model's declared creation column, renamed
timestamps included.

**Do:** Nothing to do.

### Force destruction includes trashed rows.

**Before:** `force_destroy` left matching soft-deleted rows in place.

**Now:** It physically removes them too, and counts each existing key once.

**Do:** Check calls that relied on trashed rows surviving.

### Model visibility applies to serde and appends.

**Before:** Serde output did not share the hidden and visible lists, and
appended accessors were not filtered by them.

**Now:** Array, JSON and serde outputs share the hidden and visible lists,
appended accessors included.

**Do:** Check serde output of models with hidden fields: hidden fields are
gone from it.

### A clean model save skips the update.

**Before:** A save re-read the row and ran the update, firing `Updating` and
`Updated`, even when nothing had changed.

**Now:** Dirtiness comes from the instance's original values in memory, and
a clean save skips the update. `Saving` and `Saved` still fire; `Updating`
and `Updated` fire only for a dirty save.

**Do:** Move work that must run on every save from `Updating` or `Updated`
observers to `Saving` or `Saved`.

### Pivot sync returns the changed ids.

**Before:** `sync` and `sync_without_detaching` returned `()`.

**Now:** They return `SyncChanges`, with the attached, detached and updated
id lists.

**Do:** Where a function returned the call's result as its own, end with
`Ok(())`:

```rust
post.tags().sync(ids).await?; // the result is SyncChanges
Ok(())
```

### Pivot sync updates per-id columns.

**Before:** Both methods took ids alone.

**Now:** They also take `(id, Attrs)` records and update an existing row
only when its supplied columns change.

**Do:** Nothing to do.

### Pivot filters constrain sync.

**Before:** Both methods ignored `where_pivot` filters, so they could detach
rows outside the filter.

**Now:** They honour `where_pivot` and its companions, and rows outside the
filter are untouched.

**Do:** Check a sync on a filtered relation: it no longer detaches rows
outside the filter.

### Pivot sync touches a configured parent.

**Before:** Pivot sync did not touch the parent.

**Now:** A relation's `.touch_parent()` or a related model's inverse touch
declaration updates the parent timestamp, in the same transaction as the
pivot changes.

**Do:** Nothing to do.

### The naive date-time casts read MySQL `TIMESTAMP` columns.

**Before:** The naive date-time casts did not read MySQL `TIMESTAMP`
columns.

**Now:** They read them.

**Do:** Nothing to do.

### `has` and its kin read a polymorphic owner relation.

**Before:** `has`, `or_has`, `has_count`, `doesnt_have` and
`or_doesnt_have` on a `MorphTo` rendered a probe that is always false, so
`has` matched no row and `doesnt_have` matched every row.

**Now:** They ask every owner type the relation declares. `has_count`
refuses an operator other than `=`, `<>`, `!=`, `<`, `<=`, `>` and `>=`.

**Do:** Check queries that relied on the old results; they now match owners.

## Testing

### Console test assertions check every expectation.

**Before:** `ConsoleRun::assert_successful` passed with a prepared question
the command did not ask or an unmet output expectation, and an unexpected
question failed nothing.

**Now:** Each of these fails `assert_successful`, and an unexpected question
also fails `assert_failed` and the exit-code assertions, naming it.
`unmet_expectations()` and `unexpected_questions()` list them.

**Do:** Fix the tests that now fail: remove expectations the command never
meets, and prepare every question it asks.

### Queue assertions filter unchained jobs.

**Before:** `assert_pushed_without_chain::<J>()` took no argument.

**Now:** It takes a closure, and a matching job must also carry no chain.

**Do:**

```rust
assert_pushed_without_chain::<SendInvoice>(|_| true); // was assert_pushed_without_chain::<SendInvoice>()
```

### `TestResponse::assert_inertia()` reads a first visit.

**Before:** `assert_inertia()` refused any response without
`X-Inertia: true`.

**Now:** It also reads the page from a first visit's `data-page` element,
and `assert_inertia_with(callback)` runs a callback over the page.

**Do:** Nothing to do.

### `queue::testing::pushed::<J>()` fails on a payload that does not decode.

**Before:** `queue::testing::pushed::<J>()` left out a push it could not
decode, so `assert_not_pushed` passed over it.

**Now:** It panics, naming the job and the decode error.

**Do:** Fix the job type or its payload so every push decodes.

### `Queue::reserved_jobs` under `Queue::fake()` answers the fake.

**Before:** `Queue::reserved_jobs` under `Queue::fake()` read the installed
driver.

**Now:** It lists the reservations workers made from the fake.

**Do:** Nothing to do.

## Frontend and Inertia responses

### Development pages load an existing build when no hot file exists.

**Before:** In development, the Vite tags pointed at the dev server.

**Now:** They point at the dev server only while the hot file exists, and at
the manifest's files when it does not and a manifest exists. A frontend
that declares no `@inertiajs/vite` and holds an earlier build's manifest
loads that build.

**Do:** Declare `@inertiajs/vite`, as every starter kit does, or delete the
stale build.

### `Prop::match_on` replaces the match fields on each call

**Before:** `.match_on("x").match_on("y")` added to the fields and emitted
`matchPropsOn: ["posts.x", "posts.y"]`.

**Now:** Each call replaces the fields, so the chain emits `["posts.y"]`.

**Do:** Name several fields in one call:

```rust
.match_on(["x", "y"])
```

### `until` on a once prop reads an integer as seconds from now

**Before:** `.until(60)` read an absolute timestamp in milliseconds and
emitted `expiresAt: 60`, and `fresh()` took no argument.

**Now:** An integer is seconds from the render, so `.until(60)` emits the
render time plus 60 seconds. `fresh` on `Prop` and `OnceOptions` takes a
`bool`.

**Do:** Pass a `DateTime<Utc>` for a fixed deadline, and write
`.fresh(true)`:

```rust
.until(deadline) // a DateTime<Utc>; was a timestamp in milliseconds
.fresh(true)     // was .fresh()
```

### Scroll props merge under the wrapper `data` by default

**Before:** `.scroll("posts", metadata, paginator)` emitted
`mergeProps: ["posts"]`, and a wrapped prop prefixed its `match_on` path
with the wrapper.

**Now:** A scroll prop merges under its wrapper (`["posts.data"]`), and its
`match_on` path is relative to the prop. `.paginate` and `Inertia::paginate`
still merge at the root.

**Do:** Write `match_on` paths from the prop: `.match_on("id")` on a wrapped
scroll prop becomes `.match_on("data.id")`. For another bare list, call
`Prop::scroll_at_root()`.

### `sharedProps` lists `errors`, and a setting turns the list off.

**Before:** A page with nothing else shared carried no `sharedProps`.

**Now:** `sharedProps` starts with `errors` on every response.

**Do:** Nothing to do. To leave the list out of the page object, call
`.expose_shared_props(false)` on your `InertiaConfig`.

### `X-Inertia` counts as an Inertia visit for any value PHP reads as true.

**Before:** Only `X-Inertia: true` made a request an Inertia visit.

**Now:** Every value but an empty one and `0` does, for
`Request::is_inertia`, the `InertiaRequestExt` default, every middleware and
the development error page.

**Do:** Nothing to do.

### The Inertia asset version follows Laravel's order and is empty by default.

**Before:** With no Vite manifest to hash, the page carried the version
`1.0`.

**Now:** The version comes from `InertiaConfig::asset_url` (`ASSET_URL` by
default), then the manifest's hash, then the empty string. An explicit
`.version(...)` or `.version_with(...)` still wins.

**Do:** Nothing to do. A client that holds `1.0` reloads once.

### The asset-version 409 names an absolute URL and the current version.

**Before:** A stale Inertia `GET` got a `409` with a root-relative
`X-Inertia-Location` and no version.

**Now:** The `409` carries the request's absolute URL and
`X-Inertia-Version`, and `InertiaResponse::version_conflict(url)` adds
`X-Inertia-Version` too.

**Do:** Nothing to do.

### Partial reload headers are parsed as Laravel parses them.

**Before:** List entries were trimmed, and an empty
`X-Inertia-Partial-Data` matched nothing, so the client got no props.

**Now:** Lists split on `,` with empty segments dropped and nothing trimmed,
and a header that names nothing counts as absent. `X-Inertia-Reset` and
`X-Inertia-Except-Once-Props` follow the same rule.

**Do:** Send these lists without a space after each comma, as the Inertia
client does.

### Dotted partial-reload entries narrow literal values only, as Laravel's do.

**Before:** `only: ['users.name']` narrowed every prop's resolved value,
resolvers and `defer`, `optional`, `merge` and `once` props included.

**Now:** Dotted `only` and `except` entries narrow literal values only, and
the walk is Laravel's: a path that resolves to nothing yields `[]`, a path
walks into lists by index, and a scalar a deeper path reaches ships as it
is.

**Do:** Nothing to do. A client that relied on a narrowed resolver value
reads the whole value.

### Optional and deferred props resolve on an except-only partial reload.

**Before:** An `optional` or `defer` prop resolved on a partial reload only
when `X-Inertia-Partial-Data` named it.

**Now:** It resolves whenever its key passes the only and except lists, a
reload with `except` alone included. A standard visit still leaves it out.

**Do:** Nothing to do.

### `merge` and `once` instructions follow Laravel's partial-reload rule.

**Before:** Any `only` entry that selected a prop sent its `mergeProps` and
`onceProps` instructions, and a held `.merge().once()` prop still sent a
`mergeProps` entry.

**Now:** The instructions ship only when an `only` entry names the prop or an
ancestor of it, and a held once prop that is not deferred sends its
`onceProps` entry alone.

**Do:** Nothing to do.

### The Inertia page `url` carries the query as Laravel normalises it.

**Before:** `page.url` repeated the query as the request sent it.

**Now:** The query is parsed as PHP parses it, sorted by key and re-encoded
per RFC 3986. A `url_resolver`'s URL is left as it returns it.

**Do:** Update client code and tests that compare `page.url` with the raw
query.

### An empty Inertia response redirects back as Laravel's does.

**Before:** An empty `200` on an Inertia visit got a `303` to the session's
previous URL or `/`.

**Now:** It goes to the same-origin `Referer`, then the previous URL, then
`/`, with `302`, and `303` only for `PUT`, `PATCH` and `DELETE`.

**Do:** Nothing to do.

### A redirect with a fragment keeps it on an Inertia visit.

**Before:** A redirect to `/page#section` reached the client as a plain
`302`, and the fragment was lost.

**Now:** It arrives as `409` with `X-Inertia-Redirect`, except on a
prefetch.

**Do:** Nothing to do.

### Inertia visits record the previous URL.

**Before:** The session middleware skipped Inertia visits, so
`Redirect::back` and a failed validation landed on the last full page load.

**Now:** An Inertia `GET` that matched a route records its URL, unless it is
a prefetch, a Precognition request or a partial reload of the component it
rendered.

**Do:** Nothing to do.

### `errors` under `X-Inertia-Error-Bag` takes Laravel's shape.

**Before:** With the header, a visit with no errors got `{"<bag>": {}}`, and
a session `default` bag was dropped.

**Now:** No errors give `{}`, a default bag gives
`{"<bag>": <default bag>}`, and named bags come as they are.

**Do:** Update client code that read the empty bag object.

### Inertia flash data lives in the session until a page shows it.

**Before:** `App::flash` kept its value in the request, so a plain JSON
answer or a second redirect lost it.

**Now:** `App::flash`, `Inertia::flash` and `InertiaResponse::flash` write
the session entry `inertia.flash_data`. The next Inertia page emits it under
`page.flash` and removes it, and every redirect keeps it.

**Do:** Nothing to do.

### `clearHistory` and `preserveFragment` survive any number of redirects.

**Before:** `App::clear_history()` and `Redirect::preserve_fragment()`
flashed a flag for one request, so a second redirect lost it.

**Now:** The flags are the session entries `inertia.clear_history` and
`inertia.preserve_fragment`, which last until a page emits them.

**Do:** Nothing to do.

### A dotted shared key nests when it is shared.

**Before:** `App::inertia_share` stored `user.age` as a literal key and
nested it at render, so a later `share("user", ...)` kept `age`.

**Now:** A dotted key nests at share time, as Laravel's `Arr::set` does, so
a later `share("user", ...)` replaces `user` whole.

**Do:** Share a dotted key after the parent it extends.

### `InertiaResponse::location` answers a hard navigation with a redirect.

**Before:** `InertiaResponse::location` always answered `409` with
`X-Inertia-Location`, which a plain browser visit cannot follow.

**Now:** It answers an Inertia visit with the `409` and any other request
with a `302` and `Location`. `location_for(&req, url)` stays for code outside
a dispatched request.

**Do:** Nothing to do.

### A handler panic reaches the Inertia error page.

**Before:** A panic unwound past the error-page middleware, so an Inertia
visit got the JSON `500` and the client's plain-JSON modal.

**Now:** The same `500` reaches the error callback, and with
`error_page("Error")` the visit gets the `Error` page. `Inertia::install`
registers five middlewares, not four.

**Do:** Nothing to do.

## Inertia server-side rendering

### SSR is on by default, behind bundle detection.

**Before:** `SsrConfig::enabled` defaulted to `false`.

**Now:** It defaults to `true`, with the worker at `http://127.0.0.1:13714`,
and a first visit goes to the worker only when a bundle is found at
`ssr_bundle_path` or one of six conventional paths.

**Do:** Nothing to do. To keep SSR off when a bundle exists, call
`.ssr_disabled()` on your `InertiaConfig`.

### A missing SSR bundle is no longer logged.

**Before:** With a `bundle_path` set and no file there, each first visit
logged "SSR bundle not found".

**Now:** The visit renders on the client quietly, and `ssr:start` reports a
missing bundle. Under `ssr_throw_on_error`, a worker failure fails with
Laravel's `SsrException` text.

**Do:** Nothing to do.

### SSR exclusion patterns match as Laravel's `ExcludesPaths` does.

**Before:** `ssr_exclude` matched the raw path, with `*` stopping at `/`, so
`admin/*` excluded nothing.

**Now:** Slashes at either end of a pattern are ignored, `*` matches `/`
too, the path is decoded first, and each pattern is also tried against the
full URL.

**Do:** Review your patterns: a `*` now matches deeper paths.

### `suprnova ssr:start` makes Laravel's checks before it starts a worker.

**Before:** `suprnova ssr:start` ran the runtime with any configuration.

**Now:** It runs the application binary's `ssr:start`, which reads the
installed Inertia configuration, checks the bundle and the runtime, refuses
to start beside a running worker, and forwards signals to the worker.

**Do:** Configure SSR in `InertiaConfig`. The entry on the CLI's SSR
commands below shows the settings.

### `suprnova ssr:check` asks the SSR gateway's health check.

**Before:** `suprnova ssr:check` probed `/health` with its own client, its
own messages and its own exit codes.

**Now:** It asks the installed SSR gateway's health check, prints Laravel's
messages, and exits 0 or 1.

**Do:** Update scripts that read the old messages or exit code 2.

### The CLI's `ssr:start`, `ssr:stop` and `ssr:check` run the application binary, and the CLI does not link the framework crate.

**Before:** The CLI read `--bundle`, `--url`, `--timeout-ms` and the
`SUPRNOVA_SSR_URL`, `SUPRNOVA_SSR_RUNTIME` and `SUPRNOVA_SSR_BUNDLE`
variables.

**Now:** Each command runs the application binary, which reads its
`InertiaConfig`; the flags and the variables are gone. `--runtime` and
`--graceful` pass through.

**Do:** Move the settings into `InertiaConfig`:

```rust
InertiaConfig::new()
    .ssr("http://127.0.0.1:13714")
    .ssr_bundle_path("frontend/bootstrap/ssr/ssr.js")
    .ssr_runtime("node")
    .ssr_timeout(Duration::from_secs(5))
```

## Inertia DevTools

### Inertia DevTools recording is opt-in.

**Before:** `APP_ENV=local` turned recording and the entries endpoints on
without configuration.

**Now:** An application needs `.devtools(DevToolsConfig::new())`; without
it, nothing is recorded and no entries are served. A configured instance
keeps the same environment and `enabled(..)` rules.

**Do:** Add the call to the `InertiaConfig` you pass to `Inertia::install`,
as [Inertia DevTools](frontend-inertia-devtools.md) shows:

```rust
.devtools(DevToolsConfig::new())
```

## TypeScript types

### The starter kits declare `@inertiajs/core` and read `root` through the generated types.

**Before:** A kit reached `@inertiajs/core` only through its adapter, and
pages typed `root` with `usePage<{ root: string }>()`.

**Now:** Each kit's `package.json` declares `@inertiajs/core` at `^3.6.1`,
and the generated `inertia-props.ts` augments it, so a bare `usePage()`
types `root`. A hand-written `declare module '@inertiajs/core'` block under
`frontend/src` turns the generated augmentation off, with a notice.

**Do:** Declare `@inertiajs/core`, delete a hand-written augmentation block,
and regenerate:

```bash
cd frontend && npm install @inertiajs/core@^3.6.1
suprnova generate-types
```

## CLI and scaffolding

### The project file is `suprnova.toml`, all lowercase.

**Before:** `suprnova serve` and `live:add` read `Suprnova.toml`.

**Now:** They read `suprnova.toml`, and refuse to run while a
`Suprnova.toml` exists, naming the rename.

**Do:**

```bash
git mv Suprnova.toml suprnova.toml
```

### `suprnova new` ships a root template in place of `frontend/index.html`.

**Before:** A new project had a `frontend/index.html` the server never
served, and every first visit was the framework's own document, titled
`Suprnova`.

**Now:** A new project has `templates/app.html`, declared as `AppDocument`
with `#[inertia_root]` in `src/bootstrap.rs`, and the `docker:init`
Dockerfile copies `templates/` into the build stage.

**Do:** Nothing to do. To render first visits through your own document, add
a root template as [Inertia Responses](frontend-inertia-responses.md)
describes, and copy `templates/` in your Dockerfile's build stage.

### The starter keeps an existing notes table.

**Before:** The starter's notes migration created `notes` unconditionally.

**Now:** It checks for the table first, as the users migration does.

**Do:** Nothing to do.

### Generated middleware passes the request on.

**Before:** `make:middleware` wrote a `handle` that printed request timing.

**Now:** It writes a `handle` that returns `next(request).await`.

**Do:** Nothing to do.

## Fixed entries that change behaviour

These `### Fixed` entries change what an application observes, so they
appear here in the same shape.

### `Request::full_url` and `url::full` normalise their query as Laravel's `fullUrl` does.

**Before:** `Request::full_url` and `url::full` returned the query as it
arrived.

**Now:** They sort the pairs by key and re-encode them per RFC 3986
(`?b=2&a=1+x` gives `?a=1%20x&b=2`), with no `?` when the query normalises
to nothing. The previous URL the session and Inertia record is spelled the
same way. `query()`, `uri()`, `full_url_with_query` and signed URL checks
keep the query as sent.

**Do:** Update code and tests that compare, store or log the full URL as a
string.

### An Inertia visit the fallback route answers records the previous URL.

**Before:** An Inertia `GET` that the fallback route answered left the
session's previous URL where it was.

**Now:** It records the previous URL. `Request::is_fallback_route` marks
such a request, which still has no route pattern and no route name.

**Do:** Nothing to do.

### `#[derive(MultipartRequest)]` reads indexed and bracketed part names.

**Before:** The derive matched part names letter for letter, so `photos[0]`
never reached a `photos[]` field, and `photos[N]` names got around
`max_count`.

**Now:** A part goes to the field its name addresses, as a form request
reads it: `photos[0]` and `photos[]` fill a list declared `photos[]` or
`photos`, and `user[name]` fills `user.name`. A failure is keyed by the
sent index (`photos.3`), and `max_count` and the streaming file checks count
every part the field takes. A later parent part replaces what a field held,
and a member part such as `photos[0][caption]` makes the element an array,
which fails as the wrong kind.

**Do:** Update clients and tests that relied on literal names or on the old
error keys.

### A queue driver installed in the bootstrap survives the boot.

**Before:** `queue::bootstrap_from_env` replaced a driver that
`Queue::set_driver` installed in the bootstrap with the one `QUEUE_DRIVER`
names.

**Now:** It keeps the installed driver and does not read `QUEUE_DRIVER`
then. An application with a leftover `set_driver` call switches drivers on
upgrade, with no error.

**Do:** Remove a `Queue::set_driver` call from the bootstrap when
`QUEUE_DRIVER` should choose. A test that boots the queue from the
environment after a `set_driver` call empties the slot first:

```rust
suprnova::queue::testing::forget_driver();
```

### A console command whose boot fails is reported.

**Before:** A console command whose bootstrap panicked, or whose framework
boot returned an error, never reached `Exceptions::report`.

**Now:** The failure is reported, so the callbacks the bootstrap registered
before it failed see it, then printed. The application binary's own
commands report their errors too.

**Do:** Check that your reportable callbacks expect boot failures.

### `schedule:interrupt` boots the cache it writes its mark to.

**Before:** `schedule:interrupt` booted no cache store, so it failed with
`ServiceNotFound` unless the bootstrap bound one.

**Now:** It boots the store `CACHE_DRIVER` names, and keeps a store the
bootstrap bound.

**Do:** Nothing to do.

### `cursor_paginate` keeps the query's order.

**Before:** A query with one `order_by`, such as `latest()`, was paged by
the primary key ascending, and an ordering the cursor could not carry was
dropped.

**Now:** Every ordering is kept and carried in the cursor. An ordering the
cursor cannot carry, such as `order_by_raw`, and an ordering on a joined
table's column fail with a `400` error. A joined query quotes the columns it
orders by, so an unquoted select alias with capitals no longer matches on
Postgres.

**Do:** Order cursor-paginated queries by columns of the model's table, or
use `paginate` for raw and joined orderings.

### `simple_paginate` takes a page name, and `cursor_paginate` records its cursor.

**Before:** `simple_paginate` left `page_name` as `None`, and a
`cursor_paginate` page did not record the cursor it was fetched with.

**Now:** `simple_paginate` sets `page_name` to `page`,
`Builder::simple_paginate_using(page_name, per_page)` reads another
parameter, and a cursor page keeps `current_cursor`.

**Do:** Nothing to do. Code that expected `None` in `page_name` reads
`page`.

### `where_relation` on a `MorphTo` relation matches the owners.

**Before:** `where_relation`, `where_relation_op` and `or_where_relation` on
a `MorphTo` relation matched no row.

**Now:** They match through every owner type the relation declares, under
each owner's own scopes.

**Do:** Check queries that relied on the empty result.

### `#[model(timestamps = true)]` without the columns fails the build.

**Before:** An explicit `timestamps = true`, or the bare `timestamps` flag,
on a struct with neither `created_at` nor `updated_at` compiled and managed
no timestamps.

**Now:** It is a compile error that names both columns.

**Do:** Add the columns to the struct, or remove the flag.

### A database session read keeps a session another request refreshed.

**Before:** `DatabaseSessionDriver::read` deleted an expired row by id
alone, so a request that refreshed the same session in between lost its
write.

**Now:** The delete matches the row only while its `last_activity` is still
older than the lifetime. The read still answers no session.

**Do:** Nothing to do.

### The `Inertia` facade shares a once prop.

**Before:** Only `App::inertia_share_once` and `InertiaRegistry::share_once`
registered a shared once prop.

**Now:** `Inertia::share_once(key, resolver)` makes the same registration
and returns the same `SharedOnceProp`.

**Do:** Nothing to do.

### `suprnova generate-types` pairs a page built with the builder.

**Before:** A page whose chain started with `Inertia::paginate(..)` got no
`Pages` entry, and a field the chain deferred or marked optional was written
as required.

**Now:** A chain that starts with `InertiaResponse::new`,
`Inertia::paginate`, `Inertia::data` or `Inertia::try_data` pairs its
component with the struct `.with_data(..)` gives, and a key passed to
`.defer(..)`, `.defer_with(..)` or `.optional(..)` after it is optional.

**Do:** Regenerate the types and fix the TypeScript errors they report:

```bash
suprnova generate-types
```

### The Inertia DevTools gate's `GateEvaluated` event names the signed-in user.

**Before:** The DevTools entry endpoints' `GateEvaluated` event named the
erased `dyn Any` user type and carried no user identifier.

**Now:** It names the signed-in user's own type and carries the identifier.

**Do:** Nothing to do. A listener that matched the erased type name
matches the user's type.

### Every Live engine type a public signature names is named through `suprnova`.

**Before:** Some Live engine types that public signatures name had no path
through `suprnova`, so an application could not write them down.

**Now:** `suprnova::live`, `suprnova::live::ledger`, `suprnova::render_cache`
and `suprnova::render_cache::store` re-export them.

**Do:** Nothing to do.

### The component allowlist refuses test fakes, app-wide hooks and installs.

**Before:** The scan admitted `Http::fake`, `Queue::fake`, `Mail::fake`,
`Storage::fake`, `Process::fake`, `Http::global_options`, the global
request and response middleware, `Queue::set_driver`,
`Cache::set_connection`, a service's `install`, and the test container and
clock.

**Now:** The generator refuses them by name. The embedded allowlist is also
regenerated from the documented surface: documented paths it lacked are
admitted, and the Live engine's private paths and Magnetar's engine
internals are dropped.

**Do:** If `live:add` refuses a component it admitted on 3.2.1, move the
call it names out of the component.

### The script scan refuses every write to a member of the global object.

**Before:** The `live:add` scan admitted a write such as
`window.AcmeState = {}`, `window.onload = f` or `delete window.fetch`.

**Now:** Every write to a member of the global object, or of a window that
`window.open` returns, is refused (`script-global`), `delete` included.

**Do:** Keep a component's state in its module scope, not on `window`.

### What the script scan learns from member writes does not depend on statement order.

**Before:** The scan admitted a script in one statement order and refused
it in another.

**Now:** It settles names after its first walk, so the order no longer
matters, and a read of a global name nothing wrote is refused.

**Do:** If `live:add` refuses a component it admitted on 3.2.1, change the
component as the refusal says.

### A CLI built between two releases pins the framework commit it was built from.

**Before:** `suprnova new` pinned `tag = "v<CLI version>"` even when the CLI
was built from a later commit, so such a scaffold could fail `cargo check`.

**Now:** It pins the tag only when the tag names the commit the CLI was
built from, and `rev = "<commit hash>"` otherwise.

**Do:** In a project scaffolded with a `rev`, replace it with the release
tag when you move to a release:

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.0.0" }
```

## Added entries that change behaviour

These `### Added` entries change a path that 3.2.1 already had.

### A timebox on reset links.

**Before:** `PasswordReset::send_link` answered as soon as its work was
done, so an unknown address answered sooner than a known one.

**Now:** Every answer waits until `PASSWORD_RESET_TIMEBOX_MS` milliseconds
(200 by default) have passed since the call began.

**Do:** Nothing to do. Tests that must not wait set the hold to `0`:

```env
PASSWORD_RESET_TIMEBOX_MS=0
```

### Your own reset message.

**Before:** On the provider path, `PasswordReset::send_link` sent
`PasswordResetMail` itself, greeting the user by name.

**Now:** It sends through `UserProvider::send_password_reset_notification`.
`EloquentUserProvider` calls the model's
`CanResetPassword::send_password_reset_notification`, whose default mail
goes to `email_for_reset()` and greets the reader without a name. Another
provider's default returns an error when `flow_user_by_id` finds no user.

**Do:** To keep the name or send your own message, override
`send_password_reset_notification` on your user model, as
[Auth Flows](auth-flows.md) describes.

### Retry dates.

**Before:** `down --retry` took whole seconds.

**Now:** It takes seconds or a date, sends a date as `Retry-After` in the
RFC 7231 form, and refuses a value that is neither.

**Do:** Nothing to do.

### Namespaced translations.

**Before:** The browser catalog held plain keys.

**Now:** `Lang::add_namespace` reads a package's catalogs as
`namespace::key`, the browser catalog holds them as `namespace__key`, and
the starter kits' `t()` encodes keys the same way.

**Do:** Nothing to do. To read namespaced keys in a 3.2.1
kit frontend, copy the key encoding from a new kit's `t()`.

## Public types with new fields or variants

These public types changed shape without a changelog entry of their own.

### `MultipartValue::File` has an `image_size` field.

**Before:** The variant had no `image_size`.

**Now:** It carries `image_size: Option<(u32, u32)>`, an image's width and
height read from the part.

**Do:** A pattern on `MultipartValue::File` ends with `..` or names
`image_size`.

### `GroupItem` has a `Resource` variant.

**Before:** A route group held routes and nested groups.

**Now:** It also holds a resource, as `GroupItem::Resource`.

**Do:** An exhaustive `match` on `GroupItem` adds an arm for `Resource`.

### `RelationEntry` has three more fields.

**Before:** `RelationEntry` had no `target_join_key`,
`through_soft_deletes_column` or `morph_targets`.

**Now:** It has all three.

**Do:** Nothing to do. `#[suprnova::model]` fills them. A hand-built
`RelationEntry` names them.

### `UploadValidator` has a `validate_received` method.

**Before:** A validator checked a file part through `validate_final`.

**Now:** `validate_received` also runs, with what the extractor received.

**Do:** Nothing to do. Its default calls `validate_final`.

## Next

- [Changelog](../CHANGELOG.md) - every 4.0.0 entry in full
- [Installation](installation.md) - install the CLI and pin the framework
  version
