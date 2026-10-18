# Agents

Status: Agreed 2026-10-18
Prefix: AGENT

Drafted 2026-10-18 from next-feature item c7fb7505 `webmcp-agent-tools`
and the developer's directions of that morning. WebMCP and a real MCP
server, "Not just in the browser. `claude mcp add <server-name> --transport
http <server-url> --scope user`", reading the WebMCP draft of 16 October
2026 (https://webmachinelearning.github.io/webmcp/ and its repository). Our
own code throughout: "I think we should roll our own suprnova webmcp backend
helper" and "I don't want to use a third party sdk". Browser helpers for
"react, vue, svelte and wasm front ends", and for Live: "we will need to
devise a way for this to work with Live - even if that means
adding/modifying functionality". Three ways an agent is authenticated:
"none required (public tools), user authenticated (shared auth), agent auth
(agent account). We should account for all", with auth.md as the standard
for agent accounts ("This is the standard I want to follow for agent auth
https://github.com/workos/auth.md"). Additions to Magnetar are plugins: "If
we need to add to magnetar this will be done as a plugin". The protocols
are pinned: WebMCP as drafted on 2026-10-16, MCP revision 2026-07-28 with
the initialize-based revisions 2025-06-18 and 2025-11-25, auth.md v0.6.0.
The developer confirmed the requirements on 2026-10-18 at 10:59
("confirmed"), after AGENT-013 was tightened on a side agent's note so that
mount properties an agent sends are untrusted input.

## Observed at cfe6b8a13

Status: Observed

Read on 2026-10-18 with the code map of that day.

- No MCP, JSON-RPC or WebMCP code exists. `modelContext` and `webmcp` occur
  nowhere in the framework, the Live browser runtime or the kit templates;
  the only `jsonrpc-2.0` text is WebSocket subprotocol negotiation
  (`framework/src/ws/mod.rs:415-450`). The parity table says "No
  first-party MCP server adapter | by design no" (`manual/parity.md:242`).
- Magnetar is an OAuth client, not an authorization server: its OAuth
  engine runs outbound ceremonies with PKCE and RFC 7009 revocation
  (`crates/suprnova-magnetar/src/oauth/mod.rs:1-25`), and the parity table
  says "Passport (OAuth server) | Magnetar is an OAuth client, not an
  authorization server | not built" (`manual/parity.md:161`). No
  authorization endpoint, token endpoint for third parties, client
  registration, authorization server metadata or protected resource
  metadata exists.
- Magnetar composes features from plugins behind a public `Plugin` trait
  with routes, boot validation, a before-request hook, request handling and
  lifecycle hooks (`crates/suprnova-magnetar/src/plugin/registry.rs:44`).
  The RFC 8628 device sign-in is one such plugin, mounting six routes under
  `/oauth/device` behind the `device-authorization` feature, which the
  framework does not enable
  (`crates/suprnova-magnetar/src/plugins/device_authorization.rs:22-40`;
  `crates/suprnova-magnetar/Cargo.toml:31`).
- A bearer token is a Magnetar session, opaque or JWT, verified by hash
  (`crates/suprnova-magnetar/src/sessions/mod.rs:37-42`,
  `crates/suprnova-magnetar/src/sessions/opaque.rs:145-157`). Tokens carry
  no scopes. `BearerTokenMiddleware` records the user and never rejects
  (`framework/src/magnetar_integration/middleware.rs:10-29`), and
  `AuthMiddleware` answers a guest with a JSON 401 and no `WWW-Authenticate`
  header (`framework/src/auth/middleware.rs:249-254`).
- The router cannot list its routes: its tables and names are private and
  `HandlerRecord` holds argument type names only
  (`framework/src/routing/router.rs:726-812`,
  `framework/src/routing/binding.rs:78-83,325-342`). No JSON Schema is
  generated anywhere. Typed request structs validate through the
  `validator` crate's attributes (`framework/src/http/form_request.rs:1-45`).
- A Live action declares its arguments as fields with a codec and a
  required flag (`crates/suprnova-live/src/action/arguments.rs:15-23,57-64`);
  every codec except `Json` has a plain JSON Schema shape
  (`crates/suprnova-live/src/state/codec.rs:104-129`). An action runs only
  on a component hydrated from a signed snapshot sent by the browser, and
  the action endpoint admits a request only with origin, CSRF, session,
  principal and tenant facts (`crates/suprnova-live/src/protocol/request.rs:17-32`,
  `crates/suprnova-live/src/action/dispatch.rs:165-169`,
  `framework/src/live/attestation.rs:46-56`). A snapshot is bound to a scope
  fingerprint (`crates/suprnova-live/src/snapshot/codec.rs:74-111`).
- The security-header presets do not list the `tools` permission, so
  WebMCP runs on an application's own pages under them
  (`framework/src/security_headers/mod.rs:18-21`). The CSRF middleware
  has no exemption for bearer-token requests
  (`framework/src/csrf/middleware.rs:156-158,276-336`).
- The three Inertia kits share one root template whose head carries the
  CSRF and nonce meta tags
  (`suprnova-cli/src/templates/files/backend/templates/app.html.tpl:1-15`);
  the Live kit's head emits the Live bootstrap
  (`suprnova-cli/src/templates/files/live/templates/layouts/head.html.tpl:4`).

## Requirements

[AGENT-001] An application MUST expose a capability to agents by marking a
handler `#[tool]` with a description, an input struct whose JSON Schema
(draft 2020-12) the framework derives from the type, its serde attributes
and its `validator` attributes, and an output type whose schema becomes the
tool's output schema. A tool name MUST be 1 to 128 characters of ASCII
letters, digits, `_`, `-` and `.`, unique in the application; a duplicate
or invalid name, or a missing description, MUST fail the build. Names,
descriptions and schemas MUST be fixed at build time, never taken from
data, because an agent reads them as instructions. A tool call MUST run
through the same middleware, validation, `#[authorize]` policies and rate
limits as the handler's route, whichever surface it arrives on. Nothing is
a tool unless marked.
Falsifier: a marked handler is not offered as a tool; an unmarked handler, route or Live action is offered; the published schema omits a field, a required flag or a `validator` bound of the input struct; an invalid or duplicate name builds; a name, description or schema can change at run time; or a tool call skips a middleware, validation rule or policy that the same handler's route applies.
Mechanism: `agents`.
Rationale: schemars 1.x derives the schema and reads `validator` attributes, so what the agent is told matches what the server enforces; it is a schema generator, not an agent SDK.
Status: Agreed 2026-10-18

