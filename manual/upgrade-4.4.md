# Upgrading to 4.4

This chapter moves an application from Suprnova 4.3.0 to 4.4.0. Each section
matches an entry in the 4.4.0 [changelog](../CHANGELOG.md).

## Who this chapter is for

- A person upgrading an existing application from 4.3 to 4.4.
- A maintainer looking up the exact bold lead sentence of a changelog entry.

## How to use this chapter

Find the changelog entry below, then read **Before**, **Now** and **Do**. On
MySQL and MariaDB, the first boot of 4.4 widens five of Magnetar's tables,
and on a large table that takes time. Read that entry before you deploy.

Agent tools are new: nothing is served to agents until you adopt them. Two
changes can need an edit: code that builds a Magnetar lifecycle event field
by field, and a `match` over one of three enums with no wildcard arm. With
the framework's default features on, Magnetar's `migrate` also creates eight
new tables. `Redirect::intended` ignores a few more kinds of stored value; read
that entry if your code stores the intended URL itself.

## Upgrade in this order

1. Point application dependencies and the CLI at the release you are
   upgrading to, `v4.4.0` or later. The examples pin the current release:

   ```toml
   suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.6.1" }
   ```

   ```bash
   cargo update -p suprnova
   cargo install --force --git https://github.com/eas4ai/suprnova.git --tag v4.6.1 suprnova-cli
   cargo check --all-targets
   ```

