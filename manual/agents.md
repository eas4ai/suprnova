# Agents

Suprnova lets AI agents use your application through tools. A tool is an
ordinary routed handler marked `#[tool]`. Agents reach it two ways:

- **Over MCP.** An MCP client, such as Claude Code or an agent that runs on a
  server, calls the application's MCP endpoint at `/mcp`.
- **In the browser.** An agent in the user's browser calls the tools a page
  offers through WebMCP (`document.modelContext`), with the user's session.

Either way, a tool call is a request to the handler's route. It runs the
route's middleware, bindings, form request validation, `#[authorize]`
policies and rate limits, so a tool can do nothing its route would refuse.
Nothing is a tool unless you mark it.

Every tool says which callers it accepts. A `public` tool needs no sign-in. A
`user` tool acts for a signed-in user, by browser session or by a token the
user approved for an MCP client. An `agent` tool accepts an agent account
that registered under [auth.md](https://github.com/workos/auth.md). This
chapter covers all three.

## The pieces

| Type | Role |
|---|---|
| `#[tool]` / `#[tool_input]` | Mark a handler as a tool and derive its input schema |
| `tool_scopes!` | Declare the scopes a token caller can be granted |
| `ToolCaller` | The caller a tool handler and its policies read |
| `AgentsConfig` / `Router::agents` | Serve the MCP endpoint and its discovery documents |
| `OAuthServer` / `ConnectedAgents` | Let users approve MCP clients, and list and revoke what they approved |
| `OutsideIssuer` | Accept user tokens from an identity provider you already run |
| `AgentAuth` / `AgentClaimed` | Agent accounts under auth.md, and the event a claim dispatches |
| `AgentApprovals` / `AgentClaims` / `ConsentPageHeaders` | Build your own approval and claim pages |
| `Router::share_tools` / `with_tools` / `PageTools` | Choose the server tools each page offers in the browser |
| `WebMcpConfig` | The WebMCP Origin Trial token |
| `suprnova-webmcp` | Page tools for Rust front ends compiled to WebAssembly |
| `TestClient::tool` / `McpTestClient` / `Agents::fake` | Call tools in tests as any caller |

The `agents` feature turns all of this on. It is a default feature of the
framework. A project that lists its features, as a scaffolded one does, needs
`"agents"` in that list.

## Quick start

Declare a scope, mark a handler, route it, and serve agents:

```rust
use suprnova::agents::AgentsConfig;
use suprnova::{Response, Router, ToolCaller, handler, json_response, post, routes, tool, tool_input};

suprnova::tool_scopes! {
    /// The scopes a client can be granted.
    pub mod scope {
        POSTS_WRITE = "posts:write" => "Create and edit your posts";
    }
}

/// A draft post to create.
#[tool_input]
pub struct CreatePost {
    /// The post's title.
    #[validate(length(min = 1, max = 200))]
    pub title: String,
}

#[tool(
    name = "create_post",
    description = "Create a draft post for the signed-in user.",
    callers = [user, agent],
    scopes = [scope::POSTS_WRITE],
    input = CreatePost,
)]
#[handler]
pub async fn store(caller: ToolCaller, form: CreatePost) -> Response {
    json_response!({ "title": form.title, "user": caller.user_id() })
}

mod web {
    use super::*;

    routes! {
        post!("/posts", store).name("posts.store"),
    }
}

/// `Application::routes(routes::register)` calls this once at boot.
pub fn register() -> Router {
    web::register().agents(AgentsConfig::new())
}
```

The router now serves `POST /mcp`. Point an MCP client at it, for example
Claude Code:

```bash
claude mcp add my-app --transport http https://app.example.com/mcp --scope user
```

Give the client the host `APP_URL` spells, `app.example.com` here. The
protected resource metadata names the endpoint under `APP_URL`, and a client
checks that against the URL it was given; `localhost` and `127.0.0.1` are
different hosts to it, so a mismatch stops sign-in before it starts.

`create_post` accepts users and agents, so calling it needs a token. The
client sees it in `tools/list` without one, and its first call answers 401,
which sends the client to sign in. Turn on the
[OAuth server](#users-who-approve-a-client) or
[agent accounts](#agent-accounts) to issue tokens. A `public` tool works with
no token at all.

A project made with `suprnova new` serves all of this from the first run. Its
`search_notes` and `create_note` tools, under the `notes:read` and
`notes:write` scopes, are a worked example of the sections below: see
[Agents in the Inertia kits](starter-kits.md#agents-in-the-inertia-kits) and
[Agents in the Live kit](starter-kits.md#agents-in-the-live-kit).

## Tools

### Mark a handler

`#[tool]` sits above or below `#[handler]` on a non-generic `async fn` that a
route serves. It takes these keys:

| Key | Required | Meaning |
|---|---|---|
| `name` | yes | What agents call the tool: 1 to 64 ASCII letters, digits and `_`, unique in the application |
| `description` | yes | A non-empty string literal. An agent reads it to decide when to call the tool |
| `callers` | yes | The callers it accepts: one or more of `public`, `user` and `agent` |
| `scopes` | no | The scopes a token caller needs, as constants `tool_scopes!` declared |
| `input` | no | The argument type, usually a `#[tool_input]` struct. Without it, the tool takes no input |
| `output` | no | A `#[tool_output]` struct, whose JSON Schema is the tool's output schema |
| `annotations(...)` | no | Hints for clients, described below |
| `title` | no | A display name |
| `route` | no | The route name, when the handler serves more than one route |
| `guard` | no | The auth guard a user caller is signed in to for the call. It must use the default guard's provider; another provider is refused as a configuration error because an id in one provider does not name the same person in another. |
| `budget` | no | `budget = ignore` silences this tool's compile warning; boot still logs its budget warnings |

Use short action names such as `create_post` or `search_notes`. The build
warns when a name exceeds 30 characters or a description exceeds 500
characters. Boot logs the same budgets and warns about each input field
without a description. Boot warnings never refuse a tool. Give every input
field a doc comment; `#[tool_input]` carries it into the JSON Schema.

The compile warning is a Rust `deprecated` lint. A build with
`RUSTFLAGS="-D warnings"` fails on it. Keep the tool within the budget. If a
tool keeps its long name on purpose, set `budget = ignore` to silence that
tool's compile warning. Boot still logs its budget warnings. This key works
on handlers and Live methods and leaves other deprecation warnings in their
code visible:

```rust
#[tool(
    name = "search_archived_project_records",
    description = "Search archived project records.",
    callers = [user],
    budget = ignore,
)]
#[handler]
async fn search_records() -> Response {
    json_response!([])
}
```

Names, descriptions and schemas are fixed when the application is built. An
agent reads them as instructions, so nothing at run time can change them. The
build fails on an invalid name, a missing or empty description, a missing
`callers`, a `public` tool with scopes, an `input` type none of the handler's
arguments has, or two tools with one name in a crate. Boot fails when tools in
two crates share a name, when a tool names a scope no `tool_scopes!` declares,
when a path parameter of the tool's route is not a field of its input, or when
a tool's handler serves several routes and `route` names none of them. A
marked handler that no route serves is left out, with a warning at boot.

### Input and output schemas

`#[tool_input]` makes a struct a form request and derives its JSON Schema
(draft 2020-12) from the struct, its `serde` attributes and its `validator`
attributes. The schema an agent reads names the same fields, required flags
and bounds the server enforces:

```rust
use suprnova::tool_input;

/// A product review.
#[tool_input]
pub struct CreateReview {
    /// The review's text.
    #[validate(length(min = 1, max = 10000))]
    #[serde(rename = "content")]
    pub body: String,
    /// How many stars, 1 to 5.
    #[validate(range(min = 1, max = 5))]
    pub stars: u8,
    /// Whether to publish it now.
    #[serde(default)]
    pub publish: Option<bool>,
}
```

The handler takes the struct as an argument, as it takes any form request,
and the framework validates it before the handler runs. Over MCP, invalid
input comes back as a tool result with `isError: true` that names each field,
not as a protocol error, so the agent can correct the call. In the browser the
route answers its usual validation error.

A doc comment on a field becomes its description in the schema. A call fills
the route's path parameters from the input's fields of the same names.

#### Nested types

A field's type needs a JSON Schema too. A nested struct can be a
`#[tool_input]` of its own. An enum, or a struct that should not be a form
request, takes `#[tool_schema]`, which derives the schema through the
framework and leaves the type's serde derives to you:

```rust
use serde::Deserialize;
use suprnova::{tool_input, tool_schema};

/// How to order the reviews.
#[tool_schema]
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewOrder {
    /// Newest first.
    Newest,
    /// Highest rated first.
    TopRated,
}

/// The input of `list_reviews`.
#[tool_input]
pub struct ListReviews {
    /// The order to list the reviews in.
    pub order: ReviewOrder,
}
```

The enum's schema becomes part of the tool's input schema, so an agent sees the
values it may send, and a value outside them is refused as invalid input.

`output = T` publishes `T`'s schema as the tool's output schema. Mark `T`
with `#[tool_output]`, the output counterpart of `#[tool_input]`. It derives
`Serialize` and `JsonSchema` through the framework, so the application needs
no `schemars` dependency of its own. A type nested in the output takes
`#[tool_output]` or `#[tool_schema]`:

```rust
use suprnova::tool_output;

/// What `create_review` answers.
#[tool_output]
pub struct ReviewView {
    /// The review.
    pub id: u64,
    /// Its stars, 1 to 5.
    pub stars: u8,
}
```

`T` is any struct or enum, and a doc comment on a field becomes its
description. A JSON object answer reaches every client as structured data. A
list, a scalar or another non-object answer, and a non-object output schema,
are structured data for clients on revision 2026-07-28, which takes any JSON
value. Clients on 2025-06-18 and 2025-11-25 get such an answer as text, and
see the tool without its output schema, because those revisions take an
object only. Wrap a list in a struct field, such as `{ "items": [...] }`, when
those clients should get it as structured data too.

### Declare scopes

A scope is what a token lets its client do. `tool_scopes!` declares the
application's scopes, each with a description the approval page shows:

```rust
suprnova::tool_scopes! {
    /// The scopes a client can be granted.
    pub mod scope {
        POSTS_READ = "posts:read" => "Read your posts";
        POSTS_WRITE = "posts:write" => "Create and edit your posts";
        POSTS_DELETE = "posts:delete" => "Delete your posts";
    }
}
```

Each line makes a constant of type `suprnova::agents::Scope`. A tool names its
scopes with these constants, so a scope nobody declared does not compile. A
scope name is visible ASCII without spaces, quotes or backslashes, and its
description is not blank. `declared_scopes()` lists every scope the
application declared.

A token caller needs every scope the tool lists. A browser session holds
every scope, because the user is present and acting for themselves.

### Callers

`callers = [...]` lists who may call the tool:

- **`public`** accepts anyone, with no credential. A `public` tool accepts
  every other caller too, and it cannot declare scopes, since a guest would
  call it without a token.
- **`user`** accepts a signed-in user: by browser session, or by a token the
  user approved for an MCP client. A `user` tool never accepts an agent
  account, even one bound to a user.
- **`agent`** accepts an agent account that registered under auth.md.

An action for both people and agents lists `callers = [user, agent]`.

A handler reads the caller by taking a `ToolCaller` argument:

```rust
use suprnova::{Response, ToolCaller, handler, json_response, tool};

#[tool(
    name = "read_user_profile",
    description = "Read the signed-in user's profile.",
    callers = [user, agent],
)]
#[handler]
pub async fn profile(caller: ToolCaller) -> Response {
    json_response!({
        "kind": caller.kind().as_str(),
        "user": caller.user_id(),
        "browser": caller.via_browser(),
        "client": caller.client_id(),
    })
}
```

`ToolCaller` answers what the handler needs to know:

| Method | Answers |
|---|---|
| `kind()` | `CallerKind::Public`, `User` or `Agent` |
| `is_guest()` / `is_user()` / `is_agent()` | The same, one kind at a time |
| `channel()` | `CallerChannel::Browser` (the page, with the session), `Client` (the MCP endpoint) or `Direct` (the route, with a bearer, basic or custom guard) |
| `via_browser()` | Whether the call came from a page with the user's session |
| `acting_user()` / `user_id()` | The user the call acts for: the signed-in user, or the user an agent is bound to |
| `registration()` | The agent registration, for an agent caller |
| `client_id()` | The client or provider the call came through |
| `can(scope)` | Whether the caller holds a scope; a browser session holds every scope |
| `granted_scopes()` | The token's scopes, or `None` for a browser session |
| `address()` | The client address |

Outside a tool call, a `ToolCaller` argument answers 500. A handler that
serves both people and tools reads `ToolCaller::current()`, which is `None`
outside a tool call.

Policies read the caller the same way. This one lets users delete posts and
forbids agents to:

```rust
use suprnova::{Gate, ToolCaller};

Gate::define::<User, Post>("delete_post", |_user, _post| {
    !ToolCaller::current().is_some_and(|caller| caller.is_agent())
});
```

### Annotations

Annotations tell a client what a tool does, so it can ask the user before a
risky call. A bare key means `true`:

```rust
#[tool(
    name = "delete_post",
    description = "Delete one post.",
    callers = [user, agent],
    scopes = [scope::POSTS_DELETE],
    input = PostRef,
    annotations(destructive),
)]
```

| Key | MCP | WebMCP |
|---|---|---|
| `read_only` | `readOnlyHint` | `readOnlyHint` |
| `destructive` | `destructiveHint` | `consequentialHint` |
| `idempotent` | `idempotentHint` | not sent |
| `open_world` | `openWorldHint` | not sent |
| `untrusted_content` | not sent | `untrustedContentHint` |
| `debugging` | not sent | `debugging` |

`debugging` marks a tool meant for debugging and developer tooling rather than
for end users, such as one that dumps the page's state, so an agent acting for
a user can leave it aside. MCP's annotations have no such hint.

Over MCP only the hints you set are sent. A page sends WebMCP's four hints
always, `false` where you set nothing. After a tool that is not marked
`read_only`, the page refreshes its data, so mark tools that only read.
Annotations are hints. They never replace the route's policies.

### Budgets

Every tool call counts against a budget, in the cache store the rate limiter
uses. `AgentsConfig` sets three:

| Builder | Default | Counts |
|---|---|---|
| `caller_limit(CallLimit)` | 120 a minute | Calls by one user or one agent, through the browser and the MCP endpoint together |
| `address_limit(CallLimit)` | 60 a minute | `public` calls from one client address |
| `request_limit(CallLimit)` | 600 a minute | Every request to the MCP endpoint from one client address, including `tools/list` and refused tokens |

```rust
use std::time::Duration;
use suprnova::agents::{AgentsConfig, CallLimit};

let config = AgentsConfig::new()
    .caller_limit(CallLimit::per_minute(60))
    .address_limit(CallLimit::new(20, Duration::from_secs(60)));
```

A call over budget answers 429 with `Retry-After`. If the limiter's store
fails, the call answers 503 rather than run without a limit. An IPv6 client
is counted by its `/64`. The route's own throttle middleware applies as well.
A browser request counts against the budget only when it carries the
`Suprnova-Tool-Call` header, which a page's tool call sends and an ordinary
click does not.

### Tools on GET routes

A GET tool takes its input as any tool does, with a `#[tool_input]` struct. A
form request on GET or HEAD reads its query, as in Laravel, so the same
handler serves a browser link, a page's tool call and an MCP call:

```rust
/// What to look for.
#[tool_input]
pub struct NoteQuery {
    /// The text to find.
    pub q: String,
}

#[tool(
    name = "lookup_note",
    description = "Look a note up by its text.",
    callers = [public],
    input = NoteQuery,
    annotations(read_only),
)]
#[handler]
pub async fn lookup(form: NoteQuery) -> Response {
    json_response!({ "q": form.q })
}
```

A page's call of a GET tool sends every argument in the query, path arguments
included. An MCP call sends a JSON body, which the form request reads instead
of the query. Because a browser can send only strings, numbers and booleans in
a query, boot refuses a GET tool offered on a page whose input has an object
or a list field. Serve a tool with structured input on POST.

## The MCP endpoint

`Router::agents` serves the endpoint and the documents a client discovers it
by. Every setting has a default, so `AgentsConfig::new()` is a working
endpoint at `/mcp` that serves `public` tools:

```rust
use std::time::Duration;
use suprnova::agents::AgentsConfig;

let config = AgentsConfig::new()
    .allow_origin("https://admin.example.com")
    .call_timeout(Duration::from_secs(10))
    .instructions("Posts stay drafts until a person publishes them.");
```

| Builder | Default | What it sets |
|---|---|---|
| `path` | `/mcp` | The endpoint's path |
| `allow_origin` | none | Another origin a browser may call the endpoint from, besides the application's own (`APP_URL`) |
| `max_body_bytes` | 1 MiB | The largest request body; a larger one answers 413 before it is read |
| `max_result_bytes` | 8 MiB | The largest answer a tool's route may give; a larger one becomes a tool error |
| `call_timeout` | 30 seconds | How long one call may run; a longer one is stopped and answered with a tool error |
| `caller_limit`, `address_limit`, `request_limit` | see [Budgets](#budgets) | Call budgets |
| `resolver` | none | A token resolver, asked in the order added |
| `resolver_timeout` | 5 seconds | How long a resolver may take; a slower one answers 503 |
| `server_info` | `APP_NAME` and the framework's version | The name and version the endpoint reports |
| `instructions` | none | Guidance for agents, sent on discovery and `initialize` |
| `resource_name` | `APP_NAME` | The name in the protected resource metadata |
| `oauth_server` | off | The [OAuth server](#users-who-approve-a-client) |
| `agent_auth` | off | [Agent accounts](#agent-accounts) |
| `authorization_server` | none | Metadata of an authorization server you run elsewhere |

`Router::agents` registers these routes, each under the application's public
root, so an application served under a path prefix is discoverable without
owning the host root:

| Route | Serves |
|---|---|
| `POST /mcp` | The endpoint. `GET` and `DELETE` answer 405 |
| `GET /.well-known/oauth-protected-resource` and `GET /.well-known/oauth-protected-resource/mcp` | Protected resource metadata (RFC 9728) |
| `GET /.well-known/oauth-authorization-server` and `GET /.well-known/openid-configuration` | Authorization server metadata (RFC 8414), or 404 when no authorization server is configured |

It panics when one of these routes is already registered or when it is called
twice. `Router::try_agents` returns the error instead.

### Transport and protocol revisions

The endpoint uses MCP's Streamable HTTP transport, revision 2026-07-28: one
`POST` per JSON-RPC request, with `server/discover`, `tools/list`,
`tools/call` and `ping`. Clients of revisions 2025-06-18 and 2025-11-25, which
open with `initialize`, work on the same endpoint. The endpoint keeps no
session and never mints `Mcp-Session-Id`.

- An answer is JSON, or a stream of server-sent events when the client's
  `Accept` header prefers one. Closing the response cancels the call.
- The `MCP-Protocol-Version`, `Mcp-Method` and `Mcp-Name` headers must agree
  with the body, or the request answers 400 with error `-32020`.
- An unsupported protocol version answers error `-32022`, listing the
  supported ones.
- A request whose `Origin` header names an origin the application does not
  allow answers 403. This stops a web page from reaching the endpoint through
  the user's browser. Clients outside a browser send no `Origin`, and those
  requests are accepted.
- `tools/list` with a token returns exactly the tools that token may call.
  Without a token it returns every tool, including the ones that need a
  token, so a client sees what signing in would let it call. Both lists keep
  a fixed order.

### Tokens and discovery

A tool that does not accept `public` callers needs a bearer token. A call
without one answers 401 with `WWW-Authenticate: Bearer resource_metadata="..."`
and the tool's scopes, so a client finds the protected resource metadata and,
through it, the authorization server. Claude Code, for example, then reports
that the server needs you to sign in, and `/mcp` signs you in. Every other
request works without a token, `public` tools included. A token that lacks a
scope answers 403 with `error="insufficient_scope"` and every scope the call
needs.

The endpoint verifies no token itself. It asks each resolver on
`AgentsConfig` in order: the OAuth server's and agent auth's when they are
on, then yours. A token no resolver accepts is refused, and the endpoint never
forwards a token to the tool's route. The route sees the caller, not the
token.

A resolver of your own implements `CallerResolver`:

```rust
use async_trait::async_trait;
use suprnova::ToolCaller;
use suprnova::agents::{CallerResolver, ResolveContext, ResolveError};

struct ServiceTokens;

#[async_trait]
impl CallerResolver for ServiceTokens {
    async fn resolve(
        &self,
        token: &str,
        ctx: &ResolveContext<'_>,
    ) -> Result<ToolCaller, ResolveError> {
        // `lookup` is the application's own token store.
        let Some(grant) = lookup(token).await else {
            return Err(ResolveError::NotMine);
        };
        if grant.resource != ctx.resource {
            return Err(ResolveError::Invalid("the token was issued for another resource"));
        }
        Ok(ToolCaller::user(grant.user).scopes(grant.scopes).client(grant.client_id))
    }
}
```

`NotMine` passes the token to the next resolver. `Invalid` answers 401
`invalid_token` and asks no other resolver. `Unavailable` answers 503: a
caller is never served as a guest because a store was down. Register it with
`AgentsConfig::new().resolver(Arc::new(ServiceTokens))`.

## Callers

A tool accepts the callers its `callers` list names. This section covers how
each kind reaches it, and how a token becomes a caller.

### Public callers

A `public` tool needs no credential. Every caller can call it, and the
endpoint lists it for a client with no token. Its budget is per client
address. Use it for what any visitor of the site may already do, such as a
search.

### Users who approve a client

The OAuth server lets a user approve an MCP client to act as them. It is a
Magnetar plugin, an OAuth 2.1 authorization server built for MCP clients.
Turn it on with `oauth_server`:

```rust
use suprnova::agents::{AgentsConfig, OAuthServer};

pub fn register() -> Router {
    web::register().agents(AgentsConfig::new().oauth_server(OAuthServer::new()))
}
```

The client follows the MCP authorization flow:

1. The client reads the protected resource metadata, then the authorization
   server metadata.
2. It identifies itself with a client ID metadata document, an HTTPS URL that
   describes it, or registers dynamically (RFC 7591).
3. It sends the user to `/oauth/authorize`. A guest signs in first, at
   `/login?return_to=...`.
4. The approval page names the client, its host and every scope it asked
   for. The user approves or denies.
5. The client exchanges the code at `/oauth/token`, with PKCE.

What the server enforces:

- The authorization code grant with PKCE `S256`, and no other code challenge
  method.
- Redirect URIs compared exactly. On a plain HTTP loopback redirect,
  `127.0.0.1`, `[::1]` or `localhost`, any port is allowed, as native
  clients need: Claude Code registers `http://localhost/callback` and signs
  in on whatever port the system gives it.
- The `iss` parameter on every authorization response (RFC 9207).
- Access tokens bound to the MCP endpoint (RFC 8707) and to the approved
  scopes. The endpoint refuses a token issued for another resource.
- Refresh tokens rotated on every use. A refresh token presented a second
  time, at once or later, revokes every token of that grant.
- Revocation at `/oauth/revoke` (RFC 7009).
- Tokens and codes stored only as SHA-256 digests.
- A password change, or signing out other devices, ends the user's client
  grants as it ends their sessions.
- A client metadata document fetched over HTTPS only, never from a private
  or loopback address, with no redirects and a bounded size and time.

The server's issuer is the application's public URL, `APP_URL`, with its path
prefix. Outside the `local` and `testing` environments, the router refuses to
build when `APP_URL` is unset. A client checks the issuer of every response,
so it must be the URL clients reach the application at. An issuer that is not
`https` is refused unless its host is a loopback address.

| Builder | Default |
|---|---|
| `route_prefix` | `/oauth` |
| `sign_in(path, parameter)` | `/login` and `return_to` |
| `approval_page(path)` | The framework's approval page at `/oauth/approve` |
| `custom_approval_page(path)` | Your own approval page; the framework mounts none |
| `connected_agents_page(path)` | The framework's connected-agents page at `/oauth/connected` |
| `custom_connected_agents_page()` | No connected-agents page; you build one with `ConnectedAgents` |
| `access_token_lifetime` | One hour |
| `refresh_token_lifetime` | 30 days |
| `registration_limit` | 20 dynamic registrations an hour per client address |
| `client_metadata_fetcher` | Magnetar's fenced HTTPS fetch |
| `connection` | The application's database connection |

The scopes are the ones `tool_scopes!` declared. The server keeps its grants
in four tables. `OAuthServer::migrate(&db)` creates them and is safe to run on
every boot; the Magnetar default engine's migrations create them too.

Every node prunes those tables every five minutes while it serves. It deletes
pending authorizations and codes that lapsed, tokens that expired, and grants
that ended more than 30 days ago: revoked grants, and grants with no token
left and no use since. A live grant or token is never pruned. A spent refresh
token stays until it expires, so a second use of it is still caught.
`agents:prune` runs the same pass once, with or without agent auth.

The token, revocation and registration endpoints are called by clients, with
no cookie, so they get no session and need no CSRF token. The approval page
needs the signed-in user, takes its answer by a CSRF-protected form and
refuses to be framed.

#### Replace the pages

The default pages are plain server-rendered HTML with no script. To render
the approval page in your own front end, name it with
`custom_approval_page(path)` and build it on `AgentApprovals`, as [Consent
pages of your own](#consent-pages-of-your-own) shows.

#### Connected agents

`ConnectedAgents` lists and revokes what the signed-in user approved. Each
method takes the request it serves and acts only on that user's grants:

```rust
use suprnova::{ConnectedAgents, HttpResponse, Request, Response, handler};

#[handler]
pub async fn index(request: Request) -> Response {
    let grants = ConnectedAgents::client_grants(&request).await?;
    let agents = ConnectedAgents::agent_accounts(&request).await?;
    Ok(HttpResponse::json(serde_json::json!({ "grants": grants, "agents": agents })))
}
```

| Method | Does |
|---|---|
| `client_grants(&request)` | The user's live client grants, newest first, as `ClientGrant`: client name and host, scopes, when it was approved and last used |
| `revoke_client_grant(&request, grant_id)` | Revokes one grant and its tokens; `false` when the user has no such grant |
| `agent_accounts(&request)` | The agent registrations bound to the user, as `AgentAccount` |
| `revoke_agent_account(&request, registration_id)` | Revokes one registration and its tokens; `false` when the user has none by that id |

A guest gets 401. A router with no OAuth server answers 500, naming
`AgentsConfig::oauth_server`.

### Users from an outside issuer

An application that already runs an identity provider can accept that
provider's user tokens instead of running the OAuth server. `OutsideIssuer`
verifies them against the provider's published keys, for issuer, audience,
expiry and scope, and finds the user with the default guard's provider. It
serves the `user` caller kind only: an outside token is never an agent
account.

The tool's guard must use the default guard's provider; a guard on another
provider is refused as a configuration error because an id in one provider
does not name the same person in another.

```rust
use std::sync::Arc;
use suprnova::agents::{AgentsConfig, OutsideIssuer};

let issuer = OutsideIssuer::new(
    "https://login.example.com",
    "https://login.example.com/.well-known/jwks.json",
)?
.user_claim("email")
.match_user_on("email");
let config = AgentsConfig::new().resolver(Arc::new(issuer));
```

The token names its user in one claim. Choose the claim with care, because it
decides who the token signs in:

- **`sub`, the default.** The issuer assigns it, so no verified flag is read.
  It links one user across clients only where the issuer gives a user the same
  `sub` for every client application. Many issuers do not: each MCP client is
  its own client application, and a pairwise `sub` differs for each.
- **An issuer-assigned id, with `id_claim`.** A claim the issuer assigns and
  the user can neither choose nor change. `id_claim` refuses a claim a user
  can set, such as `email`, `preferred_username` or `upn`.
- **Any other claim, with `user_claim`.** The token must also carry the
  issuer's verified flag set to `true`: `<claim>_verified`, such as
  `email_verified`, or the flag `verified_by(..)` names. A missing, `false` or
  non-boolean flag refuses the token. There is no way to turn this check off:
  without it, an issuer that lets users set their own address would let anyone
  sign in here as `victim@example.com`.

Microsoft Entra ID gives each client application a different `sub` for the
same user. Match on `oid`, the user's object id, and name the tenant's own
issuer, which pins tokens to that tenant:

```rust
let issuer = OutsideIssuer::new(
    "https://login.microsoftonline.com/<tenant id>/v2.0",
    "https://login.microsoftonline.com/<tenant id>/discovery/v2.0/keys",
)?
.id_claim("oid")?
.match_user_on("entra_oid");
```

Without `match_user_on`, the claim's value is the user's id. With it, the
provider looks the user up by that field through `retrieve_by_credentials`.
`EloquentUserProvider` matches only the columns its `credential_columns`
lists, `email` by default, so list the field there, such as
`.credential_columns(["email", "entra_oid"])`.

| Builder | Default |
|---|---|
| `audience` | The MCP endpoint's URL |
| `user_claim`, `id_claim`, `match_user_on`, `verified_by` | `sub`, matched as the user's id |
| `leeway` | 60 seconds for `exp`, `nbf` and `iat` |
| `key_cache(max_age, refetch_after)` | Keys kept 10 minutes; an unknown key id refetches at most every 30 seconds |
| `key_fetch` | The fenced HTTPS fetch, which refuses private addresses |

The token's `scope` (space-separated) or `scp` (a list) gives the caller's
scopes, and its `client_id` or `azp` the client. Only asymmetric signing
algorithms are accepted.

### Agent accounts

An agent account is an agent that registered with the application itself,
under [auth.md v0.6.0](https://github.com/workos/auth.md), rather than one a
user signed in. Agent auth is a second Magnetar plugin. It issues its tokens
on the OAuth server's token endpoint, so it needs the OAuth server too:

```rust
use suprnova::agents::{AgentAuth, AgentsConfig, IdentityType, OAuthServer, TrustedIssuer};

pub fn register() -> Router {
    web::register().agents(
        AgentsConfig::new()
            .oauth_server(OAuthServer::new())
            .agent_auth(
                AgentAuth::new()
                    .enable(IdentityType::IdentityAssertion)
                    .trust(TrustedIssuer::new(
                        "https://agents.acme.example",
                        "https://agents.acme.example/.well-known/jwks.json",
                        "Acme",
                    ))
                    .pre_claim_scopes([scope::POSTS_READ])
                    .post_claim_scopes([scope::POSTS_READ, scope::POSTS_WRITE]),
            ),
    )
}
```

Without `oauth_server`, the router refuses to build.

#### How an agent registers

An agent reads `/auth.md`, which the framework writes from your
configuration: the endpoints, the registration types that are on, the scopes
and the trust list. It then registers at `POST /agent/identity` in one of
three ways:

- **`anonymous`.** No identity. The agent gets an identity assertion at once
  and holds the pre-claim scopes until a user claims it. On by default.
- **`service_auth`.** The agent names a user by email and waits for that user
  to claim it. It holds nothing until then. On by default.
- **`identity_assertion` (ID-JAG).** The agent presents an identity assertion
  from a provider on your trust list, such as an AI platform the user signed
  in to. Off until you call `.enable(IdentityType::IdentityAssertion)` and
  `.trust(...)`.

The agent exchanges its identity assertion for an access token at the OAuth
server's token endpoint. Agent tokens carry no refresh token. Each type can
be turned off, and the endpoint then answers with auth.md's
`*_not_enabled` code. To refuse anonymous registration:

```rust
let agent_auth = AgentAuth::new().disable(IdentityType::Anonymous);
```

An ID-JAG is accepted only from an issuer on the trust list. Its signature is
checked against the issuer's keys, its audience, expiry and issue time within
two minutes of the server's clock, its verified email or phone, and its
`auth_time` within `max_auth_age` (one hour by default). A reused `jti` is
refused on every node. A user is matched first by an earlier delegation from
that issuer; a match only by verified email or phone asks the user to confirm
through the claim ceremony. With no match, `provision_users` decides; without
it the ID-JAG is refused. The display name the confirmation shows comes from
your trust list, never from the provider.

#### The claim

A user claims an agent to make it theirs:

1. The agent asks to be claimed at `POST /agent/identity/claim` and hands the
   user a link to the claim page, `/agent/claim`, and a six-digit code.
2. The user signs in. The page accepts only the user with the email the claim
   names, and only once that email is verified. An application verifies an
   address with `EmailVerification`, or with `Auth::email_verification()`
   when its users live in Magnetar's tables; see
   [Email Verification](auth-flows.md#email-verification).
3. The page shows the agent and every scope it will hold. The user enters the
   code, which a CSRF-protected form posts.
4. The agent's next poll gets a token bound to the user.

A code lasts ten minutes and allows five wrong entries. An unclaimed
registration expires after 24 hours. Claiming an anonymous registration
revokes its pre-claim tokens. Claim tokens, attempt tokens and codes are
stored only as SHA-256 digests.

To render the claim page in your own front end, name it with
`.custom_claim_page(path)` and build it on `AgentClaims`, as [Consent pages of
your own](#consent-pages-of-your-own) shows.

#### What an agent can call

A tool accepts agents only when its `callers` list `agent`. Within those:

- **Before a claim**, an anonymous registration holds no scope, unless
  `pre_claim_scopes` names some.
- **After a claim**, a registration holds every scope the application
  declared, unless `post_claim_scopes` names fewer.

So an agent tool that declares no scope is open to every agent, claimed or
not. To require a claim, give the tool a scope only claimed agents hold, one
`post_claim_scopes` includes and `pre_claim_scopes` leaves out. Or check in
the handler: `registration()` returns the agent's registration, and
`is_claimed()` is `true` once it is bound to a user.

```rust
#[tool(
    name = "publish_post",
    description = "Publish a draft post.",
    callers = [user, agent],
    scopes = [scope::POSTS_WRITE],
    input = PostRef,
)]
#[handler]
pub async fn publish(caller: ToolCaller, form: PostRef) -> Response {
    let unclaimed = caller
        .registration()
        .is_some_and(|registration| !registration.is_claimed());
    if unclaimed {
        return Err(HttpResponse::json(serde_json::json!({
            "message": "Ask a person to claim this agent first."
        }))
        .status(403));
    }
    json_response!({ "published": form.post })
}
```

A narrowed `post_claim_scopes` applies at once, to registrations claimed
before the change as to new ones. A registration keeps the post-claim scopes
of the day it registered, so a scope you add later reaches only
registrations made after the change.

#### The claim event

A claim dispatches `AgentClaimed`, so the application can move what the agent
created to the user who claimed it:

```rust
use async_trait::async_trait;
use suprnova::{AgentClaimed, FrameworkError, Listener, attrs};

use crate::models::Post;

pub struct AdoptAgentPosts;

#[async_trait]
impl Listener<AgentClaimed> for AdoptAgentPosts {
    async fn handle(&self, event: &AgentClaimed) -> Result<(), FrameworkError> {
        let user_id: i64 = event
            .user_id
            .parse()
            .map_err(|_| FrameworkError::internal("a user id is a number"))?;
        // Running this again for the same claim changes nothing.
        Post::query()
            .filter("agent_registration_id", event.registration_id.as_str())
            .update_all(attrs! { user_id: user_id })
            .await?;
        Ok(())
    }
}
```

Register it in `bootstrap.rs` as any listener, with
`EventFacade::listen::<AgentClaimed, _>(Arc::new(AdoptAgentPosts)).await`. See
[Events](events.md).

The event is delivered at least once. The claim's transaction writes it to
an outbox, so a committed claim always has its event, and it is dispatched
right after the commit with a stable `event_id`:

- Make every listener idempotent, keyed on `event_id`: do the work once per
  id, or make doing it again change nothing.
- Tag every row an agent creates with its registration id, which a tool
  reads from `caller.registration()`, so the move is one query.
- A listener's error never undoes the claim. The maintenance pass retries
  the event. After 12 failed attempts the event is marked failed, logged once
  at error level and kept. Fix the listener, then run
  `agents:prune --retry-failed` to put it back.
- Every node runs the maintenance pass in `serve`, every five minutes.
  `event_delivery_timeout` bounds one delivery (60 seconds by default).

`AgentClaimed` carries `event_id`, `registration_id`, `registration_type`,
`user_id`, `provider` (the trust list's display name, for an ID-JAG) and
`client_address`.

#### Revocation

Revocation works at both layers auth.md names:

- **Tokens**, at the OAuth server's `/oauth/revoke` (RFC 7009). An unknown
  token answers 200.
- **Registrations**, by a Security Event Token (RFC 8417) a trusted provider
  pushes to `POST /agent/event/notify` (RFC 8935). It is verified against the
  provider's keys, audience and `jti`, revokes the registration and every
  token derived from it, and answers 202. The route exists only while ID-JAGs
  are on.

A user revokes their own registrations on the connected-agents page or
through `ConnectedAgents::revoke_agent_account`. A password change or signing
out other devices ends the user's bound agents too.

#### Keys and operations

Agent auth signs identity assertions with a P-256 key you configure, with
`.signing_key(kid, pem)` or the `AGENT_AUTH_KEY_ID` and
`AGENT_AUTH_PRIVATE_KEY` variables. A missing or unreadable key fails the
router build in every environment, because a key made up at boot would make
nodes and restarts disagree. A deploy with no `.env` file must set both
variables in the process environment. The `agents:key` command prints a new
pair, ready for `.env`, and writes nothing:

```bash
cargo run -- agents:key
```

- **Rotate** by moving the old key to `AGENT_AUTH_PREVIOUS_KEY_ID` and
  `AGENT_AUTH_PREVIOUS_PRIVATE_KEY`, with `AGENT_AUTH_PREVIOUS_ROTATED_AT` set
  to when it stopped signing (RFC 3339), and the new key in the main pair. The
  previous key keeps verifying what it signed until those assertions expire.
  `.previous_signing_key(kid, pem, rotated_out_at)` does the same in code.
- **After a leaked key**, configure a new key first and leave the leaked one
  out of the previous key. Then run `agents:revoke-all --force`. The command
  builds the router, which needs a key, and revokes every registration and
  every token the leaked key could have reached. In production it also asks
  you to type the environment's name at a terminal. Running it again finishes
  a run that failed partway.
- **`agents:prune`** runs the maintenance pass once. It prunes the OAuth
  server's rows that can no longer be used. With agent auth on, it also
  delivers waiting claim events, expires unclaimed registrations, prunes
  spent replay records and deletes claim events delivered more than seven
  days ago, and reports how many events failed. `--retry-failed` puts failed
  events back first, and needs agent auth.
- **Keep the nodes' clocks in sync.** Expiry, the two-minute skew on issue
  times and the delivery leases compare each node's clock.
- **The audit log holds email addresses.** Agent auth writes one structured
  record per auth.md audit event: registration created, assertion issued,
  token issued, token revoked, claim requested, user code minted, claim
  confirmed, registration expired and registration revoked. The claim request
  names the user's email. Send `audit_log_channel(name)` to a channel where
  personal data may be kept. The same events reach listeners as
  `MagnetarLifecycleEvent`, with their details in `attributes`.
- **Keep CORS closed.** The claim routes answer with the signed-in user's
  email. Do not open them, or any `/agent` route, to cross-origin requests
  with credentials in your CORS policy.
- **Store emails in lowercase.** Agent auth matches a user's email without
  case. `EloquentUserProvider` compares `LOWER(email)`, so on a large user
  table add an index on `LOWER(email)`, and store new addresses in lowercase.
  A provider of your own implements `retrieve_all_by_email_ignoring_case`,
  whose default matches exactly.

Registration is rate-limited per client address and per application:
5 and 100 an hour for `anonymous` and `service_auth`, 60 and 1000 for ID-JAGs.
`registration_limit(type, per_address, per_application)` changes them. Every
route agent auth adds bounds its body to 16 KiB. Its four tables come from
`AgentAuth::migrate(&db)`, or from the Magnetar default engine's migrations.

| Builder | Default |
|---|---|
| `enable` / `disable` | `anonymous` and `service_auth` on, `identity_assertion` off |
| `trust` | No trusted provider |
| `pre_claim_scopes` | None |
| `post_claim_scopes` | Every declared scope |
| `max_auth_age` | One hour |
| `registration_window` | 24 hours |
| `user_code_lifetime` | Ten minutes |
| `max_user_code_failures` | 5 |
| `assertion_lifetime` | 30 days |
| `provision_users` | None: an unmatched ID-JAG is refused |
| `user_directory` | The default guard's provider |
| `route_prefix` / `document_path` | `/agent` and `/auth.md` |
| `claim_page` / `custom_claim_page` | The framework's page at `/agent/claim` |
| `service_name` | `APP_NAME` |
| `audit_log_channel` | The default log channel |
| `key_set_cache(floor, ceiling)` | 10 minutes and 24 hours |
| `event_delivery_timeout` | 60 seconds |

### Consent pages of your own

The OAuth server and agent auth ship plain default pages. To render the
approval page and the claim page in your own front end, turn the defaults off
and build the handlers on the consent facade: `AgentApprovals` for the
approval page and `AgentClaims` for the claim page. The framework's default
pages use the same two types, and the starter kits' pages are built on them:
see [Agents in the Inertia kits](starter-kits.md#agents-in-the-inertia-kits)
and [Agents in the Live kit](starter-kits.md#agents-in-the-live-kit). The API
kit serves the framework's default pages behind a sign-in for consent only;
see [Agents in the API kit](starter-kits.md#agents-in-the-api-kit).

```rust
use suprnova::agents::{AgentAuth, AgentsConfig, OAuthServer};

let config = AgentsConfig::new()
    .oauth_server(OAuthServer::new().custom_approval_page("/oauth/approve"))
    .agent_auth(AgentAuth::new().custom_claim_page("/agent/claim"));
```

Both types act only for the user the default guard signed in. Another user's
authorization reads as ended, and another user's claim as someone else's.
`AgentApprovals::of(&request)` and `AgentClaims::of(&request)` refuse a tool
call with 403, whether the MCP endpoint dispatched it or a page sent it with
the `Suprnova-Tool-Call` header: consent comes from the person, never from an
agent acting as them. Take the handle with `of` before you read the request's
body.

#### The approval page

The authorization endpoint sends the user to your page with the pending
authorization's id in the `authorization` query parameter. `pending` reads
it, and `decide` records the user's answer:

```rust
use serde::Deserialize;
use suprnova::agents::{AgentApprovals, ApprovalDecision, ApprovalLookup};
use suprnova::{FrameworkError, Inertia, InertiaResponse, Redirect, Request, Response};

pub async fn show_approval(request: Request) -> Response {
    let approvals = AgentApprovals::of(&request)?;
    let id = request.query_param("authorization").unwrap_or_default();
    match approvals.pending(&id).await? {
        ApprovalLookup::Pending(approval) => Ok(InertiaResponse::new("Agents/Approve")
            .with("approval", approval)
            .resolve(&request)
            .await?),
        ApprovalLookup::Ended => Err(ended().into()),
        ApprovalLookup::SignIn => Redirect::guest(&request, "/login").into(),
    }
}

#[derive(Deserialize)]
pub struct DecisionForm {
    authorization: String,
    decision: String,
}

pub async fn decide(request: Request) -> Response {
    let approvals = AgentApprovals::of(&request)?;
    let form: DecisionForm = request.form().await?;
    match approvals.decide(&form.authorization, form.decision == "approve").await? {
        ApprovalDecision::Redirect(url) => Ok(Inertia::location(url)),
        ApprovalDecision::Ended => Err(ended().into()),
        ApprovalDecision::SignIn => Redirect::to("/login").into(),
    }
}

fn ended() -> FrameworkError {
    FrameworkError::domain("This request has ended. Start again from the app.", 404)
}
```

- `PendingApproval` holds the `id` to post the decision with, the
  `client_name`, the `client_host`, the `redirect_uri` the user returns to,
  every scope with its `name` and `description`, and `expires_at`. Show the
  client's name beside its host and every scope. The client chose its name and
  redirect URI, so isolate their direction (`<bdi>`). The type carries no
  logo or home page on purpose: loading the logo would tell the client who
  looked, and a link the client chose is a phishing aid on a consent screen.
- `decide(id, approve)` spends the authorization either way, so deciding
  again is `ApprovalDecision::Ended`. `Ended` also covers a request that
  lapsed or belongs to another user.
- `ApprovalDecision::Redirect` sends the browser to the client's redirect
  URI, usually on another origin. The security headers' `form-action 'self'`
  blocks a redirect there after a native form post. On an Inertia page,
  answer with `Inertia::location(url)`: the visit is a script request, so
  the browser leaves by `window.location`, which `form-action` does not
  govern. A page that posts a native form gets a plain redirect from
  `Inertia::location` too, which the browser blocks, so answer it with a
  short page of your own that navigates there: a link to the address and
  `<meta http-equiv="refresh" content="0; url=...">`, with the address
  escaped for an attribute.
- `SignIn` means no user is signed in, or the session predates the user's
  last password change or "sign out everywhere".

#### The claim page

The verification URI an agent hands the user opens your page with the
attempt in the `claim_attempt_token` query parameter. `pending` reads the
claim, and `confirm` takes the code the user types:

```rust
use serde::Deserialize;
use suprnova::agents::{AgentClaims, ClaimConfirmation, ClaimLookup, ClaimRefusal};
use suprnova::{FrameworkError, InertiaResponse, Redirect, Request, Response};

pub async fn show_claim(request: Request) -> Response {
    let claims = AgentClaims::of(&request)?;
    let token = request.query_param("claim_attempt_token").unwrap_or_default();
    match claims.pending(&token).await? {
        ClaimLookup::Pending(claim) => Ok(InertiaResponse::new("Agents/Claim")
            .with("claim", claim)
            .with("token", token)
            .resolve(&request)
            .await?),
        ClaimLookup::Refused(ClaimRefusal::SignIn) => Redirect::guest(&request, "/login").into(),
        ClaimLookup::Refused(refusal) => Err(refused(refusal).into()),
    }
}

#[derive(Deserialize)]
pub struct CodeForm {
    claim_attempt_token: String,
    user_code: String,
}

pub async fn confirm(request: Request) -> Response {
    let claims = AgentClaims::of(&request)?;
    let form: CodeForm = request.form().await?;
    match claims.confirm(&form.claim_attempt_token, &form.user_code).await? {
        ClaimConfirmation::Claimed { .. } => Redirect::to("/dashboard").into(),
        ClaimConfirmation::WrongCode { attempts_remaining } => Err(FrameworkError::validation(
            "user_code",
            format!("That code is wrong. You can try {attempts_remaining} more times."),
        )
        .into()),
        ClaimConfirmation::Refused(ClaimRefusal::SignIn) => Redirect::to("/login").into(),
        ClaimConfirmation::Refused(refusal) => Err(refused(refusal).into()),
    }
}

fn refused(refusal: ClaimRefusal) -> FrameworkError {
    match refusal {
        ClaimRefusal::OtherAccount => FrameworkError::domain(
            "Sign in with the email address the agent named to claim it.",
            403,
        ),
        ClaimRefusal::NotFound | ClaimRefusal::Ended => {
            FrameworkError::domain("This claim has ended. Ask the agent to start again.", 404)
        }
        ClaimRefusal::SignIn | ClaimRefusal::Invalid => {
            FrameworkError::domain("This claim cannot be read.", 400)
        }
    }
}
```

- `PendingClaim` holds the `registration_type`, the `provider` (the trust
  list's display name, never one the provider sent), the `signed_in_as`
  address, the `email` the agent named, every scope the agent will hold,
  `expires_in_seconds`, `attempts_remaining`, and two warnings:
  `first_time_account` when no agent has acted for this account before, and
  `first_time_provider` when none from this provider has. Show both
  warnings, and isolate every text but the scopes' words, since the user, the
  agent or the provider chose it.
- `confirm` counts a wrong code against the attempt and answers
  `ClaimConfirmation::WrongCode { attempts_remaining }`. When none remain, the
  attempt ends and the agent starts a new one.
- `ClaimConfirmation::Claimed { registration_id }` means the registration is
  the user's. The agent's next poll gets its tokens, and `AgentClaimed` is
  dispatched.
- `ClaimRefusal::OtherAccount` means the claim names another address, or the
  signed-in user's address before it was verified. Tell the user to sign in
  with the address the agent named, and show nothing of the claim.
  `NotFound` means no such attempt or one already confirmed, `Ended` an
  expired code or too many wrong ones, and `Invalid` a request the engine
  cannot read.

#### Route the pages

Run every consent route behind `ConsentPageHeaders`, and the routes that
record a decision behind the CSRF middleware, which the global CSRF
middleware already provides:

```rust
use suprnova::agents::ConsentPageHeaders;
use suprnova::{get, group, post, routes};

use crate::controllers::consent;

routes! {
    group!("/", {
        get!("/oauth/approve", consent::show_approval),
        post!("/oauth/approve", consent::decide),
        get!("/agent/claim", consent::show_claim),
        post!("/agent/claim", consent::confirm),
    }).middleware(ConsentPageHeaders),
}
```

`ConsentPageHeaders` holds for every answer of those routes, errors
included: a CSRF refusal, an error page rendered after the middleware chain
and a panic's answer.

- **No framing.** The policy's `frame-ancestors` becomes `'none'` and
  `X-Frame-Options` becomes `DENY`, so no page can frame "Allow" under a
  click. The rest of the policy, nonce sources included, stays as
  `SecurityHeaders` and your overrides made it, so the page's scripts run.
  Under report-only, relaxed or off, an enforced `frame-ancestors 'none'` is
  sent all the same.
- **No caching, no referrer.** `Cache-Control: no-store` and
  `Referrer-Policy: no-referrer` keep the authorization id and the attempt
  token in the page's address out of caches and other sites' logs.

It runs inside `SecurityHeaders` and route overrides, before every other
middleware. A header your handler sets gives way to these. Without
`SecurityHeaders` installed, it sets all four headers on the answers that
pass through it, but an answer the server makes after the chain, such as a
panic's, carries only what the server set.

#### Pages that fetch JSON

A front end that fetches its data can use the plugins' JSON routes instead of
handlers of its own. They act for the signed-in user only, and the routes
that record a decision need the CSRF token.

- **Approval.** Read the pending authorization from
  `GET /oauth/authorizations/{id}`, and post the decision to
  `POST /oauth/authorizations/{id}` as the form field `decision` (`approve` or
  `deny`).
- **Claim.** Read the pending claim from
  `GET /agent/identity/claim/attempt?claim_attempt_token=...`, and post
  `claim_attempt_token` and `user_code` to
  `POST /agent/identity/claim/complete`.

The page that hosts them still runs `ConsentPageHeaders`, sends a guest to
sign in and back to itself, and shows the client, its host and every scope,
or the provider and every scope.

## Tools in the browser

WebMCP lets an agent in the browser call tools a page registers with
`document.modelContext`. The call runs in the user's tab, with the user's
session, and the page shows what the agent did. Suprnova registers three
kinds of browser tool:

- **Server tools**: `#[tool]` handlers the server lists for a page. A call
  is a same-origin request to the tool's route with the session cookie, the
  CSRF token and the `Suprnova-Tool-Call` header.
- **Live tools**: actions of a mounted Live component.
- **Front-end tools**: tools a React, Vue, Svelte or WebAssembly component
  defines and registers while it is mounted.

A browser registers a tool only on a page that is allowed to use WebMCP. See
[Security Headers](security-headers.md#webmcp-and-permissions-policy) for the
`tools` permission and the Origin Trial.

### Page tools

The server decides which server tools each page offers. Name them on the
router, a route, a group or a response:

```rust
use suprnova::{InertiaResponse, PageTools, Request, Response, Router};

/// `Application::routes(routes::register)` calls this once at boot.
pub fn register() -> Router {
    web::register()
        // Every page offers the search tool.
        .share_tools(["search_posts"])
}

// In the route list, a route adds its own:
// get!("/posts", posts::index).with_tools(["create_post"]).name("posts.index"),

pub async fn show(request: Request) -> Response {
    // A handler adds tools for this request only.
    PageTools::add(["update_post"]).await?;
    Ok(InertiaResponse::new("Posts/Show")
        .with_tools(["delete_post"])
        .resolve(&request)
        .await?)
}
```

| Where | How |
|---|---|
| Every page | `Router::share_tools([...])` |
| A route, a `get!` or `post!` route, or a `group!` | `.with_tools([...])` |
| An Inertia response | `InertiaResponse::with_tools([...])` |
| A Live document | `document.with_tools([...]).await`, before `bootstrap` |
| The current request, from any handler | `PageTools::add([...]).await` |

Each name must be a routed `#[tool]` that accepts `public` or `user` callers.
A name the router, a route or a group offers that is not one stops the server
at boot; one a handler or a response adds fails that page's render. Either
way the error names the tool. An agent-only tool is never offered in the
browser, because a browser viewer is never an agent account, and a Live tool
is offered by its own island. A page offers at most 64 tools.

Each page lists only the tools its viewer may call: a guest sees `public`
tools, and a user signed in by session sees `user` and `public` tools. An
Inertia page carries the list in its page object's `agentTools` field, on
every visit and partial reload. A Live page carries it in a
`<script type="application/json" id="suprnova-agent-tools">` element its
document's bootstrap writes. A plain view puts `PageTools::element(&request)`
in its `<head>`.

The server tools on a page leave when the user navigates away. After a call
of a tool that is not marked `read_only`, the page refreshes its data so the
screen matches what the agent did: an Inertia page reloads its props, and a
Live page renders every island again.

A request that carries the `Suprnova-Tool-Call` header names a tool. On a
route that serves no tool, or a different one, it answers 400. Only a request
with the header counts against the tool budget; the caller checks apply to
every request to a tool's route.

#### Page tools and RenderCache

A page whose tools all accept `public` callers is the same for every viewer.
The server reads no viewer to build its list, so a cached copy stays shared.
A page with a `user` tool reads who the viewer is. On a route whose cache
policy varies by principal, RenderCache stores that page once per signed-in
user, and guests share one copy. On a route with no principal variance it
declines to store the page. So keep busy public pages to public-only tools.
See [RenderCache](render-cache.md).

### Live actions as tools

Mark a Live action `#[tool]` to offer it to agents. The action's arguments
give the input schema, and fields marked `#[agent_visible]` are what the
agent reads back:

```rust
use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "app.counter", view = "live/counter.html")]
pub struct Counter {
    #[agent_visible]
    count: u64,
    #[agent_visible]
    label: String,
}

#[live]
impl Counter {
    #[mount]
    pub fn mount(start: u64, label: String) -> Self {
        Self { count: start, label }
    }

    #[tool(
        name = "add_to_counter",
        description = "Add to the counter.",
        callers = [user, agent],
    )]
    #[action]
    pub fn add(&mut self, amount: u64) {
        self.count += amount;
    }

    #[tool(
        name = "peek_counter",
        description = "Read the counter.",
        callers = [public],
        annotations(read_only),
    )]
    #[action]
    pub fn peek(&mut self) {}
}
```

A Live tool takes `name`, `description`, `title`, `callers`, `scopes`,
`annotations`, `guard`, `arguments(...)` and `headless`. Its arguments come from the action's
parameters, so it takes no `input`. Each argument must have a JSON Schema an
agent can read: `String`, `bool`, `i64`, `u64`, `f64`, `Date`,
`OffsetDateTime` or `Uuid`, or an `Option`, `Vec` or `BTreeMap` of them.

Describe action parameters with `arguments(...)` on the tool marking:

```rust,ignore
#[tool(
    name = "add_to_tally",
    description = "Add to the current tally.",
    callers = [user],
    arguments(amount = "How much to add to the tally."),
    headless,
)]
#[action]
pub fn add(&mut self, amount: u64) {
    self.count += amount;
}
```

An argument without a description raises a compiler warning and a boot
warning. A description of an argument the action does not take is a build
error. For an agent-supplied mount property, put its description on the
parameter as `#[doc = "The initial count."]`, for example
`fn mount(#[doc = "The initial count."] start: u64) -> Self`.

In the browser, the Live runtime registers a mounted component's tool actions
with `document.modelContext` and removes them when the component leaves the
page or the user navigates away. A call runs the ordinary action path: the
signed snapshot, the CSRF check, the action's authorization and its
transaction. An action marked `#[action(authorize = "current")]` asks the
Gate for `live:{component}.{action}`, as a click does. The result carries the
agent-visible fields and no other state.

The browser names a Live tool after its island: `peek_counter-main` for the
island `live:key="main"`. The key must be one literal of 1 to 63 ASCII
letters, digits, `_` and `-`, outside any loop, so data never reaches a name
an agent reads as an instruction. `suprnova live:check` refuses any other key
on a component with browser tools, and a key used twice on one page. A Live
tool name, the `-` separator and the key must fit in 64 characters in all.
The checker names the island, tool and combined length when they do not.
Tool names contain no hyphens, so the first hyphen separates the name from
its key, including a key such as `left-pane`.

Marking an action `#[tool]` narrows who can click it to the callers you list.
The caller check runs on every request to the action, with or without the
tool-call header, so `callers = [user, agent]` refuses a guest's click with
401. An action for both people and agents lists `callers = [user, agent]`,
and adds `public` for guests. An action no person should click lists only
`agent` and is never registered in the browser.

### The Inertia kits

The React, Vue and Svelte kits type-check their WebMCP registration through
`webmcp-types` 0.1.10. The package defines the model context, tool definition,
execution options and annotations. It leaves the result payload open, so
Suprnova keeps its `ToolResult` type for structured content and errors.

The React, Vue and Svelte kits ship helpers in `src/lib/agents`, and their
notes page uses them; see
[Agents in the Inertia kits](starter-kits.md#agents-in-the-inertia-kits).
Each kit's entry calls `installPageTools(router)` before `createInertiaApp`. It
registers the server tools in the page object's `agentTools`, swaps them on
every visit, and reloads the page's props after a write tool.

A component registers a tool for as long as it is mounted with
`usePageTool`:

```tsx
import { usePageTool } from '../lib/agents'

export default function Cart({ items }: { items: string[] }) {
  usePageTool({
    name: 'list_cart_items',
    description: 'List the items in the visible cart.',
    inputSchema: { type: 'object', properties: {} },
    readOnly: true,
    execute: () => ({ items }),
  })
  return <ul>{items.map((item) => <li key={item}>{item}</li>)}</ul>
}
```

`readOnly`, `untrustedContent`, `consequential` and `debugging` set the
tool's four WebMCP hints, as the `#[tool]` annotations above do. `useFormTool`
takes `destructive` and `debugging`.

`useFormTool` makes a kit form a tool. The input schema comes from the
form's server request type: `suprnova generate-types` writes every form
request's schema to `frontend/src/types/tool-schemas.ts`, keyed by the
struct's name, and `suprnova serve` keeps it current. A call fills the form,
which the user sees, and submits it through Inertia. Validation errors come
back as a tool result with `isError: true` that names each field:

```tsx
import { useForm } from '@inertiajs/react'
import { useFormTool } from '../lib/agents'
import { toolSchemas } from '../types/tool-schemas'

export default function NewNote() {
  const note = useForm({ title: '', body: '' })
  useFormTool(note, {
    name: 'create_note',
    description: 'Write a note.',
    schema: toolSchemas.StoreNoteRequest,
    action: { method: 'post', url: '/notes' },
  })
  // ...the form itself
}
```

A form for a destructive action, marked `destructive: true` or sent with
`DELETE`, is filled and left for the user to submit. A form tool sends no
tool-call header: it drives the page's own form, so the route's throttle and
checks apply rather than the tool budget. Its submission carries
`Suprnova-Form-Tool: 1` instead (`suprnova::agents::FORM_TOOL_HEADER`), and
the consent facade refuses a request that carries it, so a form tool on an
approval or claim page can never approve or claim for the user. Vue and
Svelte take the same options; their `useFormTool` takes the `useForm` object.

`generate-types` refuses, by name, a request type it cannot describe exactly
as the server does, rather than write an approximate schema.

### WebAssembly front ends

The `suprnova-webmcp` crate brings the same tools to a Rust front end
compiled to WebAssembly. `PageTool` registers a typed tool whose input
schema is derived as the server derives a `#[tool]`'s, and `ServerTools`
registers and calls the server tools the page lists:

```rust
use suprnova_webmcp::{PageTool, ServerTools, ToolHandle, ToolResult, WebMcpError, tool_input};

/// An item to add to the visible todo list.
#[tool_input]
struct AddTodo {
    /// What to do.
    #[validate(length(min = 1, max = 200))]
    title: String,
}

/// Register the page's tools. They stay registered while the caller keeps
/// the handles this returns.
fn boot() -> Result<(ToolHandle, Vec<ToolHandle>), WebMcpError> {
    let add = PageTool::<AddTodo>::new("add_todo", "Add an item to the visible todo list.")
        .title("Add todo")
        .register(|input: AddTodo| async move { Ok(ToolResult::json(&input.title)?) })?;
    let server = ServerTools::from_document()?;
    let tools = server.register_all(|| async { /* refetch the app's data */ })?;
    Ok((add, tools))
}
```

A registration lasts as long as its `ToolHandle`; dropping the handle removes
the tool. `ServerTools::from_document` reads the
`suprnova-agent-tools` element, so the page that boots the front end puts
`PageTools::element(&request)` in its `<head>`. A server tool call carries
the session, the CSRF token and the tool-call header, and its validation
errors name each field as the server's do. The crate's `#[tool_input]`
brings its own copies of `serde`, `schemars` and `validator`, so the front
end needs none of them. An enum or another type nested in the input takes
`#[suprnova_webmcp::tool_schema]`, as on the server. `PageTool` sets the
WebMCP hints with `read_only`, `untrusted_content`, `consequential` and
`debugging`.

Every function compiles on every target, so a crate shared with the server
builds natively too. Outside the browser, registering or calling a tool
returns `Unsupported::Target`. In a browser without WebMCP, registering
returns `Unsupported::NoModelContext`, while reading and calling server tools
still work.

## Headless Live tools

A Live tool action marked `headless` can also be called over the MCP
endpoint, with no browser. An action without the marking is never offered
there.

```rust
// src/live/tally.rs
use suprnova::live::{LiveComponent, live};
use suprnova::{FrameworkError, ToolCaller};

#[derive(LiveComponent)]
#[live(name = "app.tally", view = "live/tally.html")]
pub struct Tally {
    #[agent_visible]
    count: u64,
    #[locked]
    owner: String,
}

#[live]
impl Tally {
    #[mount]
    pub fn mount(start: u64, #[resolve(crate::live::tally::owner_of)] owner: String) -> Self {
        Self { count: start, owner }
    }

    #[tool(
        name = "add_to_tally",
        description = "Add to the tally.",
        callers = [user, agent],
        headless,
    )]
    #[action]
    pub fn add(&mut self, amount: u64) {
        self.count += amount;
    }
}

/// The owner is the caller, never a value the agent sends.
pub async fn owner_of(caller: &ToolCaller) -> Result<String, FrameworkError> {
    caller
        .user_id()
        .ok_or_else(|| FrameworkError::internal("a headless tally needs a user"))
}
```

The tool's input has three parts:

- **`mount`**: the component's mount properties, to start a new component.
  Each is declared by its `#[mount]` parameter, appears in the schema and is
  validated like an action argument. An `Option` parameter may be left out.
- **`handle`**: the handle an earlier call returned, to act on the same
  component. Send `mount` or `handle`, never both.
- **`arguments`**: the action's arguments.

Mount properties an agent sends are untrusted input. Before anything loads,
the framework asks the Gate ability `live:{component}.mount` with the caller
and the values, and refuses the mount when no policy defines it:

```rust
use suprnova::{Gate, LiveMountRequest, ToolCaller};

Gate::define::<ToolCaller, LiveMountRequest>("live:app.tally.mount", |caller, mount| {
    caller.is_user()
        && mount
            .property("start")
            .and_then(|value| value.as_u64())
            .is_some_and(|start| start < 100)
});
```

Each property reaches the policy in its codec's one JSON form, whatever form
the agent wrote. Write the policy so a property it cannot read refuses the
mount. A property filled by `#[resolve(...)]` is left out of the schema: the
agent cannot set it, and the resolver fills it from the caller after the
policy allows the mount.

The action then runs through the same authorization and transaction as in
the browser. The call returns the outcome, the agent-visible fields, a new
`handle` and its `expiresAt`. The handle is Live's signed snapshot, sealed so
the agent sees none of the state it carries, and bound to the caller: a
handle issued to one caller is refused for any other. It expires as a
browser snapshot does, after `LIVE_LEDGER_INSTANCE_LIFETIME_MS`. A stale
handle answers 409, and the agent calls again with the latest one or without
one.

Headless outcomes are `accepted`, `no_render`, `redirect`, and `invalid`.
If the sealed successor would make even the smallest follow-up request exceed
the endpoint's `max_body_bytes`, the outcome is `too_large` with `isError: true`.
The result keeps the agent-visible `fields` and any `validation` issues, but
returns no `handle`. Its message explains that the component's state no longer
fits one request and the mount must be started again.

A headless tool's component cannot have `#[session]` fields, and an action
that finalizes an upload cannot be headless.

## Testing

`TestClient` calls tools through both surfaces, as any caller, without
running the OAuth or auth.md ceremonies. The test path runs every check the
real path does: middleware, validation, policies, scopes, budgets and the
`Origin` check.

`TestClient::tool` calls a tool from the browser, as a page does: the session,
the CSRF token and the tool-call header. Sign a user in with `acting_as`:

```rust
use std::sync::Arc;
use serde_json::json;
use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::container::testing::TestContainer;
use suprnova::testing::TestClient;
use suprnova::MiddlewareRegistry;

#[tokio::test]
async fn a_signed_in_user_creates_a_post_from_the_page() {
    // Tool budgets count in the cache store.
    let _container = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    let client = TestClient::new(routes::register(), MiddlewareRegistry::new());
    // `user` is a user the test created.
    let call = client
        .acting_as(&user)
        .tool("create_post", json!({ "title": "From a page" }))
        .send()
        .await;
    call.assert_ok();
}
```

`TestClient::mcp` calls the MCP endpoint. `as_caller` chooses who calls, with
the scopes the token would carry:

```rust
use suprnova::ToolCaller;

#[tokio::test]
async fn an_agent_needs_the_write_scope() {
    let _container = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    let client = TestClient::new(routes::register(), MiddlewareRegistry::new());

    let refused = client
        .mcp()
        .as_caller(ToolCaller::agent("reg-1").claimed(true))
        .call("create_post", json!({ "title": "No scope" }))
        .await;
    assert_eq!(refused.status(), 403);

    let created = client
        .mcp()
        .as_caller(ToolCaller::agent("reg-1").claimed(true).scopes([scope::POSTS_WRITE]))
        .call("create_post", json!({ "title": "Scoped" }))
        .await;
    assert!(!created.is_error());
}
```

| Helper | Does |
|---|---|
| `client.tool(name, arguments)` | A browser tool call, as a guest or the `acting_as` user |
| `client.live_tool(name, page, arguments)` | A Live tool call on the island that offers it in `page`, the HTML of a page the client rendered |
| `client.mcp()` | An MCP client, a guest speaking revision 2026-07-28 |
| `.as_caller(ToolCaller)` | Call as a user or an agent, with chosen scopes, claimed or not |
| `.protocol_version(...)`, `.header(...)`, `.with_token(...)` | Speak an older revision, send a header, or present a real token |
| `.discover()`, `.initialize()`, `.list()`, `.call(name, arguments)` | The MCP methods, answered as `McpResponse` and `McpToolResult` |
| `response.page_tools()`, `page_tool(name)` | The tools a rendered page offers |
| `Agents::fake().token_for(caller)` | A token that stands for a caller, for a raw request |

`ToolCaller::user(user)`, `ToolCaller::agent(id)` and `ToolCaller::guest()`
build callers; `.scopes([...])`, `.claimed(bool)`, `.client(id)` and
`.bound_to(user)` shape them. Over MCP, only the token's verification is
faked. [Mocking](mocking.md) covers `TestContainer`.

### Try it in a browser

For local work, open `chrome://flags/#enable-webmcp-testing` in Chrome,
set it to **Enabled**, and relaunch. See [Chrome's WebMCP guide](https://developer.chrome.com/docs/ai/webmcp#local-webmcp).
WebMCP needs a [secure context](https://webmachinelearning.github.io/webmcp/#idl-index):
use HTTPS or [localhost](https://w3c.github.io/webappsec-secure-contexts/#is-origin-trustworthy).

In DevTools, open **Application > WebMCP**. The
[WebMCP pane](https://developer.chrome.com/docs/devtools/application/webmcp)
lists available tools and their calls, including inputs, outputs and errors.
Select a tool, enter its arguments and click **Run tool** to test it directly.

The [example agentic extension](https://github.com/GoogleChromeLabs/webmcp-extension)
needs Chrome 155 or later; its manifest sets the minimum to 155.0.8052.0.
It always asks before calling a tool marked `consequentialHint: true`.
By default, it also asks before tools without `readOnlyHint: true`. Users can
turn those alerts off or allow a site's actions for the chat. Those choices
never bypass a consequential tool's prompt.
The extension cuts results at 8,000 characters and appends a warning. Keep
answers small enough for the client that reads them. Suprnova's browser tools
carry JSON in `structuredContent` only, without repeating it as text.

The [Model Context Tool Inspector](https://github.com/beaufortfrancois/model-context-tool-inspector),
listed in [webmcp-tools](https://github.com/GoogleChromeLabs/webmcp-tools),
inspects exposed tools and their input schemas and helps debug connections.

For an agent using [chrome-devtools-mcp](https://developer.chrome.com/docs/devtools/application/webmcp#command-line-flags),
add `--categoryExperimentalWebmcp` to its server arguments. This enables
`list_webmcp_tools` and `execute_webmcp_tool`, which are off by default.

### Why Suprnova diverges

Laravel serves MCP through the `laravel/mcp` package: a tool is a class
registered on a server class, with its own schema, validation and
authorization, and Passport or Sanctum authenticates clients. Suprnova marks
the routed handler itself, so a tool call runs the route's middleware,
validation and policies, and a tool cannot drift from the page it backs. The
schema is derived from the input type at build time, never written beside it.
Suprnova offers tools only, not MCP prompts or resources.

Laravel has no first-party WebMCP or auth.md support. Suprnova's browser
tools let an agent work in the user's own tab, and its agent accounts let an
agent register before any person signs it in. The OAuth server is built for
MCP clients only: the authorization code grant with PKCE and client metadata
documents, without Passport's password, client credentials and implicit
grants or its personal access tokens.

## Next

- [Authorization](authorization.md) - policies a tool's route already runs
- [Validation](validation.md) - the rules `#[tool_input]` publishes
- [Live](live.md) - components, actions and snapshots
- [Starter Kits](starter-kits.md#agents-in-the-inertia-kits) - what each kit
  wires for agents, its consent pages and its notes tools
- [Security Headers](security-headers.md) - the `tools` permission and the Origin Trial
- [Environment Variables](env-vars.md#agents) - the signing key and trial token