[AGENT-002] A tool MUST declare which callers it accepts: `public` (no
authentication), `user` (a signed-in user, by browser session or by a
token the user approved for a client) and `agent` (an agent account
registered under auth.md, AGENT-007), and the scopes each call needs. A
tool that accepts `user` MUST NOT accept an agent registration, even one
bound to a user, unless it also accepts `agent`. The application declares
its scopes, each with a description the approval page shows, and a tool
naming an undeclared scope MUST fail the build. A browser session holds
every scope. The handler MUST receive the caller: guest, user, or agent registration with
the user it is bound to, if any, and the client or provider it came
through. Gate policies MUST be able to read the caller, so an application
can allow agents to read and forbid them to delete. Every tool call MUST
be rate-limited per caller, and a public call per client address.
Falsifier: a tool runs for a caller kind it did not accept or without a scope it declared; a `user` tool accepts an agent registration bound to a user; a tool names an undeclared scope and builds; the handler cannot tell a browser user from a client acting for that user or from an agent account; a policy cannot read the caller; or one caller can call a tool without limit.
Mechanism: `agents`.
Rationale: the developer, 2026-10-18 10:43: "none required (public tools), user authenticated (shared auth), agent auth (agent account). We should account for all."
Status: Agreed 2026-10-18

[AGENT-003] The framework MUST serve an MCP endpoint, `/mcp` unless the
application names another path, over the Streamable HTTP transport of
revision 2026-07-28: one POST per JSON-RPC request, the
`MCP-Protocol-Version`, `Mcp-Method` and `Mcp-Name` headers checked
against the body with 400 and error -32020 on a mismatch, `server/discover`,
`tools/list` in a fixed order and `tools/call`, a JSON or request-scoped
SSE answer, and the close of a response stream treated as cancellation.
The same endpoint MUST serve clients of revisions 2025-06-18 and
2025-11-25, which open with `initialize`, without sessions: it MUST NOT
mint `Mcp-Session-Id`, and it MUST answer GET and DELETE with 405. It MUST
answer 403 when an `Origin` header names an origin the application does
not allow, MUST bound the request body and the call's run time, and MUST
answer an unsupported protocol version with error -32022 listing the
supported ones. Invalid tool input MUST come back as a tool result with
`isError: true` naming each field, not as a protocol error.
Falsifier: a 2026-07-28 client or an initialize-based 2025-06-18 or 2025-11-25 client cannot list and call the gate fixture's tools; a header that disagrees with the body is accepted; the endpoint mints a session or answers GET with a stream; a request from a foreign `Origin` runs; an oversized body or an overlong call is not cut off; or invalid input yields a JSON-RPC error.
Mechanism: `agents`.
Status: Agreed 2026-10-18