2. If your `Cargo.toml` lists the framework's features, as a scaffolded
   project's does, add `"agents"` and `"search-media"` to the list, as the
   [default features](#new-projects-get-every-default-feature-of-the-framework)
   section shows. A version bump does not add them for you.
3. On MySQL or MariaDB, plan the first boot as the Magnetar section
   describes.
4. Run your tests.

## Added entries

### Handlers become agent tools with `#[tool]`.

**Before:** 4.3 had no agent tools.

**Now:** `#[tool]` on a routed `#[handler]` makes it a tool, with an input
schema `#[tool_input]` derives and scopes `tool_scopes!` declares. A tool call
runs the route's middleware, validation, policies and rate limits. Tool names
use 1 to 64 ASCII letters, digits and `_`, for example `create_post`.
A Live island registers `{name}-{key}`, at most 64 characters in all.
The compiler and boot warn about names over 30 characters and descriptions
over 500; boot also warns about each input field without a description.

**Do:** Nothing unless you adopt it. To adopt it, follow
[Agents](agents.md). The `agents` feature must be on.

### An MCP endpoint serves the application's tools.

**Before:** 4.3 had no MCP endpoint.

**Now:** `Router::agents(AgentsConfig::new())` serves `POST /mcp` and the
OAuth discovery documents under the application's root.

**Do:** Nothing unless you adopt it. To adopt it, call `Router::agents` on
the router your `routes::register` returns, as [The MCP
endpoint](agents.md#the-mcp-endpoint) shows.

### Users approve MCP clients through a built-in OAuth server.

**Before:** 4.3 had no authorization server.

**Now:** `AgentsConfig::oauth_server(OAuthServer::new())` mounts an OAuth 2.1
authorization server for MCP clients under `/oauth`, with default approval
and connected-agents pages and `ConnectedAgents`.

**Do:** Nothing unless you adopt it. To adopt it, set `APP_URL` to the URL
clients reach the application at, append `suprnova::agents::migrations()` to
your migrator so its four tables exist, and follow [Users who approve a
client](agents.md#users-who-approve-a-client).

### Tokens from an outside identity provider sign users in.

**Before:** 4.3 had no outside issuer.

**Now:** `OutsideIssuer`, added with `AgentsConfig::resolver`, accepts user
tokens a provider you name signs.

**Do:** Nothing unless you adopt it. Choose the claim it matches users on
with care: see [Users from an outside
issuer](agents.md#users-from-an-outside-issuer).

### Agents hold accounts of their own under auth.md.

**Before:** 4.3 had no agent accounts.

**Now:** `AgentsConfig::agent_auth(AgentAuth::new())` serves auth.md v0.6.0
beside the OAuth server, with the `agents:key`, `agents:revoke-all` and
`agents:prune` commands.

**Do:** Nothing unless you adopt it. To adopt it, turn on the OAuth server
too, set `AGENT_AUTH_KEY_ID` and `AGENT_AUTH_PRIVATE_KEY` in every
environment (the `agents:key` command prints a pair), append
`suprnova::agents::migrations()` to your migrator, and follow [Agent
accounts](agents.md#agent-accounts). In tests, call
`suprnova::testing::install_test_agent_auth_key()` before you build the
router.

### Live actions can be agent tools, in the browser and headless.

**Before:** 4.3 had no Live tools.

**Now:** `#[tool]` on a Live action offers it to agents in the browser, and
`headless` over MCP too.

**Do:** Nothing unless you adopt it. Marking an action `#[tool]` narrows who
can click it to the callers it lists, so an action people also click lists
`callers = [user, agent]`, plus `public` for guests. See [Live actions as
tools](agents.md#live-actions-as-tools).

### Pages offer server tools to agents in the browser.

**Before:** 4.3 offered no tools to the browser.

**Now:** `Router::share_tools`, `with_tools` and `PageTools` choose the tools
each page registers with WebMCP. `WEBMCP_ORIGIN_TRIAL_TOKEN` adds the
`Origin-Trial` header to every HTML response.

**Do:** Nothing unless you adopt it. A page that offers only `public` tools
stays shared in RenderCache; see [Page tools and
RenderCache](agents.md#page-tools-and-rendercache).

### The React, Vue and Svelte kits register page and form tools.

**Before:** 4.3's kits had no agent helpers.

**Now:** a new project ships `src/lib/agents` with `installPageTools`,
`usePageTool` and `useFormTool`, and calls `installPageTools(router)` in its
entry.

**Do:** Nothing unless you adopt it. An application scaffolded before 4.4
has no `src/lib/agents`. To adopt the helpers, scaffold a new project of the
same kit and copy `src/lib/agents` and the `installPageTools(router)` call
from its entry file.

Your project keeps its copy of `src/lib/agents/core.ts` when you later move
to a newer tag. It reads a page's tools by the keys it knows and ignores the
rest, so a release that adds a field leaves it working. A change it must not
ignore raises the payload's `v`, and a `core.ts` that does not know the new
`v` registers no server tools. When a release's upgrade chapter says the
version moved, copy `src/lib/agents` again from a new project of that
release.

### Rust front ends compiled to WebAssembly register WebMCP tools.

**Before:** 4.3 had no WebAssembly crate.

**Now:** the `suprnova-webmcp` crate registers typed page tools and calls
server tools from a Rust front end.

**Do:** Nothing unless you adopt it. Depend on `suprnova-webmcp` at the same
tag as the framework.

### Tests call tools as any caller.

**Before:** 4.3 had no tool test helpers.

**Now:** `TestClient::tool`, `TestClient::live_tool`, `TestClient::mcp` and
`Agents::fake` call tools as a guest, a user or an agent.

**Do:** Nothing unless you adopt them. See [Testing](agents.md#testing).

### Test requests can come from an address and carry any body.

**Before:** a test request had no peer address, and `TestRequest` sent a body
only as JSON or a form.

**Now:** `TestClient::with_peer_address` and `TestRequest::body` set both.

**Do:** Nothing unless you adopt them.

### Applications build their own consent pages on `AgentApprovals` and `AgentClaims`.

**Before:** 4.3 had no consent pages.

**Now:** `AgentApprovals` and `AgentClaims` read and record the signed-in
user's decisions for an approval page and a claim page of your own, and
answer 403 to a tool call. `ConsentPageHeaders` keeps those pages unframed,
unstored and out of referrers on every answer.

**Do:** Nothing unless you adopt them. A route that serves an approval or a
claim page of your own runs `ConsentPageHeaders`, and the route that records
the decision runs the CSRF middleware.

### `url::same_origin_path` checks a return address, and `Redirect::intended_url` reads the intended one.

**Before:** 4.3 had neither function.

**Now:** `url::same_origin_path` accepts only a path on the application, and
`Redirect::intended_url()` reads the intended address without spending it.

**Do:** if a sign-in page of yours stores a `return_to` from the query with
`Redirect::set_intended_url`, check it with `url::same_origin_path` first, as
[Storing the intended URL](responses.md#storing-the-intended-url) shows.
Otherwise a link can send your users to another site after they sign in.

### An application's migrator can create the agents tables and a case-insensitive email index.

**Before:** 4.3 had neither migration.

**Now:** `suprnova::agents::migrations()` creates the OAuth server's and agent
auth's tables, and `UniqueEmailIgnoringCase::on(table, column)` makes the
database refuse two addresses that differ only in case.

**Do:** Nothing unless you adopt them. Before you add
`UniqueEmailIgnoringCase` to an existing users table, merge any accounts
whose addresses differ only in case: the migration fails on them and says how
many there are. On MariaDB and MySQL before 8.0.13, the column needs a unique
index of its own and a `_ci` collation, or the migration fails naming what is
missing.

### Tests build a router with agent auth without a signing key.

**Before:** 4.3 had no agent auth.

**Now:** `suprnova::testing::install_test_agent_auth_key()` gives the test
process a generated signing key when the environment names none.

**Do:** Nothing unless you adopt agent auth.

### `#[tool_output]` and `#[tool_schema]` derive schemas through the framework.

**Before:** 4.3 had no tool schemas.

**Now:** `#[tool_output]` marks a tool's output type, and `#[tool_schema]` a
type nested in a tool's input or output.

**Do:** Nothing unless you adopt agent tools.

### Tools can carry WebMCP's `debugging` hint.

**Before:** 4.3 had no tool hints.

**Now:** `annotations(debugging)`, the kits' `debugging` option and
`PageTool::debugging` mark a tool meant for developers.

**Do:** Nothing unless you adopt agent tools.

### New projects serve agents.

**Before:** a project 4.3 scaffolded had no agents wiring, no signing key and
no case-insensitive email index, and its sign-in page did not follow a
`return_to`.

**Now:** a new project gets a signing key in `.env`, and every kit serves
agents from `src/agents.rs` and creates the agents tables. The Inertia and
Live kits also create the email index, and their sign-in follows a
same-origin `return_to` and keeps the intended address through
registration. Every kit folds an address to ASCII lowercase before it
validates it, and bounds names at 255 characters and addresses at 254.

**Do:** an application scaffolded before 4.4 keeps its files. To serve agents
as a new project does, scaffold a new project of the same kit and copy:

- `src/agents.rs`, its `pub mod agents;` line in `src/lib.rs`, and the
  `agents::serve(...)` call around `routes::register()` in `cmd/main.rs`;
- the consent pages `src/agents.rs` names, with their controller and routes,
  as the next sections describe;
- `suprnova::agents::migrations()`, and in the Inertia and Live kits
  `UniqueEmailIgnoringCase::on("users", "email")`, in your migrator;
- a signing key in `.env` from the `agents:key` command, and its two names in
  `.env.example`;
- `src/email_address.rs`, and its `normalize` call wherever your sign-in,
  registration and reset forms read an address.

### The agents maintenance pass prunes the OAuth server's ended rows.

**Before:** 4.3 had no OAuth server.

**Now:** the pass in `serve` and `agents:prune` delete the OAuth server's
lapsed and revoked rows whenever the router mounts it.

**Do:** Nothing unless you adopt the OAuth server.

### The Inertia and Live kits ship their own approval, claim and connected-agents pages.

**Before:** 4.3's kits had no consent pages.

**Now:** a new Inertia or Live project renders the approval, claim and
connected-agents pages itself, on the consent facade, behind
`ConsentPageHeaders`.

**Do:** Nothing unless you adopt agents. To adopt the pages, copy
`src/controllers/agents.rs`, the consent routes and the pages from a new
project of the same kit, as
[Agents in the Inertia kits](starter-kits.md#agents-in-the-inertia-kits) and
[Agents in the Live kit](starter-kits.md#agents-in-the-live-kit) list them.

### The API kit serves agents, with a sign-in for consent only.

**Before:** 4.3's API kit served no agents and started no browser session.

**Now:** a new API project serves agents with the framework's default pages,
signs a browser in at `/login` for consent only, lists and revokes connected
agents over JSON, and verifies an address by a mailed link.

**Do:** Nothing unless you adopt agents. To adopt them as a new project does,
see [Agents in the API kit](starter-kits.md#agents-in-the-api-kit). The
session runs only on the consent pages, so your JSON API still starts none.

### `Auth::email_verification()` verifies an address for its signed-in owner.

**Before:** 4.3 had no `Auth::email_verification()`.

**Now:** `Auth::email_verification()` issues and verifies Magnetar-backed
links for the signed-in user, bound to the address each link was mailed to.

**Do:** Nothing unless you adopt it. See
[Email verification through Magnetar](auth-flows.md#email-verification-through-magnetar).

### Consent pages refuse a form tool's submission.

**Before:** 4.3 had no form tools or consent pages.

**Now:** a kit's form tool sends `Suprnova-Form-Tool`
(`suprnova::agents::FORM_TOOL_HEADER`), and `AgentApprovals::of` and
`AgentClaims::of` refuse it with 403.

**Do:** Nothing. A form tool of your own that posts a page's form sends the
header as the kits' does.

### New projects ship two notes tools.

**Before:** 4.3's kits shipped no tools.

**Now:** the Inertia and Live kits serve `search_notes` and `create_note`
under the `notes:read` and `notes:write` scopes.

**Do:** Nothing. They are a worked example for your own tools; see
[Agents in the Inertia kits](starter-kits.md#agents-in-the-inertia-kits).

### The Live input takes `maxlength`.

**Before:** the Live library's input had no `maxlength` parameter.

**Now:** it sets the attribute when a caller passes one.

**Do:** Nothing unless you adopt it. The attribute is a hint for the
browser; keep the length rule on the server.

## Changed entries

### `MagnetarLifecycleEvent` and `LifecycleEvent` carry an `attributes` map.

**Before:** both structs had `mutation_id`, `kind` and `user_id`.

**Now:** both also have `attributes: BTreeMap<String, String>`, which agent
auth events fill and every other kind leaves empty.

**Do:** where your code builds either struct with every field spelled out,
such as a test that fakes a lifecycle event, add `attributes:
Default::default()`, or build a `LifecycleEvent` with `LifecycleEvent::new`.
Where it destructures one, end the pattern with `..`.

### Three public enums have new variants.

**Before:** `LifecycleEventKind`, `CryptPurpose` and `HandlerArgKind` had the
variants 4.3 documented.

**Now:** `LifecycleEventKind` has nine agent auth kinds, from
`AgentRegistrationCreated` to `AgentRegistrationRevoked`. `CryptPurpose` has
`LiveAgentHandle`, and `HandlerArgKind` has `Context`.

**Do:** add a wildcard arm to a `match` over one of them that lists every
variant. A listener of `MagnetarLifecycleEvent` receives the agent auth kinds
only once agent auth is on, and their `user_id` is empty for a registration
no user has claimed.

### The `agents` feature is on by default.

**Before:** the framework had no `agents` feature.

**Now:** `agents` is a default feature. With it compiled in:

- Magnetar's `migrate` creates eight tables the first time it runs:
  `auth_oauth_clients`, `auth_oauth_authorizations`, `auth_oauth_grants`,
  `auth_oauth_tokens`, `auth_agent_registrations`, `auth_agent_grants`,
  `auth_agent_replays` and `auth_agent_events`. They stay empty until you turn
  the OAuth server or agent auth on.
- A request carrying the `Suprnova-Tool-Call` header to a route that serves
  no tool answers 400.
- An application with Live routes gains `POST /__live/agent`, which answers
  only the MCP endpoint's headless calls.
- `#[handler]` reads an argument of a type named `ToolCaller` as the tool's
  caller, and `#[live]` claims the `#[tool]`, `#[resolve]` and
  `#[agent_visible]` attributes.

Nothing is served to agents until the router calls `Router::agents` or names
page tools.

**Do:** nothing if you keep the defaults. If your `Cargo.toml` turns the
default features off and lists them, as a scaffolded project's does, the
feature stays off until you add `"agents"` to the list; your tests, whose
dev-dependency keeps the defaults, compile it either way.

### `generate-types` and `serve` write `frontend/src/types/tool-schemas.ts`.

**Before:** `suprnova generate-types` and `suprnova serve` wrote the Inertia
props types only.

**Now:** they also write `frontend/src/types/tool-schemas.ts`, the input
schema of each form request, and name each form request they cannot describe
exactly.

**Do:** commit the file with the other generated types, or ignore it as you
ignore them. A line naming a request with no schema matters only when a form
tool uses that request.

### `Redirect::intended` follows only a path, an `http` or `https` URL, or a relative reference.

**Before:** `Redirect::intended` ignored an empty stored value, a value with
a control character and a path that starts with `//`, `/\` or `\`, and
followed anything else.

**Now:** it also ignores a value with another scheme, such as `javascript:`,
`mailto:` or `localhost:3000/x`, a value with surrounding whitespace, and a
value with a backslash anywhere. It redirects to its fallback instead.
`Redirect::intended_url()` reads by the same rules.

**Do:** nothing, unless your code stores such a value with
`Redirect::set_intended_url`. Store an absolute `https` URL, a path that
starts with `/`, or a relative reference such as `?tab=2`, with no
surrounding whitespace. A user whose session holds an older value of another
kind lands on the fallback once. See [Storing the intended
URL](responses.md#storing-the-intended-url).

### An Inertia error page keeps a `no-store` the replaced response carried.

**Before:** the error page always set `Cache-Control: no-cache, private`.

**Now:** it sets `no-store` when the response it replaces said `no-store`,
and `no-cache, private` otherwise.

**Do:** nothing. A test that expected `no-cache, private` on the error page
of a `no-store` response expects `no-store` now.

### Magnetar's `auth_tokens` table gains an `address_digest` column.

**Before:** `auth_tokens` had no column for the address a token was mailed
to.

**Now:** it has a nullable `address_digest` string column, which holds the
SHA-256 of the address an email verification link was mailed to. Magnetar's
`migrate` adds it the first time it runs, which the framework does at boot
unless `MagnetarConfig` was built with `.apply_migrations(false)`.

**Do:** nothing if Magnetar migrates at boot. If you manage Magnetar's schema
yourself, add a nullable string column `address_digest` to `auth_tokens`, or
run Magnetar's `migrate` once, before you deploy.

### A custom Magnetar engine, store or token binding refuses email verification until it implements the new methods.

**Before:** `MagnetarPasswordAuthEngine`, `TokenStore`, `UserStore` and
`TokenFields` had the methods 4.3 documented.

**Now:** `MagnetarPasswordAuthEngine` has `email_verification_issue` and
`email_verification_verify`, `TokenStore` has `issue_bound` and `inspect`,
`UserStore` has `mark_email_verified_for`, and `TokenFields` has
`read_address_digest` and `write_address_digest`. Each default refuses.

**Do:** nothing unless you implement one of these traits yourself and call
`Auth::email_verification()`. Then implement the new methods, as Magnetar's
SeaORM store does, or every issue and verify fails closed. Code that does not
call it compiles and runs unchanged.

## Fixed entries that change behaviour

### Magnetar's tables hold long values on MySQL and MariaDB.

**Before:** on MySQL and MariaDB, Magnetar created its byte columns as
`varbinary(255)`, and the session user agent, the passkey credential id and
public key and the provider scope columns as `varchar(255)`. A longer value
failed to save.

**Now:** new tables get `blob`, `longblob` and `text` columns. The first time
`migrate` runs against an existing database, it widens these columns in five
tables: `auth_sessions`, `auth_methods`, `auth_ceremonies`,
`auth_two_factor` and `auth_provider_tokens`. MySQL and MariaDB change a
column's type by copying the table, and they block writes to that table
while they copy. The framework runs `migrate` when the application boots,
unless you built `MagnetarConfig` with `.apply_migrations(false)`.

**Do:** if `auth_sessions` is large, deploy in a quiet window. Start one
instance of 4.4 first and let it finish booting before you start the rest,
so only one process widens the tables. Sign-ins and new sessions wait while
`auth_sessions` is copied. If a health check stops that first instance
before it finishes, the copy is rolled back and the next boot starts it
again, so give the first boot enough time. If you added your own index on
one of these columns, `migrate` leaves that column narrow and logs a
warning naming the index. Drop the index, or give it a key length, and boot
again. Postgres and SQLite need nothing.

### Registration answers a validation failure with 422.

**Before:** `Auth::password().register` answered a weak password or an
unusable address with a 401 error.

**Now:** it answers the framework's validation error: 422, with the message
under the field (`password` or `email`) in `errors`. Sign-in still answers
invalid credentials as before.

**Do:** if your code or your front end matched the 401 from registration,
match the 422 validation error instead.

### Email addresses longer than 254 characters never reach the database.

**Before:** Magnetar accepted an address of any length. On MySQL a long one
failed to save, and sign-in with one answered 503.

**Now:** sign-in with an address longer than 254 characters answers as an
unknown address does. Sign-up, the account import and OAuth email completion
refuse one as a validation error. An account that already holds such an
address can no longer sign in by email. The first `migrate` on a database
logs how many such addresses `app_users` holds, without naming them.

**Do:** if that warning appears, contact those users and change their
address. Nothing else changes.

### New projects get every default feature of the framework.

**Before:** a project scaffolded by 4.3 turned the framework's default
features off and listed the ones it kept, and the list left out
`search-media`.

**Now:** a new project lists `search-media` and `agents` too.

**Do:** an application scaffolded before 4.4 keeps its list when you change
the tag, so add both to it to match a new project. Without `"agents"`,
`#[tool]`, `Router::agents` and the agent auth plugin do not compile; without
`"search-media"`, search cannot prepare uploaded media:

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.6.1", default-features = false, features = [
    # ...the features already listed...
    "search-media",
    "agents",
] }
```

### A form request on GET or HEAD reads its query.

**Before:** a form request on a GET or HEAD request without a JSON, form or
multipart body answered 415, unless it was a Precognition request.

**Now:** it reads the query string as it reads a url-encoded body, as
Laravel's form requests read a GET's query. It validates the query and
answers 422 naming each invalid field. A request with a body still reads the
body, and other methods are unchanged.

**Do:** nothing, unless a client or a test of yours expected the 415. A GET
route whose handler takes a form request now validates the query, so a link
to it with missing parameters answers 422 or redirects back with errors, as a
form post does.

### Precognition and the real request read a query alike.

**Before:** Precognition decoded a query value holding a JSON object into an
object wherever it stood, while `request.query_into()` read it as text.

**Now:** Precognition, a GET form request and `request.query_into()` read
such a value as an object where a struct or a map belongs, and as text
everywhere else.

**Do:** nothing, unless a Precognition-validated field typed
`serde_json::Value` expected an object from the query: it now receives the
text. Type the field as the struct or map it holds.

## Fixed entries with no upgrade step

- **Two servers migrating Magnetar's tables at once no longer collide.** Nothing to change.
- **Session metadata stays within its columns.** Nothing to change.
- **An `Option` Live mount parameter can be left out.** Nothing to change.
- **A page's scripts and styles no longer become the previous URL.** Nothing to change.
  `Redirect::back()` without a `Referer` returns to the last page a route
  answered.
- **Two uses of one email verification link at once no longer answer 500 on
  MariaDB.** Nothing to change.
- **Concurrent writes no longer fail on MariaDB 11.8 and later.** Nothing to
  change: the framework turns off `innodb_snapshot_isolation` on its own
  connections, and leaves the server's setting for your other clients.
- **`DB::transaction_with_attempts` retries by the database's error code.**
  Nothing to change.
- **A Postgres schema dump loads where an administrator installed an
  extension.** Nothing to change. Run `schema:dump` again to get a dump
  without the extension comments.

## Next

- [Agents](agents.md) documents tools, the MCP endpoint, the OAuth server,
  agent accounts and the browser helpers.
- [Authentication](authentication.md) documents Magnetar's schema and
  `apply_migrations`.
- [Upgrading to 4.3](upgrade-4.3.md) covers the preceding release.
