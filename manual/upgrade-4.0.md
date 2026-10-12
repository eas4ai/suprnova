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
covers types that gained fields without an entry of their own. Every `### Fixed`
entry of the release is either a section here or listed under
[Fixes with no upgrade step](#fixes-with-no-upgrade-step).

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

### Runtime threads have an 8 MiB stack.

**Before:** 3.2.1's `#[suprnova::main]` built the Tokio runtime with Tokio's
default stack, 2 MiB for each worker and blocking thread.

**Now:** Each runtime thread has an 8 MiB stack, the size Linux gives the
main thread. The size is reserved address space; a thread uses memory only
as its stack is touched.

**Do:** Nothing to do. An application that set `RUST_MIN_STACK` to keep a
debug build from overflowing under a Live action can drop it.

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
`404`, an `OPTIONS` request included, or ran the fallback route when one
was registered.

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

**Before:** A route key that did not parse as the key's type answered `400`
and repeated the value. A key that broke a `unique_id` key's format was
looked up as given and answered the `404` a missing row gets.

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

**Before:** A handler that read a parameter its path did not declare failed
at request time, and two body readers were a compile error. 3.2.1 had no
binding fields, scoped child bindings or route binders.

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

**Before:** `Request::host()` kept the letter case the host was sent in, cut
everything after its last `:`, port or not, and refused no character.

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

**Before:** 3.2.1 had no `type/*` wildcards: `MimeType` compared each
allowlist entry with the file's type exactly, and an SVG passed only when
the allowlist named `image/svg+xml`.

**Now:** A `type/*` wildcard admits every subtype except `image/svg+xml`.

**Do:** Nothing to do. To accept every image type but SVG, name `image/*`;
to accept SVG too, name `image/svg+xml` as well.

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

**Before:** 3.2.1 had no `Exceptions` and no reportable callbacks. A 5xx
response wrote the `framework error` log line, and a console command's error
was printed but not logged.

**Now:** A 5xx response, each failed attempt of a queued job and a console
command's error run your `Exceptions::reportable` callbacks before the
`framework error` log line, and a console command's error is logged as well
as printed.

**Do:** Nothing to do. To act on these errors, register a callback with
`Exceptions::reportable`, as [Errors](errors.md) describes.

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

**Before:** S3 disks read the `S3_*` names, with two exceptions: the region
fell back to `AWS_REGION` and `AWS_DEFAULT_REGION`, and a disk with no
`S3_ACCESS_KEY` took its credentials from the AWS provider chain, which
reads `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`. `AWS_BUCKET`,
`AWS_URL`, `AWS_ENDPOINT` and `AWS_USE_PATH_STYLE_ENDPOINT` were not read.

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

**Before:** The fake recorded the headers you added and the URL as you
passed it, without the `Content-Type` a JSON or form body set or the
client's user agent. 3.2.1 had no base URL, URL parameters, query builder,
multipart body or global middleware.

**Now:** It records the request as it is sent.

**Do:** Update assertions on recorded headers that did not expect the body's
`Content-Type` or the user agent.

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

**Before:** Neither driver applied the EXIF orientation. The built-in driver
wrote output with no metadata, the orientation tag included, so a photo that
relied on the tag came out as stored; the `magick` driver kept the tag and
the metadata. Rotation corners and flattened transparency were black in JPEG
and GIF.

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
decimal representation: a `try_*` form returned an error, and the
infallible forms wrote Rust's `inf` or `NaN`.

**Now:** They use `∞` and `NaN`.

**Do:** Nothing to do. A test that asserted `inf` asserts `∞`.

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

**Before:** With no shared cache store, an `on_one_server()` task ran on
every server outside production, with a warning. In production,
`schedule:work` and `schedule:run` refused to start unless
`SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true`.

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

**Before:** 3.2.1's `live:add` had no lock or journal, and a new project's
`.gitignore` listed neither.

**Now:** It lists `/.suprnova-live.lock` and `/.suprnova-live-journal.json`.

**Do:** Add both lines to your `.gitignore`:

```bash
printf '%s\n' '/.suprnova-live.lock' '/.suprnova-live-journal.json' >> .gitignore
```

### A component's script can no longer replace a built-in.

**Before:** 3.2.1 had no script scan.

**Now:** `live:add` scans a component's scripts, and a write to a member of
a built-in object or function, such as `Object.keys = f`, is refused
(`script-builtin`).

**Do:** Nothing to do for an application that installs no such component.
If `live:add` refuses one, its author changes it as the refusal says.

## Authentication

### Suprnova never writes `users.remember_token`.

**Before:** The scaffold's `User` carried `update_remember_token` (the
`api` scaffold's did not), read `id` as `i64` and its timestamps as
`DateTime<Utc>`.

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

**Before:** 3.2.1 rewrote no password hash: `validate_credentials` on
`EloquentUserProvider` and `DatabaseUserProvider` only verified the
password, and a sign-in left an outdated hash as it was.

**Now:** It writes nothing. The sign-in (`attempt`, `once` and
`Auth::logout_other_devices`) rewrites the hash through
`UserProvider::rehash_password_if_required`, and a rewrite that fails still
fails the sign-in.

**Do:** Nothing to do. A custom `UserProvider` that rehashed in
`validate_credentials` moves that work to `rehash_password_if_required`.

## Authorization

### Nullable-user policies authorize guests.

**Before:** 3.2.1 had no nullable-user policies: a policy took `&User`, and
handler authorization answered a guest `401` before any policy ran.

**Now:** A policy accepting `Option<&User>` receives `None` for a guest and
decides whether the handler runs. A policy that requires a user keeps the
`401`.

**Do:** Nothing to do. To let a policy decide for guests, take the user as
`Option<&User>`, as [Authorization](authorization.md) shows.

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

**Before:** `DB::table` and the model builder refused an update or delete on
a joined builder with an error, since the statement would have ignored the
join.

**Now:** Both builders keep the join when they write the base table, and
model deletion still honours soft deletes. The model builder also has
`update` and `delete`.

**Do:** Nothing to do. A joined update or delete now runs and touches only
the rows the join matches.

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

**Before:** `save` ran the update and fired `Updating` and `Updated` even
when nothing had changed.

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

**Before:** Both methods refused a relation with `where_pivot` filters: they
returned an error and wrote nothing.

**Now:** They honour `where_pivot` and its companions, and rows outside the
filter are untouched.

**Do:** Nothing to do. Where you dropped a relation's `where_pivot` filters
to sync it, you can keep them now.

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

**Before:** `ConsoleRun::assert_successful` checked the exit code alone, so
it passed with a prepared question the command did not ask. An unexpected
question made `ask` return an error, which failed the run only when the
command passed the error on. 3.2.1 had no output expectations.

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
decode, so a test that read the list, or called `assert_pushed`, saw no such
push. 3.2.1 had no `assert_not_pushed`.

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
always registers the error-page middleware now; 3.2.1 registered it only
when `error_page` was set.

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

**Before:** With SSR on and a `bundle_path` set but no file there, each
first visit printed "SSR bundle not found" on the standard error, or passed
that message to `on_ssr_error` when one was set.

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

**Before:** `suprnova ssr:start` checked that the bundle file existed, then
ran the runtime on it, whatever the application's SSR configuration said.

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

**Before:** 3.2.1 had no Inertia DevTools: nothing recorded requests, and no
entries endpoint existed.

**Now:** Inertia DevTools records and serves its entries only for an
application that passes `.devtools(DevToolsConfig::new())` to its
`InertiaConfig`; without the call nothing is recorded and no entries are
served. A configured instance records when `APP_ENV` names `local`, or as
its `enabled(..)` says.

**Do:** Nothing to do. To record, add the call to the `InertiaConfig` you
pass to `Inertia::install`, as [Inertia DevTools](frontend-inertia-devtools.md)
shows:

```rust
.devtools(DevToolsConfig::new())
```

## TypeScript types

### The starter kits declare `@inertiajs/core` and read `root` through the generated types.

**Before:** A kit reached `@inertiajs/core` only through its adapter, and
the generated `inertia-props.ts` did not augment it. 3.2.1 had no `root`
prop.

**Now:** Each kit's `package.json` declares `@inertiajs/core` at `^3.8.0`,
and the generated `inertia-props.ts` augments it, so a bare `usePage()`
types `root`. A hand-written `declare module '@inertiajs/core'` block under
`frontend/src` turns the generated augmentation off, with a notice.

**Do:** Declare `@inertiajs/core` at the kits' range, delete any
hand-written augmentation block, and regenerate from the application root,
where `suprnova generate-types` reads `Cargo.toml`:

```bash
(cd frontend && npm install @inertiajs/core@^3.8.0)
suprnova generate-types
```

## CLI and scaffolding

### The project file is `suprnova.toml`, all lowercase.

**Before:** `suprnova serve` read its extra dev processes from the file
`Suprnova.toml` that 4.0.0 asks you to rename; `live:add` read no project
file.

**Now:** They read `suprnova.toml`, and refuse to run while a
`Suprnova.toml` exists, naming the rename.

**Do:**

```bash
git mv Suprnova.toml suprnova.toml  # the rename
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

**Before:** 3.2.1's starter had no notes migration.

**Now:** It checks for the table first, as the users migration does.

**Do:** Nothing to do.

### Generated middleware passes the request on.

**Before:** `make:middleware` wrote a `handle` that printed request timing.

**Now:** It writes a `handle` that returns `next(request).await`.

**Do:** Nothing to do.

## Fixed entries that change behaviour

These `### Fixed` entries change what an application observes, so they
appear here in the same shape.

### The full-stack Dockerfile copies the localization catalogs.

**Before:** The Dockerfile 3.2.1's `docker:init` wrote for a Svelte, React
or Vue project copied the binary, the schema dumps and `public/` into the
runtime image and not `lang/`, so a container started without the
catalogs `Localization::bootstrap` reads at boot.

**Now:** The generated Dockerfile's runtime stage copies `lang/` beside
`public/`.

**Do:** In a Dockerfile `docker:init` wrote before 4.0.0, add
`COPY lang/ ./lang/` to the runtime stage, after the `COPY` of `public`.

### The error callback sees the error responses from outside the Inertia stack.

**Before:** `InertiaConfig::error_page` decided only the responses that
passed through the error-page middleware, which `Inertia::install`
registered innermost. A middleware registered above `Inertia::install`
that answered without calling `next`, such as `CsrfMiddleware`'s `419` or
an outer rate limiter's `429`, and a panic in a middleware outside it never
reached it, so an Inertia visit got the plain-JSON modal.

**Now:** The server runs the same decision after the whole stack for every
error response nothing inside the stack decided, so those responses become
the page too. Such a page carries the shared registry and the shared
providers, but no session data, no detected locale and none of the
middleware hooks' shares. `Inertia::install` registers its error-page
middleware just outside the validation redirect.

**Do:** Nothing to do. To build those pages inside the visitor's session
and locale, register `InertiaErrorPageMiddleware` yourself, before
`Inertia::install` and outside the middleware whose rejections it covers,
as [Inertia Responses](frontend-inertia-responses.md#where-the-page-is-rendered)
shows.

### An error page placed outside the Inertia stack follows a fragment redirect.

**Before:** `InertiaErrorPageMiddleware` passed a response carrying
`X-Inertia-Location` through, but not one carrying `X-Inertia-Redirect`.
With `error_page` set, a handler's `InertiaResponse::redirect`, a `409`
with `X-Inertia-Redirect` and an empty body, was read as an error and
answered with the error page, so the client never followed the soft
redirect.

**Now:** An Inertia protocol response, one carrying `X-Inertia`,
`X-Inertia-Location` or `X-Inertia-Redirect`, is never handed to an error
callback, the default one included, wherever the middleware is registered,
so the client follows the redirect to its target, a `#fragment` included.

**Do:** Nothing to do.

### A drop-in Inertia form with a file, and a `GET` filter form, read as the client shapes them.

**Before:** A form request answered `415` for a `multipart/form-data` body.
Every reader matched names letter for letter, apart from a trailing `[]`
for a list, so `filters[status]=x` arrived as one literal key, not as the
member `status` of `filters`.

**Now:** A form request and `Request::input` read a multipart body, and an
`UploadedFile` field takes a file part and runs its validators. Bracketed
and indexed names in a url-encoded body, a multipart body and a query
string decode into nested data as PHP's `parse_str` reads them, up to 64
levels deep: `user[name]` is the member `name` of `user`, and
`photos[1]=b&photos[0]=a` is a list in index order. A field that fails is
named by its dotted path, such as `user.name` or `photos.1`. A multipart
body stays under the form request's `max_body_bytes`, 8 MiB by default, and
1,000 parts. `query_params` and `query_param` still read each name as sent.

**Do:** Read nested names into a nested struct or a list. A field renamed to
a literal bracketed name, such as `#[serde(rename = "filters[status]")]`,
no longer matches. Update tests that expected `415` for a multipart form
request.

### JPEGs from the built-in driver show the right colours everywhere.

**Before:** The `oxideav` driver wrote RGB samples behind a JFIF header, so
libjpeg-based decoders, ImageMagick and most browsers among them, showed
the wrong colours.

**Now:** It writes standard JFIF YCbCr, every component at full resolution
(4:4:4), so every decoder shows the colours the image held. The file's
header changes with the samples.

**Do:** Nothing to do. Encode again from their sources the JPEGs the driver
wrote on 3.2.1 where their colours matter, and update tests that compare
the encoded bytes.

### Redirects stay on the origin for a request-target that names another host.

**Before:** A request such as `GET //evil.example/x` or `GET /\evil.example`
that reached a fallback route had a current URL that a browser reads as
another host. `Redirect::refresh_for` sent the browser there,
`Redirect::guest` stored it as the intended URL, `Redirect::intended`
followed it after sign-in, and `url::full` returned `//evil.example/x` as
it was.

**Now:** `refresh_for` and `guest` check the request's URL with the rule
the previous URL follows: for such a request `refresh_for` redirects to the
root, and `guest` stores no intended URL and removes one already stored.
`intended` checks the stored value when it reads it, so such a value stored
on 3.2.1 is not followed, while an absolute URL the application stored
itself still is. `url::full` stays on the `APP_URL` origin.

**Do:** Nothing to do.

### Queue and failed-job table names with capitals or reserved words work.

**Before:** The jobs driver and the failed-jobs store wrote
`QUEUE_DB_TABLE` and `QUEUE_FAILED_DB_TABLE` into their SQL unquoted.
Postgres folded a name with capitals to lower case, so it missed a table
created under its exact name, and a reserved word such as `order` failed as
SQL.

**Now:** Both quote each part of the configured name, as the shipped
migrations do, so the name is the table's exact name on every database.

**Do:** Nothing to do for a lower-case name. On Postgres, if a name with
capitals worked on 3.2.1 because its table was created without quotes, set
the variable to the lower-case name the table has.

### `generate-types` reads `{post:slug}`

**Before:** `suprnova generate-types --routes` looked for a handler's form
request only in its first argument, so a handler that took a bound model or
a path value first, such as `update(post: posts::Model, form: UpdatePost)`,
got a route helper with no `data` argument.

**Now:** It skips every argument named after a path parameter and takes the
first other argument whose type is a form request, so that helper takes
`data: UpdatePost`. A `{post:slug}` segment is the parameter `post`.

**Do:** Regenerate the helpers and pass the form data to each helper that
now takes it:

```bash
suprnova generate-types --routes
```

### A prop holding `<!--<script>` no longer breaks the first Inertia page.

**Before:** The first visit's page JSON, in its
`<script type="application/json" data-page="app">` element, had `/` escaped
but `<` and `>` as sent. A prop such as `<!--<script>` kept the real
`</script>` from closing the element, so the mount element became script
text and the client could not start.

**Now:** `<` and `>` are written as `\u003c` and `\u003e` beside the `/`
escaping, as Laravel's `JSON_HEX_TAG` does. The client reads the same
values.

**Do:** Nothing to do. A test that searches the first visit's HTML for a
prop holding `<` or `>` searches for the escaped form.

### An empty SSR answer renders on the client.

**Before:** A worker answer of `{}`, or an object without `body`, was
inlined as an empty body, so the first visit's document had neither the
page data nor the mount element and the client could not start. An answer
of `null`, `false`, another value that is not an object, or bytes that are
not JSON, was a failure: the visit rendered on the client and
`on_ssr_error` fired, or it answered `500` under `ssr_throw_on_error(true)`.

**Now:** Each of those answers, and any answer without a non-empty `body`
string, renders on the client from the page data, as Laravel's does, and is
not reported as a failure.

**Do:** Nothing to do. A test that expected `on_ssr_error`, or a `500` under
`ssr_throw_on_error`, for such an answer gets neither.

### SSR workers at `https` URLs are reached.

**Before:** The SSR client spoke plain HTTP only and refused an `https` URL
in `.ssr(...)`, so the worker was never contacted: every first visit
rendered on the client and `on_ssr_error` fired, or the visit answered
`500` under `ssr_throw_on_error(true)`.

**Now:** The client speaks TLS through rustls, as the HTTP client facade
does, and verifies the worker's certificate against the platform's trust
store. The timeout, the response cap and the refusal of an oversized
`Content-Length` are unchanged.

**Do:** Nothing to do. If your SSR URL is `https`, first visits now render
on the worker, so check that it renders your pages before you deploy.

### A DevTools entry and `Request::route_name` name the route that matched by its method.

**Before:** `Request::route_name` looked the name up by the matched pattern
alone, so `GET /users` named `users.index` and `POST /users` named
`users.store` both reported whichever name was listed first, and an unnamed
`PUT /users` reported one of them too. `Request::route_is` read the same
name.

**Now:** The router records each name against its method and pattern, and
the server sets the matched route's name on the request, so each method
reports its own and an unnamed route reports `None`. `route_is` and the
DevTools entry follow it. A request that never went through dispatch, such
as one a test builds with `with_route_pattern`, still falls back to the
pattern lookup.

**Do:** Nothing to do. A `route_is` check on a path that several methods
share now matches only the method whose route carries the name.

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

**Before:** 3.2.1 had no `Exceptions`. A console command whose bootstrap
panicked, or whose framework boot returned an error, printed the failure
and nothing else saw it.

**Now:** The failure is reported, so the callbacks the bootstrap registered
before it failed see it, then printed. The application binary's own
commands report their errors too.

**Do:** Check that your reportable callbacks expect boot failures.

### `cursor_paginate` keeps the query's order.

**Before:** `cursor_paginate` replaced every ordering with the primary key,
ascending, so a newest-first query such as `latest()` came back oldest
first.

**Now:** Every ordering is kept and carried in the cursor. An ordering the
cursor cannot carry, such as `order_by_raw`, and an ordering on a joined
table's column fail with a `400` error. A joined query quotes the columns it
orders by, so an unquoted select alias with capitals no longer matches on
Postgres.

**Do:** Order cursor-paginated queries by columns of the model's table, or
use `paginate` for raw and joined orderings.

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

**Before:** `generate-types` wrote one interface per
`#[derive(InertiaProps)]` struct and no `Pages` map, and a field a handler
deferred or marked optional was written as required.

**Now:** A chain that starts with `InertiaResponse::new`,
`Inertia::paginate`, `Inertia::data` or `Inertia::try_data` pairs its
component with the struct `.with_data(..)` gives, and a key passed to
`.defer(..)`, `.defer_with(..)` or `.optional(..)` after it is optional.

**Do:** Regenerate the types and fix the TypeScript errors they report:

```bash
suprnova generate-types
```

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

### Web Push refuses endpoints that resolve to private or local addresses.

**Before:** `EndpointPolicy::Strict` checked the shape of a subscription
endpoint: `https`, a host name in place of an IP address, and a list of
local and cloud metadata names. It did not check where the name resolved,
so an endpoint whose name resolved to `127.0.0.1` or `10.0.0.5` was sent
to, and the transport looked the name up again when it connected.
`WebPushError::PushServiceRejected` carried up to 8 KiB of the push
service's response body in its `body` field, and `WebPushChannel` put
that text in its error.

**Now:** The strict policy resolves the name and refuses the send with
`WebPushError::RefusedDestination(AddressClass)` when any answer is a
loopback, private, link-local, multicast, unspecified, documentation,
reserved or cloud metadata address. A failed lookup returns
`WebPushError::Lookup`, which is retryable. The transport that
`WebPushClient::new` and `WebPushClient::with_client_builder` build
connects to an address that was checked, without a second lookup, and
`with_client_builder` replaces the builder's DNS resolver to do it.
`PushServiceRejected` has no `body` field: the first 200 bytes of the body
go to the debug log. `WebPushChannel` logs a refused destination as a
subscription that cannot be used, and the dispatch succeeds; its error for
a rejection names the status alone.

**Do:** If your push service runs on a private network, give its client
`with_endpoint_policy(EndpointPolicy::AllowAny)`. Endpoints at public push
services need nothing. In code that matches on `WebPushError`, drop `body`
from a `PushServiceRejected` pattern, add arms for `RefusedDestination`
and `Lookup` to an exhaustive `match`, and remove a subscription that
returns `RefusedDestination`, as you remove one that returns
`InvalidSubscription`. If you set a DNS resolver on the builder you pass
to `with_client_builder`, it is replaced; map a fixed address with
`ClientBuilder::resolve` instead.
### The same-origin WebSocket policy compares the scheme.

**Before:** `OriginPolicy::SameOrigin` compared the `Origin` host and port
with the `Host` header and ignored the scheme. A page at
`http://app.example.com` could open a socket on the application served at
`https://app.example.com`, and a TLS-terminating proxy needed no
configuration for the check to pass.

**Now:** It compares the `Origin` scheme, host and port with the request's
public origin: the scheme `Request::scheme` reports, which reads
`X-Forwarded-Proto` and `X-Forwarded-Ssl` from a trusted proxy only, the
host `Request::host` reports, and the port `Request::port` reports or the
scheme's default. It refuses with `403` an upgrade whose public scheme it
cannot determine: a trusted proxy that sends neither header or names a
scheme other than `http` or `https`, or either header from a peer that is
not a trusted proxy. A malformed handshake is answered `400` before its
origin is checked.

**Do:** Behind a TLS-terminating proxy, list the proxy in
`APP_TRUSTED_PROXIES` and make sure it sends `X-Forwarded-Proto`, as
`proxy_set_header X-Forwarded-Proto $scheme;` does in nginx, or give the
route an `OriginPolicy::AllowList` of its public origins. An application
whose browsers reach it with no proxy in between has nothing to do. See
[Same-origin behind a proxy](websockets.md#same-origin-behind-a-proxy).

## Fixes with no upgrade step

These `### Fixed` entries change nothing an application upgrading from 3.2.1
observes, or touch only the framework's own tooling and tests.

- **The Laravel parity map holds the Inertia adapter and client.** The parity map is the framework's own tooling.
- **Images are no longer copied on their way through a driver.** It removes copies inside the drivers, and counts the memory of the ICC profiles 4.0.0 reads and keeps: on 3.2.1 the built-in driver dropped a profile, and the `magick` driver left it to ImageMagick.
- **The Inertia CRUD tutorial compiles** The fix is to the manual's tutorial, not to the framework.
- **Live upload and async routes follow the configured endpoint** Serving under a path prefix is new in 4.0.0, and at the host root the routes are the same `/__live` routes.
- **Batch settlement counts work on MySQL** It concerns a `job_batches` table Laravel created, which 4.0.0 is the first release to support.
- **On Postgres, a session write for a guest** It concerns Laravel's `sessions` layout, new in 4.0.0; on 3.2.1 `sessions.user_id` was text.
- **A component's scripts import by relative path only.** The script scan is new in 4.0.0.
- **A component's script can no longer reach a built-in prototype through a computed key or a name.** The script scan is new in 4.0.0.
- **A destructuring assignment's defaults, nested keys and shorthand targets are scanned.** The script scan is new in 4.0.0.
- **The script scan follows a value by the names it was written with.** The script scan is new in 4.0.0.
- **The script scan sees the global object through an expression.** The script scan is new in 4.0.0.
- **Writing a browser global no longer unlocks reading it.** The script scan is new in 4.0.0.
- **Magnetar's API documentation builds without the `two-factor` feature.** It changes API documentation only.
- **API documentation writes ranges in words.** It changes API documentation only.
- **An Inertia page that cannot be encoded answers an error.** Encoding a page does not fail in practice, so no response changes.
- **DevTools redacts a flattened key by its parts.** DevTools is new in 4.0.0.
- **DevTools redacts the URLs in headers and bodies.** DevTools is new in 4.0.0.
- **A lost DevTools index no longer drops the entries it listed.** DevTools is new in 4.0.0.
- **A commented-out augmentation no longer hides the generated one.** The generated `@inertiajs/core` augmentation is new in 4.0.0.
- **A hook-shared prop names a file and line.** It changes only what DevTools records, and DevTools is new in 4.0.0.
- **A page whose document fails is recorded as the error the client got.** It changes only what DevTools records, and DevTools is new in 4.0.0.
- **Every render prop has a `renderSource`.** It changes only what DevTools records, and DevTools is new in 4.0.0.
- **DevTools reads a request body as the handler would.** DevTools is new in 4.0.0, and without it the handler reads its body as on 3.2.1.
- **A DevTools JSON request body is recorded whatever it parses to.** DevTools is new in 4.0.0.
- **DevTools no longer reads an upload before its authorization.** DevTools is new in 4.0.0.
- **The DevTools id tag finds a closing body tag in any case.** DevTools is new in 4.0.0.
- **A tagged first visit no longer carries the untagged page's length.** The DevTools id tag is new in 4.0.0.
- **`schedule:interrupt` boots the cache it writes its mark to.** `schedule:interrupt` is new in 4.0.0.
- **Every Live engine type a public signature names is named through `suprnova`.** It only adds paths, so code that built on 3.2.1 is unaffected.
- **The Inertia DevTools gate's `GateEvaluated` event names the signed-in user.** DevTools is new in 4.0.0.
- **`simple_paginate` takes a page name, and `cursor_paginate` records its cursor.** Both fields are new in 4.0.0; the struct literals are covered under Public types with new fields or variants.
- **What the script scan learns from member writes does not depend on statement order.** The script scan is new in 4.0.0.
- **The script scan refuses every write to a member of the global object.** The script scan is new in 4.0.0.
- **The Laravel-database tests run each SQLite test on one connection.** It changes the framework's own tests.
- **The component allowlist refuses test fakes, app-wide hooks and installs.** A component's Rust, and the allowlist that reads it, are new in 4.0.0; a 3.2.1 component carried no Rust.
- **Every Live and Magnetar type a public signature leads to is named through `suprnova`.** It only adds paths, so code that built on 3.2.1 is unaffected.
- **The dogfood application's password reset reports success only when both revocations succeed.** The dogfood application is not a published surface.

## Added entries that change behaviour

These `### Added` entries change a path that 3.2.1 already had.

### Third-party Live component libraries.

**Before:** `live:add` installed a component of the shipped library by
name, or a third-party one with `--manifest`, by copying the view,
stylesheet and script its `manifest.json` named under `templates/<root>/`,
with no scan and no question. A component carried no Rust.

**Now:** `live:add` also installs from a library's git repository by
address, from an `https://` URL and from a `./path`, and a component can
carry the Rust of a Live component. Before it writes anything, it scans
every view, stylesheet, script and Rust file as data, admits only what it
can classify, and shows a plan. For a third-party component it asks you on
a terminal to approve each capability the component uses, to pin the
library's key on its first install and to confirm the plan; without a
terminal it refuses unless you pass `--yes`, and `--allow <capability>`
approves a capability. A shipped component still installs with no
questions.

**Do:** Nothing to do for the shipped library. Run a third-party install on
a terminal, or pin the library's key in `suprnova.toml` yourself and pass
`--yes` and `--allow` for each capability the plan names. If the scan
refuses a component you installed on 3.2.1, its author changes it as the
refusal says. See [Installing Live Components](live-add.md).

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

### `Paginator` has a `page_name` field.

**Before:** The simple `Paginator` had `data`, `current_page`, `per_page`,
`has_more` and `path`.

**Now:** It also has `page_name: Option<String>`, the query parameter the
page is read from, `None` for `page`. It is not serialized by itself.

**Do:** A struct literal that builds a `Paginator` names `page_name`. Code
that reads the fields is unchanged.

### `CursorPaginator` has a `current_cursor` field.

**Before:** `CursorPaginator` had `data`, `per_page`, `next_cursor`,
`prev_cursor`, `path` and `cursor_name`.

**Now:** It also has `current_cursor: Option<String>`, the cursor the page
was fetched with, `None` for the first page. It is not serialized by
itself.

**Do:** A struct literal that builds a `CursorPaginator` names
`current_cursor`. Code that reads the fields is unchanged.

## Next

- [Changelog](../CHANGELOG.md) - every 4.0.0 entry in full
- [Installation](installation.md) - install the CLI and pin the framework
  version
