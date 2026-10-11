# Adversary brief: laravel-gaps-http

## Your role
Your task is READ-ONLY in the repo. You may use a scratchpad to record your finding but will clean up after yourself once you have created a report. Your report is your only output: deliver it to the builder and stop.

Rules:
- Do not build, run tests or run any project code. The receipts below say what already ran.
- Do not start subagents. Do the whole review inline, yourself.
- Keep your scratchpad and your report file outside the repository. A file written inside it changes the workspace, and sudus report refuses a report once the workspace differs from the reviewed snapshot.
- Read the whole specification under docs/spec, with its glossary and roadmap, and docs/decisions.jsonl before you judge anything. Judge this commitment as part of the whole system, not as a feature alone.
- Treat every claim in the builder's account as something to disprove, not as settled.
- Do not read the paths listed under Paths not to read.

You have no stake in Done. The builder wants this commitment finished; you do not inherit that goal. Your role is very important as it is to prove that the implementation for this commitment is not production ready. The builder will record your report and it will be committed to the repo as evidence. You are accountable for the contents of your report to the builder. Everything you report will be validated. Ensure that you are satisfied with the report before you give it to the builder. The user will use these reports to score the builders performance as well as yours. Accuracy and candidness is imperative.

If your report includes a bug report about Sudus, that you have discovered - stop the review, provide the bug report to the builder and stop. Sudus will likely require remediation before continuing the review.

Scrutinize the commitment against the requirement, not against the builder's account of it. Put the frozen text of each requirement beside what was built, and judge the work through five lenses:

1. The falsifier. Is the chosen failure the right observable failure for this obligation, or a weaker neighbour? Does the mechanism detect it, and could the code satisfy the check without meeting the obligation? Is there an input that reaches the falsifier which the check never exercises?
2. The decisions. Every one the builder made, recorded or not: the defaults, the assumptions, the convenient reading of an ambiguous line. Were the premises true? Was the option not taken closer to the text? Did any decision belong to the developer?
3. Security. The trust boundaries the change touched: input crossing them, paths and symlinks, secrets in code, records or output, new dependencies, new egress, new defaults.
4. Logic. Every new branch, catch, default and early return, and the input that takes the other side of it. Ordering, idempotency, partial failure.
5. Complexity and spec adherence. Behavior no requirement names. Behavior a requirement names that holds only on the happy path. Changed paths no mechanism covers. Documentation the code no longer matches. The same falsifier unreachable with less.

If you know there is a likely issue, start there.