[AGENT-004] `tools/list` MUST return exactly the tools the presented
credentials may call: the public tools with no token, and with a token the
tools that accept its caller kind and whose scopes it carries. A request without a needed
token MUST get 401 with `WWW-Authenticate: Bearer resource_metadata="..."`
and the tool's scopes; a token without a needed scope MUST get 403 with
`error="insufficient_scope"` and every scope the operation needs. The
framework MUST serve OAuth protected resource metadata (RFC 9728) at
`/.well-known/oauth-protected-resource` and its path-inserted form for the
MCP endpoint, and authorization server metadata (RFC 8414) at the
path-inserted well-known location, at `<issuer>/.well-known/oauth-authorization-server`
and at `<issuer>/.well-known/openid-configuration`, so an application
served under a path prefix (PFX) is discoverable without owning the host
root.
Falsifier: a token-less list shows a non-public tool, a token sees a tool that does not accept its caller kind, or a scoped token sees a tool outside its scopes; a 401 lacks the metadata URL or the scopes; a 403 for a missing scope lacks `insufficient_scope` or names scopes one at a time; or a client cannot find the metadata of an application served under a prefix.
Mechanism: `agents`.
Status: Agreed 2026-10-18

[AGENT-005] A Magnetar plugin, the authorization server, MUST let a user
approve an MCP client to act as them under OAuth 2.1: the authorization
code grant with PKCE S256 and no other code challenge method, an approval
page naming the client, the client's host and the scopes, client
identification by Client ID Metadata Document and by dynamic client
registration (RFC 7591), redirect URIs compared exactly with any port
allowed on a loopback address, the `iss` parameter in every authorization
response (RFC 9207), access tokens bound to the resource the client named
(RFC 8707) and to the approved scopes, refresh tokens rotated on every use,
and revocation (RFC 7009). Tokens MUST be stored only as hashes. A
refresh token presented a second time, concurrently or later, MUST revoke
every token of that grant. A change of the user's authentication epoch
(a password change, signing out other devices) MUST end the user's client
grants as it ends their sessions. The approval page MUST require the user
to be signed in, take its answer by a CSRF-protected form and refuse to be
framed. The MCP endpoint MUST refuse a token issued for another resource
and MUST NOT accept or forward any token it did not issue, except through
AGENT-006. Fetching a client metadata document MUST accept HTTPS only,
refuse private and loopback addresses, follow no redirect, and bound its
size and time.
Falsifier: an MCP client following the 2026-07-28 authorization flow, identified by a client metadata document or by dynamic registration, cannot sign in and call a user tool; a code exchange without PKCE or with `plain` succeeds; the approval page omits the client's host or a scope, answers without a signed-in user or the CSRF token, or can be framed; a redirect URI that differs from the registered one is honoured; a refresh token works twice, including two concurrent uses, or its reuse leaves the grant's other tokens working; a token survives the user's password change; a token is stored in plain text; a token minted for another resource is accepted; or a client metadata URL on a private address is fetched.
Mechanism: `agents`.
Rationale: the developer, 2026-10-18 10:50: "If we need to add to magnetar this will be done as a plugin". Magnetar is an OAuth client today and the parity table lists Passport as not built; this plugin is that authorization server.
Status: Agreed 2026-10-18

[AGENT-006] Instead of the authorization server plugin, an application
MUST be able to accept user tokens from an outside authorization server
it names, verified against that server's published keys for issuer,
audience, expiry and scope, with the user resolved by a configured claim.
This driver serves the `user` caller kind only.
Falsifier: an application cannot accept tokens from a named outside issuer; a token with the wrong issuer, audience or an expired time is accepted; or an outside token is accepted as an agent account.
Mechanism: `agents`.
Rationale: design principle 3: sign-in for MCP clients is not locked to one backend.
Status: Agreed 2026-10-18

[AGENT-007] A second Magnetar plugin, agent auth, MUST implement auth.md
v0.6.0 for agent accounts: `POST /agent/identity` with the `anonymous`,
`service_auth` and `identity_assertion` (ID-JAG) types, each returning a
service-signed identity assertion (`typ: oauth-id-jag+jwt`, `sub` the
registration) or the claim ceremony; the
`urn:ietf:params:oauth:grant-type:jwt-bearer` and
`urn:workos:agent-auth:grant-type:claim` grants on the authorization
server's token endpoint, with no refresh token; the `agent_auth` block in
the authorization server metadata; and the protocol's request bodies,
responses and error codes. Identity assertions MUST be signed with a key
the application configures; after a key rotation the previous key MUST
still verify assertions it signed until they expire. An application MUST be able to turn each type
off, which the endpoint reports with the protocol's `*_not_enabled` code.
An anonymous registration MUST get the application's pre-claim scopes
until it is claimed.
Falsifier: an agent following the auth.md v0.6.0 document cannot register by any enabled type and call an agent tool with the resulting token; a response or error body departs from v0.6.0; the token endpoint issues a refresh token for an agent grant; a disabled type is accepted; or an unclaimed anonymous registration gets more than the pre-claim scopes.
Mechanism: `agents`.
Rationale: the developer, 2026-10-18 10:43: "This is the standard I want to follow for agent auth https://github.com/workos/auth.md".
Status: Agreed 2026-10-18

[AGENT-008] Agent auth MUST accept an ID-JAG only from an issuer on the
application's trust list, where each entry names the issuer, its key set
URL and a display name the application controls. It MUST verify the
signature against the issuer's keys, fetched over HTTPS with a bounded size
and time and cached between a floor and a ceiling with one refetch on an
unknown key id; MUST check audience, expiry, issue time within a two-minute
skew, a verified email or phone, and `auth_time` within the configured
maximum age (one hour by default), answering a stale or missing
`auth_time` with 401 `login_required`; and MUST refuse a reused `jti`
through a database record that holds across every node. A user MUST be
matched first by an existing delegation for the issuer and subject; a
match only by verified email or phone MUST NOT bind silently but answer
401 `interaction_required` with the claim ceremony; with no match the
application's provisioning policy decides.
Falsifier: an ID-JAG from an issuer off the list, with a bad signature, the wrong audience, an expired time, no verified email or phone, or a stale `auth_time` is accepted; a `jti` replayed against a second node is accepted; an email-only match binds without the ceremony; or the confirmation page shows a display name the provider supplied.
Mechanism: `agents`.
Status: Agreed 2026-10-18

[AGENT-009] The claim ceremony MUST follow auth.md v0.6.0: a claim token,
a claim attempt token and a six-digit user code from a cryptographic
random source, stored only as SHA256 digests and returned once; a user
code valid for ten minutes with a bounded number of wrong entries per
attempt; a registration window of 24 hours; a claim page that requires
the user to be signed in, refuses a user other than the email the
registration is bound to, and accepts the code by a CSRF-protected form;
re-initiation with the same claim token; polling with
`authorization_pending`, `slow_down` and `expired_token`. Claiming an
anonymous registration MUST revoke its pre-claim tokens, return a second
identity assertion carrying the user's email, and dispatch an event naming
the registration and the user, so the application can move what the agent
created to that user.
Falsifier: a token or code is stored in plain text or returned twice; a code works after ten minutes or after the wrong-entry limit; a user signed in as another email completes the claim; the form accepts a post without the CSRF token; a pre-claim token still works after the claim; or no event fires on the claim.
Mechanism: `agents`.
Status: Agreed 2026-10-18

[AGENT-010] Revocation MUST work at both layers auth.md names: RFC 7009
at the token endpoint's revocation endpoint, answering 200 for an unknown
token; and Security Event Tokens (RFC 8417) pushed to the events endpoint
(RFC 8935), verified against the issuer's keys, audience and `jti`, which
revoke the registration and every token derived from it and answer 202.
A user MUST be able to see and revoke every client grant and agent
registration bound to them, and an operator MUST be able to revoke every
agent credential of the application in one command.
Falsifier: a revoked token still calls a tool; a revocation request for an unknown token answers other than 200; a valid revocation event leaves a derived token working or a forged or replayed one is accepted; a user cannot revoke a grant or registration bound to them; or the operator command leaves an agent credential working.
Mechanism: `agents`.
Status: Agreed 2026-10-18