A finding is a specific claim against the work: where (path and line, record, or requirement id), what is wrong, and why, quoting the requirement text or the decision it contradicts. Mark it Critical (a requirement is unmet, the falsifier is reachable, or a security exposure ships), Major (an uncovered defect in a touched path, or a decision that was the developer's) or Minor (an edge the commitment did not promise, or complexity the next commitment pays for). No fixes: a remedy is one line of advice, and optional. A note that is not a defect goes with the attempt that raised it, not in the numbered findings.

The attempts the brief requires are made by reading: for each pair, say what you looked for, what you found, and whether it held. Do not pad. A clean report earned by scrutiny is a good outcome. Developer rulings are settled; the implementation of them is not.

The user requires your best effort.

## Roadmap section
## laravel-gaps-http

Requirements: PAR-111, PAR-112, PAR-113, PAR-114, PAR-115, PAR-116, PAR-117, PAR-118, PAR-119, PAR-120, PAR-121, PAR-122

45. laravel-gaps-http - the tenth of the parity rounds the developer
   ordered on 2026-10-07 (17:35), ruled build by the parity default: the
   thirty-two rows of the log's routing, controller, request, response,
   URL, middleware, CSRF, validation, error, view and session areas where
   Suprnova lacks or differs from Laravel 13.35.0, grouped into PAR-111 to
   PAR-120, with PAR-121, the polymorphic owner relation under the
   existence filters, and PAR-122, the seeded SQLite order on a
   text-keyed model, both surfaced from the ninth round's reviews; `TrustProxies::at`, which the log keeps, and `View::with`,
   which chained `with` and `prop` calls already cover, stay as they are.
   Delivery: reportable callbacks, `Exceptions::report`, the stopped
   retries, the error's type name and the editor links in
   `framework/src/error/`, `framework/src/queue/` and the console; the
   CSRF methods, the JSON `_token` and the token a regenerated session
   gets in `framework/src/csrf/` and `framework/src/session/`; controller
   middleware, `without_middleware`, the `405` and `OPTIONS` answers and
   `route_has` in `framework/src/routing/`, `framework/src/middleware/`,
   the server and the `resource!` macro; the signature middleware and the
   URL helpers in `framework/src/routing/`; the guest redirect, the request
   host, `TrustHosts`, `ajax` and cookie deletion in
   `framework/src/http/`; the IP and numeric messages, `RequiredIf::when`,
   `ImageFile` and `Dimensions` in `framework/src/validation/`, the English
   catalog and the upload validators; `make:middleware`, `make:view` and
   `make:inertia` in `suprnova-cli`; the `MorphTo` dispatch of `has`,
   `doesnt_have`, `has_count` and `where_has` in
   `framework/src/eloquent/builder.rs` and the rowid carried through a
   union's projection for the seeded SQLite order, with their tests in
   the eloquent suite's `laravel_gaps` module; a `laravel_http_gaps` test module in
   each of the error, queue, session, routing, http and validation suites
   and in the CLI's `make_scaffolds` test; the manual chapters each area
   owns and the changelog.
   Mechanisms: `par-laravel-gaps-http` (the `laravel_http_gaps` modules of
   the six suites and the CLI test) and `par-laravel-gaps-data` (the
   `laravel_gaps` modules of the data suites, for PAR-121 and PAR-122).
   Done when it passes on a committed tree, the manual passages are in
   place, and the review is recorded.

Out of every commitment above, by the developer's 2026-09-12 ruling:
complete blocks (auth flows, account settings, CTAs, pricing, dashboard
layouts), tag input, nested menus, command palette, drag-reorder,
auto-infinite-scroll, carousel, image cropper, rich text editor, rating
input. They are a separate project and enter this roadmap only if the
developer writes them in.

Every commitment inherits the Live gate and the repository gate; a
commitment is Done only when its mechanisms pass on a committed tree and
its review is recorded (`sudus review`).

## Frozen requirements and falsifiers
[PAR-111] `Exceptions::reportable(callback)` MUST register a callback that every reported error reaches before the default log, typed as Laravel's closures are: a callback taking `&FrameworkError` runs for every error, and one taking another error type `E` runs only for an error whose wrapped source downcasts to `E`; `.stop()` on the returned registration MUST keep the later callbacks and the default log from running for an error it handles. `Exceptions::report(&error)` MUST report an error the code handles itself, and every error the framework turns into a `5xx` response, every failed queue job attempt and every console command that returns an error MUST be reported the same way; the `ErrorOccurred` event stays. `Exceptions::dont_retry::<E>()` and `Exceptions::dont_retry_when(predicate)` MUST name the errors that end a queued job's retries, `Exceptions::should_stop_retries(&error)` MUST answer for them, and the worker MUST fail a job at once, without a further attempt, when it is true.
Falsifier: a callback taking `&std::io::Error` does not run when `FrameworkError::from_external(io_error)` becomes a 500, or runs for `FrameworkError::internal("x")`; a registration with `.stop()` lets the `framework error` log line or a later callback run; `Exceptions::report(&err)` reaches no callback; a queued job that fails, or a console command that returns `Err`, reaches no callback; or a job failing with an error `dont_retry_when` accepts runs again before its tries are spent.

[PAR-112] The development error page MUST name the type of the error it reports, and `ErrorReport::type_name()` MUST return it: the path `std::any::type_name` gives for the concrete type handed to `FrameworkError::from_external` or `from_external_with` (for an `std::io::Error`, whatever `std::any::type_name::<std::io::Error>()` reports on the building toolchain), `FrameworkError::<Variant>` for the framework's own errors (`FrameworkError::ModelNotFound`), and `panic` for a panic. With `APP_EDITOR` set, every frame with a location MUST link to its line in the editor, as Laravel's `Frame::editorHref` does: a name from Laravel's editor list (`vscode`, `phpstorm`, `idea`, `cursor`, `zed`, `sublime` and the others `ResolvesDumpSource` names) uses that editor's URL format, a value holding `{file}` and `{line}` is a template filled with the absolute file path and the line, and any other name gives `<name>://open?file={file}&line={line}`; without `APP_EDITOR` no frame carries a link. The source lines shown for application frames stay.
Falsifier: the page for a 500 from `FrameworkError::from_external(std::io::Error::other("disk"))` does not contain the text `std::any::type_name::<std::io::Error>()` returns, or a panic's page does not name `panic`; with `APP_EDITOR=vscode` an application frame at `src/handlers.rs:12` carries no `vscode://file/<absolute path>/src/handlers.rs:12` link; `APP_EDITOR=myeditor://{file}#{line}` is not filled in; or a frame carries a link with `APP_EDITOR` unset.

[PAR-113] `CsrfMiddleware` MUST check the token on every method but `GET`, `HEAD` and `OPTIONS`, `QUERY` and extension methods such as `PROPFIND` included, as Laravel's `isReading` does, and MUST read `_token` from a JSON request body as it reads it from a form body: ahead of the `X-CSRF-TOKEN` and `X-XSRF-TOKEN` headers, with `""` and `"0"` counting as no value; the same-origin pass stays off by default. `regenerate_session_id()` MUST also issue a new CSRF token, as Laravel's `Session::regenerate` does, so a token read before it is refused after it; the old session row is still destroyed.
Falsifier: a `QUERY` request without a token reaches its `query!` route behind `CsrfMiddleware`, or a `PROPFIND` request without one answers anything but 419; a `POST` with `Content-Type: application/json`, the body `{"_token": "<session token>"}` and no header answers 419; a JSON body whose `_token` is `""` beside a valid `X-CSRF-TOKEN` header answers 419; a same-origin request without a token passes under the default policy; or after `regenerate_session_id()` the session's CSRF token is the one it had before.

[PAR-114] A resource controller MUST declare its own middleware, as Laravel's `HasMiddleware` does: `ResourceController::middleware()` returns a list of `ControllerMiddleware` values, each a middleware value or an alias name such as `"auth"` or `"throttle:60,1"`, scoped with `only(actions)` or `except(actions)`, and a module named by `resource!` MUST be able to declare the same list in a `pub fn middleware()`. The resource's routes MUST run each listed middleware on exactly the actions it is scoped to, after the group and route middleware, an unknown alias failing the registration as `middleware_named` does, and `ResourceRoutes::middleware` and `ResourceDef::middleware` MUST scope a `ControllerMiddleware` the same way at registration. A route MUST be able to leave out a middleware its group gives it, as Laravel's `withoutMiddleware` does, with `without_middleware::<M>()` or `without_middleware_named(name)`; the global middleware stays.
Falsifier: a controller whose `middleware()` lists `ControllerMiddleware::new(Auth).only(&[ResourceAction::Store])` runs `Auth` on `index` or not on `store`; `.except(&[ResourceAction::Index])` runs it on `index`; an alias in the list is not resolved, or an unknown alias registers; `ResourceRoutes::middleware` scopes differently from the trait's list; the controller's middleware runs before the group's; or a route with `.without_middleware::<EnsureJson>()` in a group with `EnsureJson` still runs it, or a sibling route stops running it.

[PAR-115] A request whose path matches no route of its method but a route of another method MUST answer `405` with an `Allow` header listing those methods (`HEAD` beside `GET`) and Laravel's message, `The DELETE method is not supported for route posts/1. Supported methods: GET, HEAD.`, and an `OPTIONS` request to such a path MUST answer `200` with an empty body and an `Allow` header joined without spaces, as Laravel's `getRouteForMethods` does; both answers run through the global middleware as the `404` does, a route's constraints count as they do for a match, and a path no route matches keeps the `404` or the fallback. `route_has(names)` MUST return true only when every name is registered, as Laravel's `Route::has` does.
Falsifier: `DELETE /posts/1` on a router holding only `GET /posts/{id}` answers 404, or 405 without `Allow: GET, HEAD` or with another message; `OPTIONS /posts/1` there answers other than 200 with an empty body and `Allow: GET,HEAD`; a CORS preflight to that path loses the CORS middleware's answer; a path no route matches, or one every route's constraints refuse, answers 405; or `route_has(&["home", "missing"])` is true while `missing` is unregistered.

[PAR-116] A `ValidateSignature` middleware MUST let a request with a valid signature through and answer every other one, a missing, wrong or expired signature alike, with `403` and the message `Invalid signature.`, as Laravel's `InvalidSignatureException` does; `ValidateSignature::relative(ignore)` and `ValidateSignature::from_alias_args`, which reads `relative` and then the parameters to ignore (`signed:relative,utm_source`), MUST build it, every form verifying the path and query Suprnova signs. The parameters it is told to ignore, by `ignore(names)`, by the alias arguments or for the whole application with `ValidateSignature::except(names)`, MUST be left out of the verified text, and `url::has_valid_signature_ignoring(request, names)` MUST leave them out the same way. `signature_verdict` keeps telling `Expired` from `Invalid`, and the key-sorted signed text and the refusal of a repeated `signature` or `expires` stay.
Falsifier: a route behind `ValidateSignature::new()` runs its handler for a signed URL with a changed query, an expired `temporary_signed_route` URL or no signature, or answers one of them with anything but 403 `Invalid signature.`; a valid signed URL is refused; `ValidateSignature::new().ignore(["utm_source"])` or the alias `signed:relative,utm_source` refuses a valid signed URL with `&utm_source=mail` appended; `has_valid_signature_ignoring(&request, &["utm_source"])` is false for it; or a repeated `signature` passes.

[PAR-117] `url::current(request)` MUST return the absolute URL of the request without its query, as Laravel's `current()` does: the `APP_URL` origin, the public root and the path, while `url::full`, the signature check and the redirects that send a request back keep the path and the query. `url::previous_path(fallback)` MUST return the path of the session's previous URL without its query, the public root or a trailing slash, `/` when nothing is left, and the fallback's path when no previous URL is recorded. `url::secure_with(path, segments)` MUST append each segment percent-encoded as a path segment, as Laravel's `secure($path, $parameters)` does, keeping the `APP_URL` base and the https upgrade. `Redirect::guest` MUST store the request's own path and query as the intended URL only for a `GET` request that does not expect JSON, and the session's previous URL for any other request, never the `Referer`, with the same-site check kept.
Falsifier: `url::current` for `GET /invoices?page=2` with `APP_URL=https://example.com` is anything but `https://example.com/invoices`; `url::full` there loses `?page=2`; `url::previous_path("/")` after a recorded previous URL `/billing/invoices/?page=2` under the root `/billing` is anything but `/invoices`, or with no previous URL anything but `/`; `url::secure_with("users", &["a b", "7"])` is anything but `https://<APP_URL host>/users/a%20b/7`; a `POST` or a JSON `GET` behind `Redirect::guest` stores its own URL as the intended URL, or does not store a recorded previous URL; a plain `GET` stores anything but its own path and query; or a `Referer` header becomes the intended URL.

[PAR-118] `Request::ajax()` MUST be true only for `X-Requested-With: XMLHttpRequest` exactly, as Symfony's `isXmlHttpRequest` compares it. `Request::host()` MUST lowercase the host, strip only a numeric port, and return `None` for a host holding anything but letters, digits, `-`, `_`, `.`, `:` and the brackets of an IPv6 literal, so `http_host` and `scheme_and_http_host` never carry one. A `TrustHosts` middleware, `TrustHosts::at(patterns, subdomains)`, MUST answer `400` with the message `Bad request.` for a request whose host is invalid or matches none of the trusted patterns, matched without regard to case as Laravel's are, `subdomains` adding the `APP_URL` host and its subdomains, which are also the default; it trusts every host in the local environment, as Laravel's does. `Cookie::forget` MUST take its path, domain and SameSite from the session configuration (`SESSION_PATH` or the public root, `SESSION_DOMAIN`, `SESSION_SAME_SITE`), so the deletion cookie matches the cookie it deletes; `Secure` stays forced and `forget_with` still overrides the path and the domain.
Falsifier: `ajax()` is true for `X-Requested-With: xmlhttprequest`; `host()` for `Host: Example.COM:8080` is anything but `example.com`, or for `Host: example.com:abc` drops `:abc`; `host()` for `Host: bad<host>` is not `None`, or `scheme_and_http_host` carries it; a request for `evil.test` behind `TrustHosts::at([r"^example\.com$"], false)` reaches its handler, or one for `api.example.com` behind `TrustHosts::new()` with `APP_URL=https://example.com` is refused; or `Cookie::forget("prefs")` under `SESSION_PATH=/app`, `SESSION_DOMAIN=.example.com` and `SESSION_SAME_SITE=strict` lacks `Path=/app`, `Domain=.example.com`, `SameSite=Strict` or `Secure`.

[PAR-119] The English catalog MUST carry `validation-ip`, `The { $field } field must be a valid IP address.`, so `#[validate(ip)]` no longer shows the generic message, and the numeric `max` message (`validation-range` with `$kind` `max`) MUST read `must not be greater than { $max }`. `RequiredIf::when(condition)` MUST build Laravel's `Rule::requiredIf`: a rule taking a boolean or a closure answering one, that requires the field with `validation-required`'s message when the condition holds and passes it otherwise. `ImageFile` MUST accept only the types Laravel's `image` rule does, JPEG, PNG, GIF, BMP, WebP, AVIF, HEIC and HEIF, and refuse TIFF, PSD, ICO and JPEG XL, SVG staying reachable only through an explicit `MimeType` allowlist; a `Dimensions` upload validator MUST check Laravel's `dimensions` constraints (`min_width`, `max_width`, `min_height`, `max_height`, `width`, `height`, `ratio`), refusing with `validation-dimensions`, `The { $field } field has invalid image dimensions.`
Falsifier: `#[validate(ip)]` on a field `address` holding `999.1.1.1` reports anything but `The address field must be a valid IP address.`; `#[validate(range(max = 10))]` on 11 reports `must be at most 10`; `RequiredIf::when(true)` passes an empty value, or `when(false)` or a closure answering false refuses one; `ImageFile` passes a TIFF, PSD, ICO or JPEG XL file, or refuses a JPEG, WebP, AVIF or HEIC one; or `Dimensions` with a `max_width` of 100 passes a PNG 101 pixels wide, or refuses one 100 pixels wide.

[PAR-120] `make:middleware` MUST write a middleware whose `handle` passes the request to `next` and returns its response, printing nothing, as Laravel's stub does; it MUST accept a nested name (`Admin/EnsureRole` writes `src/middleware/admin/ensure_role.rs` and declares the modules on the way), and `--test` MUST also write a test that runs the middleware. `make:view` MUST scaffold a checked view: the template under `templates/` (`admin.dashboard` and `admin/dashboard` both write `templates/admin/dashboard.html`) and a `#[view]` struct naming it, refusing to overwrite an existing file unless `--force` is given, with `--test` writing a test that renders it. `make:inertia` MUST accept a nested name, `--force` and `--test` the same way.
Falsifier: the file `make:middleware Audit` writes contains `println!` or does anything but return `next(request).await`; `make:middleware Admin/EnsureRole` fails or writes outside `src/middleware/admin/`; `--test` writes no test file; `make:view admin.dashboard` writes no template or no `#[view]` struct naming `admin/dashboard.html`, overwrites an existing file without `--force`, or refuses with it; or `make:inertia Admin/Users --force --test` is refused.

[PAR-121] The existence filters on the model builder MUST read a polymorphic owner relation (a `MorphTo`) the way Laravel's `has` does: `has`, `or_has`, `has_count`, `doesnt_have` and `or_doesnt_have` on such a relation MUST go through the morph existence query over every registered owner type, so a row whose type column names a registered type whose table holds the id counts as having its owner, and a row whose owner is missing, soft-deleted under the owner's scopes, or of an unregistered type does not; `where_has` and `where_doesnt_have` with a typed predicate on such a relation MUST apply the predicate to the owners of the predicate's model type, as `where_has_morph` with that one type does.
Falsifier: `MorphComment::query().has("commentable").get()` returns no comment while a comment points at an existing registered post; `doesnt_have("commentable")` leaves out a comment whose post row was deleted or whose type is unregistered; `has_count("commentable", "=", 1)` matches no comment with an owner; or `where_has::<MorphPost, _>("commentable", |q| q.filter("title", "kept"))` returns a comment whose post has another title.

[PAR-122] On SQLite, the model builder's seeded random order MUST shuffle rows whatever the type of the model's primary key: a text-keyed model ordered with two different seeds MUST return two different orders over the same rows, and the same seed MUST repeat its order, in a plain query and over a union of two queries, with every row returned once.
Falsifier: a model with a text primary key and eight rows whose keys start with letters returns the same order for `in_random_order_seeded(1)` and `in_random_order_seeded(2)` on SQLite; the same seed returns two different orders; or a union of two such queries fails or returns the same order for the two seeds.

## Mechanism definitions
par-laravel-gaps-data: {"command":"export RUST_MIN_STACK=33554432 CARGO_INCREMENTAL=0 && cargo nextest run -p suprnova --test database --test eloquent --test factory --test pagination --test resources --test schema --test seed -E 'test(/laravel_gaps::/)'","cwd":null,"documents":["CHANGELOG.md","manual/database-testing.md","manual/eloquent-collections.md","manual/eloquent-factories.md","manual/eloquent-mutators.md","manual/eloquent-relationships.md","manual/eloquent-resources.md","manual/eloquent-serialization.md","manual/eloquent.md","manual/migrations.md","manual/pagination.md","manual/queries.md","manual/seeding.md"],"identity":{"env":[],"image":null,"tools":{}},"inputs":["CHANGELOG.md","framework/src/console","framework/src/database","framework/src/eloquent","framework/src/factory","framework/src/lib.rs","framework/src/pagination","framework/src/resources","framework/src/schema","framework/src/seed","framework/tests/database","framework/tests/eloquent","framework/tests/factory","framework/tests/pagination","framework/tests/resources","framework/tests/schema","framework/tests/seed","manual/database-testing.md","manual/eloquent-collections.md","manual/eloquent-factories.md","manual/eloquent-mutators.md","manual/eloquent-relationships.md","manual/eloquent-resources.md","manual/eloquent-serialization.md","manual/eloquent.md","manual/migrations.md","manual/pagination.md","manual/queries.md","manual/seeding.md","suprnova-macros/src"],"requirements":["PAR-099","PAR-100","PAR-101","PAR-102","PAR-103","PAR-104","PAR-105","PAR-106","PAR-107","PAR-108","PAR-109","PAR-110","PAR-121","PAR-122"],"results":null}
par-laravel-gaps-http: {"command":"export RUST_MIN_STACK=33554432 CARGO_INCREMENTAL=0 && cargo nextest run -p suprnova --test error --test queue --test session --test routing --test http --test validation -E 'test(/laravel_http_gaps::/)' && cargo nextest run -p suprnova-cli --test make_scaffolds -E 'test(/laravel_http_gaps::/)'","cwd":null,"documents":["CHANGELOG.md","manual/cli-generators.md","manual/controllers.md","manual/csrf.md","manual/error-model.md","manual/errors.md","manual/middleware.md","manual/queues.md","manual/requests.md","manual/responses.md","manual/routing.md","manual/session.md","manual/urls.md","manual/validation.md"],"identity":{"env":[],"image":null,"tools":{}},"inputs":["CHANGELOG.md","framework/src/console","framework/src/csrf","framework/src/error","framework/src/error.rs","framework/src/http","framework/src/lib.rs","framework/src/localization/catalogs","framework/src/middleware","framework/src/queue","framework/src/routing","framework/src/server.rs","framework/src/session","framework/src/validation","framework/tests/error","framework/tests/http","framework/tests/queue","framework/tests/routing","framework/tests/session","framework/tests/validation","manual/cli-generators.md","manual/controllers.md","manual/csrf.md","manual/error-model.md","manual/errors.md","manual/middleware.md","manual/queues.md","manual/requests.md","manual/responses.md","manual/routing.md","manual/session.md","manual/urls.md","manual/validation.md","suprnova-cli/src","suprnova-cli/tests","suprnova-macros/src"],"requirements":["PAR-111","PAR-112","PAR-113","PAR-114","PAR-115","PAR-116","PAR-117","PAR-118","PAR-119","PAR-120"],"results":null}

## Receipts (what already ran; do not run it again)
PAR-111: current receipt c516cc4620f856f8f065433c10972a01103b2e76 from mechanism par-laravel-gaps-http: pass
PAR-111: fail receipt 1d51ec20528bbacaf25017fff09cbd6a6742f0c8 from mechanism par-laravel-gaps-http, the violating example its review binds
PAR-112: current receipt c516cc4620f856f8f065433c10972a01103b2e76 from mechanism par-laravel-gaps-http: pass
PAR-112: fail receipt 1d51ec20528bbacaf25017fff09cbd6a6742f0c8 from mechanism par-laravel-gaps-http, the violating example its review binds
PAR-113: current receipt c516cc4620f856f8f065433c10972a01103b2e76 from mechanism par-laravel-gaps-http: pass
PAR-113: fail receipt 1d51ec20528bbacaf25017fff09cbd6a6742f0c8 from mechanism par-laravel-gaps-http, the violating example its review binds
PAR-114: current receipt c516cc4620f856f8f065433c10972a01103b2e76 from mechanism par-laravel-gaps-http: pass
PAR-114: fail receipt 1d51ec20528bbacaf25017fff09cbd6a6742f0c8 from mechanism par-laravel-gaps-http, the violating example its review binds
PAR-115: current receipt c516cc4620f856f8f065433c10972a01103b2e76 from mechanism par-laravel-gaps-http: pass
PAR-115: fail receipt 1d51ec20528bbacaf25017fff09cbd6a6742f0c8 from mechanism par-laravel-gaps-http, the violating example its review binds
PAR-116: current receipt c516cc4620f856f8f065433c10972a01103b2e76 from mechanism par-laravel-gaps-http: pass
PAR-116: fail receipt 1d51ec20528bbacaf25017fff09cbd6a6742f0c8 from mechanism par-laravel-gaps-http, the violating example its review binds
PAR-117: current receipt c516cc4620f856f8f065433c10972a01103b2e76 from mechanism par-laravel-gaps-http: pass
PAR-117: fail receipt 1d51ec20528bbacaf25017fff09cbd6a6742f0c8 from mechanism par-laravel-gaps-http, the violating example its review binds
PAR-118: current receipt c516cc4620f856f8f065433c10972a01103b2e76 from mechanism par-laravel-gaps-http: pass
PAR-118: fail receipt 1d51ec20528bbacaf25017fff09cbd6a6742f0c8 from mechanism par-laravel-gaps-http, the violating example its review binds
PAR-119: current receipt c516cc4620f856f8f065433c10972a01103b2e76 from mechanism par-laravel-gaps-http: pass
PAR-119: fail receipt 1d51ec20528bbacaf25017fff09cbd6a6742f0c8 from mechanism par-laravel-gaps-http, the violating example its review binds
PAR-120: current receipt c516cc4620f856f8f065433c10972a01103b2e76 from mechanism par-laravel-gaps-http: pass
PAR-120: fail receipt 1d51ec20528bbacaf25017fff09cbd6a6742f0c8 from mechanism par-laravel-gaps-http, the violating example its review binds
PAR-121: current receipt 5a1194dc26fd7722114b2bdfef6e1ac6aeb99899 from mechanism par-laravel-gaps-data: pass
PAR-121: fail receipt 9730d4fc9c2652fac185a6631522a19c7428492c from mechanism par-laravel-gaps-data, the violating example its review binds
PAR-122: current receipt 5a1194dc26fd7722114b2bdfef6e1ac6aeb99899 from mechanism par-laravel-gaps-data: pass
PAR-122: fail receipt 2d8fd502c145a0a20e7451507dafd874342382d7 from mechanism par-laravel-gaps-data, the violating example its review binds

## Builder claims
mechanism par-laravel-gaps-http Q1 observed: Bound on fail receipt 1d51ec20528bbacaf25017fff09cbd6a6742f0c8 (exit 100): the mechanism command ran the one laravel_http_gaps test on the tree after the spec commit, nextest reported 1 failed, and the receipt records that exit; the check runs the laravel_http_gaps modules of the error, queue, session, routing, http and validation suites and the CLI's make_scaffolds test with CARGO_INCREMENTAL=0 and RUST_MIN_STACK raised.
mechanism par-laravel-gaps-http Q2 observed: The failure stands in for PAR-113's token-rotation clause; the clause-level evidence is every test in a laravel_http_gaps module of the six suites and the CLI test, one or more per Falsifier clause of each block, and a test outside those modules is invisible to the mechanism by construction.
mechanism par-laravel-gaps-data Q1 observed: Bound for PAR-121 on fail receipt 9730d4fc9c2652fac185a6631522a19c7428492c and for PAR-122 on 2d8fd502c145a0a20e7451507dafd874342382d7 (both exit 100), each a planted eloquent laravel_gaps test failing for the documented reason; the mechanism, declared in the ninth round, lists both requirements since 806e8eefd.
mechanism par-laravel-gaps-data Q2 observed: The failures stand in for PAR-121's has clause and PAR-122's two-seeds clause; the clause-level evidence is the eloquent suite's laravel_gaps modules morph_has.rs and text_keys.rs, one or more tests per Falsifier clause.
requirement PAR-111 Q3 observed: Exceptions (framework/src/error/reporting.rs): reportable(callback) over &FrameworkError or a wrapped source type with stop(), report(error) running the callbacks in order then the framework error log line unless stopped, dont_retry and dont_retry_when with should_stop_retries; the 5xx response path, the worker's failed attempts (dead-lettering at once when the application named the error) and the console's command errors report through it; tests in the error and queue suites' laravel_http_gaps modules.
requirement PAR-111 Q4 observed: Not covered: a panic the HTTP boundary catches, a 5xx a middleware builds itself, a timed-out job attempt and a console bootstrap failure, which do not report through Exceptions (backlog item exceptions-report-misses-the-boundary-and-timeout-paths); callbacks walking the source() chain (Laravel does not either). Pass receipt c516cc4620f856f8f065433c10972a01103b2e76 on 9674623b1, current on 007e11448.
requirement PAR-112 Q3 observed: ErrorReport::type_name (framework/src/error/report.rs) gives the std::any::type_name path for a wrapped external error, FrameworkError::<Variant> for the framework's own and panic for a panic, recorded beside the error without a new field on External; the development page shows it above the headline and links every frame with a location through APP_EDITOR (Laravel's editor names, a {file} template, or <name>://open?file=...&line=...), with the innermost application frame taking the error's creation site as its location; tests in the error suite's laravel_http_gaps module.
requirement PAR-112 Q4 observed: Not covered: a relative frame path (src/handlers.rs:12) end to end, asserted by library unit tests the mechanism does not run, since integration frames resolve to the test file's absolute path; an External built as a struct literal around a type nothing wrapped, which falls back to FrameworkError::External. Pass receipt c516cc4620f856f8f065433c10972a01103b2e76 on 9674623b1, current on 007e11448.
requirement PAR-113 Q3 observed: CsrfMiddleware (framework/src/csrf/middleware.rs) skips the check for GET, HEAD and OPTIONS only, so QUERY and extension methods such as PROPFIND answer 419 without a token while the Live read branch still passes a Live stream; a top-level string _token in a JSON body (a content type holding /json or +json) is read before X-CSRF-TOKEN and X-XSRF-TOKEN, with an empty string or "0" counting as no value and a repeated key counting by its last value; regenerate_session_id (framework/src/session/middleware.rs) issues a new CSRF token and still destroys the old session row; tests in the session suite's laravel_http_gaps module and the http suite's requests module.
requirement PAR-113 Q4 observed: Not covered: a JSON body over the form size limit, which reaches the handler with its _token unread rather than answering 413 (a JSON endpoint may allow a larger body); a non-string JSON _token, which counts as no value where Laravel answers 419. Pass receipt c516cc4620f856f8f065433c10972a01103b2e76 on 9674623b1, current on 007e11448.
requirement PAR-114 Q3 observed: ControllerMiddleware (framework/src/routing/resource.rs) with new(middleware), named(alias), only(actions) and except(actions) scoped as Laravel scopes them (only unset or listing the action, except not listing it; update covers PUT and PATCH); ResourceController::middleware() empty by default and a module's pub fn middleware() found by the resource! macro; ResourceRoutes::middleware and ResourceDef::middleware; each route runs group middleware, then the registration list, then the controller's list, then authorize_resource, and an unknown alias fails try_register; without_middleware::<M>(), without_middleware_named(name) and try_without_middleware_named(name) on RouteDefBuilder, AnyRouteDefBuilder, RouteBuilder and MultiMethodRouteBuilder, the by-type form also removing an alias that resolves to the type and a group name removing every member; resource! inside group! takes the group's prefix, name prefix, middleware, session block and bindings; tests in laravel_http_gaps/controller_middleware.rs.
requirement PAR-114 Q4 observed: Not covered: the fluent Router::group and Router::resource forms inside a group, which still cannot nest; outside code matching GroupItem exhaustively, which the new Resource variant breaks (a public enum, pre-1.0). Pass receipt c516cc4620f856f8f065433c10972a01103b2e76 on 9674623b1, current on 007e11448.
requirement PAR-115 Q3 observed: The server (framework/src/server.rs) answers 405 when the request's method has no route and another method does, with Allow listing those methods in Laravel's order (HEAD beside GET) and Laravel's message in the JSON error body; OPTIONS to such a path answers 200 with an empty body and Allow; both run through the global middleware so CORS still answers preflights; route constraints count as for a match and a path no method matches keeps its 404 or fallback; route_has(names) (framework/src/routing/router.rs) is true when every name is registered, an empty list included; tests in laravel_http_gaps/matching.rs, with three existing tests moved from 404 to 405.
requirement PAR-115 Q4 observed: Not covered: other suites asserting a 404 for a known path under the wrong method (the likely ones were grepped and run: routing, middleware, cors, server, http, error, session, validation, queue, inertia, path_prefix, static_files, authorization); a 405 for a path a fallback route would also serve, which keeps Laravel's precedence as implemented. Pass receipt c516cc4620f856f8f065433c10972a01103b2e76 on 9674623b1, current on 007e11448.
requirement PAR-116 Q3 observed: ValidateSignature (framework/src/routing/validate_signature.rs): a validly signed request passes, a missing, wrong or expired signature answers 403 with Invalid signature., no encryption key answers 500; ignored query names are dropped before the signed text is built, signature and expires cannot be ignored (framework/src/routing/signed.rs, verify_signature_ignoring, url::has_valid_signature_ignoring); tests in the routing suite's laravel_http_gaps/signatures.rs.
requirement PAR-116 Q4 observed: Not covered: a signed URL built by Laravel itself (the signing scheme is Suprnova's, as manual/urls.md's divergence note says); key rotation under the middleware, unchanged from the signing module's own tests. Pass receipt c516cc4620f856f8f065433c10972a01103b2e76 on 9674623b1, current on 007e11448.
requirement PAR-117 Q3 observed: url::current returns the APP_URL origin, root and path without the query (framework/src/routing/url.rs), with current_path_and_query kept crate-internal for the callers that need the query; previous_path(fallback) strips the root at a segment boundary; secure_with(path, segments) appends encoded segments; Redirect::guest stores the request's own URL for a page GET and the session's previous URL otherwise, never the Referer (framework/src/http/response.rs); tests in the http suite's laravel_http_gaps/urls.rs; the path_prefix suite's expectations moved to the absolute form.
requirement PAR-117 Q4 observed: Not covered: guest on a non-page request with no previous URL, which stores nothing where Laravel stores the root; secure_with on an absolute path, which appends where Laravel drops the segments (both in the manual). Pass receipt c516cc4620f856f8f065433c10972a01103b2e76 on 9674623b1, current on 007e11448.
requirement PAR-118 Q3 observed: Request::ajax (framework/src/http/request.rs) is true only for X-Requested-With: XMLHttpRequest exactly; host() lowercases, strips only a numeric port and returns None for an invalid or empty host, the checked host reaching Inertia's version check, the validation redirect, Live's transport and the render cache key; TrustHosts (framework/src/http/trusted_hosts.rs, TrustHosts::new and ::at(patterns, subdomains) -> Result) refuses a host outside its patterns except in the local environment and passes every valid host when it has no pattern; Cookie::forget and forget_with (framework/src/http/cookie.rs) take path, domain and SameSite from the session configuration with an explicit path or domain winning and Secure and HttpOnly always set; tests in the http suite's laravel_http_gaps/requests.rs.
requirement PAR-118 Q4 observed: Not covered: TrustHosts under unit tests, where Laravel also skips the check and Suprnova checks (local only, as the block says); an invalid TrustHosts pattern, which fails at boot rather than refusing every request (manual divergence note). Pass receipt c516cc4620f856f8f065433c10972a01103b2e76 on 9674623b1, current on 007e11448.
requirement PAR-119 Q3 observed: The English catalog gains validation-ip and validation-dimensions and the numeric max wording; RequiredIf::when(condition) over RequiredCondition (bool or Fn() -> bool) behaves as Required while the condition holds (framework/src/validation/rule.rs); ImageFile accepts jpeg, png, gif, bmp, webp, avif, heic and heif only; Dimensions<D: DimensionLimits> checks min and max width and height, exact width and height and ratio with Laravel's one-pixel tolerance, refusing a file whose size the first 16 KiB do not state (framework/src/http/upload/validators.rs, imagesize 0.15.0); tests in the validation suite's laravel_http_gaps module and the http suite.
requirement PAR-119 Q4 observed: Not covered: a JPEG whose metadata pushes the size header past the 16 KiB the upload extractor streams to validators, refused by Dimensions where Laravel reads the stored file; an SVG under Dimensions, refused where Laravel passes it unchecked (manual divergence note). Pass receipt c516cc4620f856f8f065433c10972a01103b2e76 on 9674623b1, current on 007e11448.
requirement PAR-120 Q3 observed: make:middleware writes a handle that returns next(request).await, accepts nested names declaring each module level, and --test writes a test through TestClient; make:view writes templates/<path>.html and a #[view] struct from a dotted or slashed name, refusing an existing file without --force, with --test; make:inertia takes nested names, --force and --test (suprnova-cli/src/commands/make_middleware.rs, make_view.rs, make_inertia.rs, generator.rs); every generator checks first and writes all files or none; tests in suprnova-cli/tests/make_scaffolds.rs inside mod laravel_http_gaps.
requirement PAR-120 Q4 observed: Not covered: make:view and make:inertia exit 0 on an existing file without --force (a warning, as Laravel and live:make do) while make:middleware exits 1 as before; the ignored scaffold_snapshot compile of a generated project, which the release gate runs. Pass receipt c516cc4620f856f8f065433c10972a01103b2e76 on 9674623b1, current on 007e11448.
requirement PAR-121 Q3 observed: has, or_has, doesnt_have, or_doesnt_have and has_count on a MorphTo relation go through the morph engine over the relation's declared owners (framework/src/eloquent/builder.rs: morph_to_relation, morph_owner_branch); doesnt_have keeps rows with a null type or a type no owner answers to; has_count maps the 0-or-1 owner count onto the comparison and refuses an operator it cannot read; where_has and where_doesnt_have with a typed predicate narrow to the predicate's owner type as where_has_morph with that type; load_existence_flags calls has; nine tests in laravel_gaps/morph_has.rs.
requirement PAR-121 Q4 observed: Not covered: where_relation on a MorphTo, which still matches nothing (backlog item where-relation-on-a-morph-to-matches-nothing); a MorphTo whose owners include the row's own table (backlog item self-referential-existence-probe-reads-one-table). Pass receipt 5a1194dc26fd7722114b2bdfef6e1ac6aeb99899 on 007e11448.
requirement PAR-122 Q3 observed: The model builder's seeded SQLite order shuffles {table}.rowid inside a query's own SELECT and, over a union with a seeded order, projects the rowid into every arm as __suprnova_row and orders the outer query by it, the paginate count deciding the same way and a joined write over such a union selecting one column (framework/src/eloquent/builder.rs: own_row, projects_row; framework/src/database/expression.rs); DB::table keeps its rowid form; seven tests in laravel_gaps/text_keys.rs: two seeds differ and each repeats on a text-keyed model, over a union and its pages.
requirement PAR-122 Q4 observed: Not covered: a seeded order over a union whose arms narrow their select so equal-valued rows were merged before (UNION now keeps distinct rows apart, in the manual); a view or WITHOUT ROWID table, which returns the engine's error on both builders (pinned by a test). Pass receipt 5a1194dc26fd7722114b2bdfef6e1ac6aeb99899 on 007e11448.
commitment laravel-gaps-http Q5 observed: What could still be wrong while every check passes: eight Opus lanes and one Haiku lane built from one start tree (269718a45) and the controller merged them in sequence, resolving CHANGELOG three times (both sides kept), manual/requests.md once (both divergence paragraphs kept), the http and routing suites' laravel_http_gaps module lists (unioned) and laravel_delta.rs (h2's helper kept) by hand, so a semantic conflict between lanes would show only as a compile error or a changed test; h1 and h3 edited framework/src/http/response.rs in different hunks, h7, h8 and h9 edited framework/src/eloquent/builder.rs in different regions, h5 and h6 edited framework/src/http/upload/validators.rs in different functions, and the chain's type check, the suites, workspace clippy and the Postgres pass exercise the merged result. A behaviour the requirements name but a test could pass without: the 405 answer's Allow order, pinned by string; the CSRF JSON token read before the headers, pinned; url::current's absolute form, pinned across suites. The gate's MariaDB pass on the merged tree found two schema fixtures with i64 keys against id()'s unsigned MySQL columns, fixed as fixtures (7dab3e2f3, 90fe19e0c); the kits' migration templates still declare signed keys under u64 models, recorded as a backlog item and being fixed in a worktree for the next round.
commitment laravel-gaps-http Q6 observed: Not tested: Postgres and MySQL execution of the new HTTP behaviour (the suites run in-process on SQLite; the Postgres pass ran on the merged tree and the MySQL pass runs the schema, eloquent and database engine tests, none of the HTTP suites); the dependency the Dimensions validator adds, imagesize 0.15.0, under cargo audit (the release gate's step); a scaffolded project's generated middleware, view and page compiled end to end other than the ignored scaffold_snapshot (run once by the kit-fix lane: 2 passed); the rustdoc JSON link check on the merged tree (each lane ran it on its own tree); Laravel-signed URLs against ValidateSignature (the signing scheme differs by design).

## Builder findings

## Agent decisions (each is a decision pair below)
decision 01M4GPTPDG1VEPGZPJSNH0E431: [Consequential] Correction: commitment 9's realize record named a MySQL run that had not happened. Rests on: PAR-102. Wrong if: the developer reads the realize record 01M4GEFYRNRVXV7Q2XHP6RW5MM as proof that the MySQL pass ran before commitment 9 closed. The realize record 01M4GEFYRNRVXV7Q2XHP6RW5MM (2026-10-09 09:44) says the live Postgres and MySQL enum-collection test (framework/tests/eloquent/casts_native_columns.rs) was added and run. At that time only the gate's Postgres pass had run (scripts/check-postgres.sh on 097df9914, 50 steps green, 08:50); the MySQL pass had not. A side review raised it at 11:03. The MySQL pass (scripts/check-mysql.sh, MariaDB on the host and the gate's MySQL 8.4 container) then ran: green on both engines at 12:08 on tree 007e11448, MariaDB 12.3 on the host and the gate's MySQL 8.4 container, after three schema fixtures (integer_aggregates, signed_columns, unsigned_pivots, cases, laravel_cases) were made consistent with the unsigned keys id() has created on MySQL since cde96cfe5; the enum-collection test passed on both engines (casts_native_columns::mysql_enum_collection_reads_and_writes_a_native_json_column). Nothing in this correction changes commitment 9's code; it corrects the record.
decision 310cae7a17b2bc35a6f2bf66e28496758c1b6bea: captured the backlog item where-relation-on-a-morph-to-matches-nothing from PAR-121: where_relation(relation, column, op, value) on a MorphTo relation still builds the generic probe with no target table and matches nothing; has, doesnt_have, has_count and where_has route a MorphTo through the morph engine since 2026-10-09 (PAR-121). Laravel's whereRelation on a MorphTo routes through whereHasMorph.; outside this commitment because: PAR-121 names has, or_has, has_count, doesnt_have, or_doesnt_have, where_has and where_doesnt_have; where_relation is a method it does not name
decision 23740f66d43bda826eb7d3d4f0fd00d060eddddf: captured the backlog item exceptions-report-misses-the-boundary-and-timeout-paths from PAR-111: Exceptions::report runs at the three points PAR-111 names (the 5xx response path, the worker's failed attempts, a console command's Err). Still unreported: a panic the HTTP panic boundary catches, a 5xx a middleware builds itself, a job attempt that times out, a job dead-lettered before it runs, and a console bootstrap failure. Laravel reports bootstrap failures and max-attempts-exceeded jobs through its handler. Noted 2026-10-09 by lane h1.; outside this commitment because: PAR-111 names the reporting points it binds (the 5xx path, the worker's failed attempts, a console command's error); the boundary, timeout and bootstrap paths are outside its text
decision ebdd3d8dbd277ba3052396f103b216e64945b7cb: captured the backlog item api-kit-user-model-declares-a-signed-id from PAR-045: The API starter kit's user model template (suprnova-cli/src/templates/files/api/src/models/user.rs.tpl) declares pub id: i64 while the kit's manifest leaves unsigned_ids at its default, so id() creates BIGINT UNSIGNED on MySQL and the model cannot decode its own key there (the MariaDB pass showed the same mismatch in framework/tests/schema/integer_aggregates.rs on 2026-10-09). The backend kit's models already declare u64. The dogfood app's models (app/src/models) declare i64 under the same default; they run on SQLite today.; outside this commitment because: the requirement binds what id() creates and the manifest setting; the kit template's field type is the scaffold's, outside the open HTTP commitment's text

## Interface obligations

## Changed paths (start snapshot to reviewed snapshot; A added, M modified, D deleted)
M .sudus/mechanisms/par-laravel-gaps-data.json
M CHANGELOG.md
M Cargo.lock
M docs/decisions.jsonl
M framework/Cargo.toml
M framework/src/console/mod.rs
M framework/src/csrf/middleware.rs
M framework/src/database/expression.rs
M framework/src/eloquent/builder.rs
M framework/src/error.rs
M framework/src/error/debug_page.rs
M framework/src/error/report.rs
A framework/src/error/reporting.rs
M framework/src/http/cookie.rs
M framework/src/http/mod.rs
M framework/src/http/request.rs
M framework/src/http/response.rs
A framework/src/http/trusted_hosts.rs
M framework/src/http/upload/validators.rs
M framework/src/inertia/headers_middleware.rs
M framework/src/inertia/validation_redirect_middleware.rs
M framework/src/lib.rs
M framework/src/live/action.rs
M framework/src/localization/catalogs/en/validation.ftl
M framework/src/middleware/identity.rs
M framework/src/middleware/mod.rs
M framework/src/payments/webhook_route.rs
M framework/src/queue/worker.rs
M framework/src/routing/macros.rs
M framework/src/routing/mod.rs
M framework/src/routing/resource.rs
M framework/src/routing/router.rs
M framework/src/routing/signed.rs
M framework/src/routing/url.rs
A framework/src/routing/validate_signature.rs
M framework/src/server.rs
M framework/src/session/config.rs
M framework/src/session/middleware.rs
M framework/src/validation/rule.rs
M framework/tests/database/query_helpers.rs
M framework/tests/eloquent/laravel_gaps.rs
A framework/tests/eloquent/laravel_gaps/morph_has.rs
A framework/tests/eloquent/laravel_gaps/text_keys.rs
M framework/tests/eloquent/query_helpers_model.rs
A framework/tests/error/laravel_http_gaps.rs
M framework/tests/error/main.rs
A framework/tests/http/laravel_http_gaps.rs
A framework/tests/http/laravel_http_gaps/requests.rs
A framework/tests/http/laravel_http_gaps/urls.rs
M framework/tests/http/main.rs
M framework/tests/http/multipart_validation.rs
M framework/tests/path_prefix/root.rs
A framework/tests/queue/laravel_http_gaps.rs
M framework/tests/queue/main.rs
M framework/tests/routing/laravel_delta.rs
A framework/tests/routing/laravel_http_gaps.rs
A framework/tests/routing/laravel_http_gaps/controller_middleware.rs
A framework/tests/routing/laravel_http_gaps/matching.rs
A framework/tests/routing/laravel_http_gaps/signatures.rs
M framework/tests/routing/main.rs
M framework/tests/routing/params.rs
M framework/tests/routing/verbs.rs
M framework/tests/schema/cases.rs
M framework/tests/schema/integer_aggregates.rs
M framework/tests/schema/laravel_cases.rs
M framework/tests/schema/signed_columns.rs
M framework/tests/schema/unsigned_pivots.rs
A framework/tests/session/laravel_http_gaps.rs
M framework/tests/session/main.rs
A framework/tests/validation/laravel_http_gaps.rs
M framework/tests/validation/main.rs
M manual/cli-generators.md
M manual/controllers.md
M manual/csrf.md
M manual/eloquent-relationships.md
M manual/error-model.md
M manual/errors.md
M manual/middleware.md
M manual/queries.md
M manual/queues.md
M manual/requests.md
M manual/responses.md
M manual/routing.md
M manual/session.md
M manual/urls.md
M manual/validation.md
A suprnova-cli/src/commands/generator.rs
M suprnova-cli/src/commands/make_inertia.rs
M suprnova-cli/src/commands/make_middleware.rs
A suprnova-cli/src/commands/make_view.rs
M suprnova-cli/src/commands/mod.rs
M suprnova-cli/src/main.rs
M suprnova-cli/src/templates/mod.rs
M suprnova-cli/src/ui.rs
A suprnova-cli/tests/make_scaffolds.rs
M suprnova-macros/src/lib.rs
M suprnova-macros/src/resource.rs

## Paths not to read
These hold credentials or are excluded from every model by the settings. Do not open them; any file matching one of the patterns is excluded too, tracked or not.
pattern **/.env
pattern **/.env.*
pattern **/*.pem
pattern **/*.p12
pattern **/*.pfx
pattern **/*.key
pattern **/id_rsa
pattern **/id_dsa
pattern **/id_ecdsa
pattern **/id_ed25519
pattern **/id_ecdsa_sk
pattern **/id_ed25519_sk
credential app/.env
credential suprnova-cli/src/templates/files/api/.env.tpl

## Report
Write your report as one JSON file outside the repository and give back its path; it goes to `sudus report` unchanged. It is an object with exactly these fields:
- "attempts": one {"question", "target", "looked_for", "found", "held"} for each pair below, spelled exactly as written. "looked_for" and "found" are text; "held" is true when the work held against what you looked for and false when it did not.
  falsifier PAR-111
  falsifier PAR-112
  falsifier PAR-113
  falsifier PAR-114
  falsifier PAR-115
  falsifier PAR-116
  falsifier PAR-117
  falsifier PAR-118
  falsifier PAR-119
  falsifier PAR-120
  falsifier PAR-121
  falsifier PAR-122
  decision 01M4GPTPDG1VEPGZPJSNH0E431
  decision 310cae7a17b2bc35a6f2bf66e28496758c1b6bea
  decision 23740f66d43bda826eb7d3d4f0fd00d060eddddf
  decision ebdd3d8dbd277ba3052396f103b216e64945b7cb
  security laravel-gaps-http
  logic laravel-gaps-http
  complexity laravel-gaps-http
- "interface_attempts": one {"path", "looked_for", "found", "held"} for each path below, each path a JSON string to copy as it is (none: write [])
- "findings": [{"n": 1, "severity": "Critical", "where": "...", "text": "...", "remedy": null}, ...], numbered from 1 in order, or [] when you found nothing. "severity" is Critical, Major or Minor; "where" is the path and line, record or requirement id; "text" is what is wrong and why; "remedy" is one line of advice or null.
- "sudus_bug": null, or the bug report when you stopped on a Sudus bug; then "attempts", "interface_attempts" and "findings" are [].