[AGENT-011] Agent auth MUST serve `/auth.md`, the instructions an agent
reads, generated from the application's configuration: its endpoints,
enabled identity types, scopes and trust list. It MUST rate-limit
registration per address and per application, with defaults of 5 an hour
and 100 an hour for anonymous and 60 and 1000 for ID-JAG, and record the
auth.md audit events with the client address: registration created,
assertion issued, token issued, token revoked, claim requested, user code
minted, claim confirmed, registration expired and registration revoked.
The endpoints that clients call (MCP, token, revocation, registration,
claim initiation and events) MUST NOT require the CSRF token, and every
endpoint the two plugins add MUST bound its request body.
Falsifier: `/auth.md` names an endpoint, type or scope the application does not serve; registration beyond a limit succeeds; one of the listed events is not recorded; a client endpoint refuses a request for lacking a CSRF token; or an endpoint the plugins add reads an unbounded body.
Mechanism: `agents`.
Status: Agreed 2026-10-18

[AGENT-012] Live actions MUST be able to be tools: `#[tool]` on an action
with a description, its arguments' codecs giving the input schema, and
component fields marked agent-visible. In the browser the Live runtime MUST
register a mounted component's tool actions with `document.modelContext`
and remove them when the component leaves the page or the user navigates
away, run a call through the ordinary action path (snapshot, CSRF,
authorization `live:{component}.{action}`, transaction), and return the
action's result with the agent-visible fields and no other state. A
component whose actions are tools MUST carry a distinct `live:key` where it
appears more than once on a page, which the checker enforces, and the key
MUST be part of the tool name.
Falsifier: a mounted component's tool action is not registered or stays registered after the component leaves; a call skips the snapshot, CSRF, authorization or transaction an ordinary action has; the result carries a field not marked agent-visible; or two unkeyed instances of a tool component pass the checker.
Mechanism: `agents-browser`.
Status: Agreed 2026-10-18

[AGENT-013] A Live tool action marked `headless` MUST also be callable
over the MCP endpoint with no browser. Its input schema has three parts:
the component's mount properties, an optional state handle and the
action's arguments. Without a handle the call mounts the component on the
server as the MCP caller from the mount properties alone; with one it
hydrates the component from it. Mount properties sent by an agent are
untrusted input: each MUST be declared with its codec, appear in the
schema and be validated like an action argument, and before anything is
loaded the framework MUST check the Gate ability `live:{component}.mount`
for the caller with those values, refusing when no policy defines it. A
component MAY instead take a property from a resolver the application
supplies, given the caller, which the agent cannot set. The action runs
through the same authorization and transaction path as in the browser, and the call
returns the result, the agent-visible fields and a new state handle,
Live's signed snapshot, which a later call passes to act on the same
component. A handle's snapshot scope MUST be derived from the MCP caller,
so a handle issued to one caller is refused for any other, and a handle
MUST expire as a browser snapshot does. An action not marked `headless`
MUST NOT be offered over the MCP endpoint.
Falsifier: an MCP client cannot call a `headless` Live tool action without a browser; an undeclared mount property reaches the component; a headless mount runs without the `live:{component}.mount` check or with no policy defined; an agent sets a resolver-supplied property; a headless call skips the authorization or transaction an ordinary action has; a second call with the handle does not see the first call's state; a handle issued to one caller works for another or after its expiry; or an action without the marking is offered over the MCP endpoint.
Mechanism: `agents`.
Rationale: the developer, 2026-10-18 10:51: "we will need to devise a way for this to work with Live - even if that means adding/modifying functionality". MCP has no session and recommends an explicit handle for stateful tools; Live's snapshot is already a signed state handle. Opt-in, because a component mounted with no request has only its mount properties, and the developer is the one who knows that is enough.
Status: Agreed 2026-10-18

[AGENT-014] In the browser, the server MUST decide which server tools each
page offers: the tools the application lists as global plus those the
page's route or response adds, never one that does not accept the viewer's
caller kind. The Live
runtime and each kit MUST register them with `document.modelContext` when
the page opens and remove them when it closes, with WebMCP annotations from
the tool's markings (`readOnlyHint`, `untrustedContentHint`,
`consequentialHint`). A call MUST go to the application with the user's
session and CSRF token, and after a tool not marked read-only, the page
MUST refresh its data (an Inertia partial reload, a Live re-render of the
mounted components) so the screen matches what the agent did. With
`WEBMCP_ORIGIN_TRIAL_TOKEN` set, every HTML response MUST carry the
`Origin-Trial` header.
Falsifier: a page offers a tool that does not accept the viewer's caller kind or keeps a previous page's tools after navigation; a browser call runs without the session or the CSRF token; the page shows stale data after a write tool; an annotation does not match the tool's marking; or the trial token is set and an HTML response lacks the header.
Mechanism: `agents-browser`.
Rationale: the WebMCP explainer names the backend call that leaves the visible page stale as the problem the API exists to solve, and recommends registering only the tools relevant to the current page.
Status: Agreed 2026-10-18

[AGENT-015] The React, Vue and Svelte kits MUST each provide, in their own
frontend idiom, a way for a component to register a page tool for as long
as it is mounted, and a way to make a kit form a tool: the input schema
comes from the form's server request type through `suprnova
generate-types`, the call fills the form and submits it through Inertia,
and validation errors return as a tool result with `isError: true` naming
each field. A form tool for a destructive action MUST fill the form and
leave the submit to the user.
Falsifier: a kit cannot register a component tool, or the tool outlives its component; a form tool's schema disagrees with the server request type; a form tool submits outside Inertia or returns validation errors as a rejection; or a destructive form tool submits without the user.
Mechanism: `agents-browser`.
Status: Agreed 2026-10-18

[AGENT-016] A Suprnova crate for Rust front ends compiled to WebAssembly
MUST register typed page tools with `document.modelContext`, with the input
schema derived as in AGENT-001 and the registration removed when its handle
is dropped, and MUST call the application's server tools as AGENT-014
does. Built for a target without a browser it MUST compile and return an
unsupported error at registration.
Falsifier: a WebAssembly front end cannot register a typed tool; a dropped handle leaves the tool registered; a server tool call from it lacks the session or CSRF token; or a native build fails to compile.
Mechanism: `agents-browser`.
Rationale: the developer, 2026-10-18 10:26: "I think react, vue, svelte and wasm front ends".
Status: Agreed 2026-10-18

[AGENT-017] Every kit MUST wire the MCP endpoint, the two Magnetar plugins
and the browser helpers, and ship, in its own frontend, the approval page,
the claim page and a connected-agents page listing and revoking every client
grant and agent registration bound to the signed-in user. The framework
MUST provide plain default pages an application without a kit can use or
replace, and the API kit MUST expose the connected-agents list and
revocation as JSON routes.
Falsifier: a new project from any kit lacks the endpoint, a plugin, a helper or one of the three pages; an application without a kit has no approval or claim page; or the API kit offers no way to list or revoke grants.
Mechanism: `agents`.
Status: Agreed 2026-10-18

[AGENT-018] The testing surface MUST let a test call a tool through the
MCP endpoint and through the browser endpoint by `handle_request` as each
caller kind, a guest, a user with chosen scopes and an agent registration
claimed or not, without running the OAuth or auth.md ceremonies.
Falsifier: a test cannot call a tool as one of the caller kinds without driving a ceremony, or the test path skips a check the real path applies.
Mechanism: `agents`.
Status: Agreed 2026-10-18

[AGENT-019] `manual/agents.md` MUST explain tools, the MCP endpoint, the
three caller kinds with the authorization server and agent auth plugins,
WebMCP in the browser for Live, the three Inertia kits and WebAssembly
front ends, headless Live tools and testing, linked from
`manual/documentation.md`. The parity rows for the MCP server and Passport,
the security-headers chapter's Permissions-Policy section, the environment
variable table, the starter kits chapter and the release's upgrade chapter
MUST say what changed.
Falsifier: the chapter is missing a listed part or is not linked; a parity row still says the MCP server is "by design no" or Passport "not built"; or an environment variable this feature reads has no row.
Mechanism: `agents`.
Status: Agreed 2026-10-18
