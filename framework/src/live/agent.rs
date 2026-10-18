//! Headless Live tool calls: a Live action an agent runs over the MCP
//! endpoint with no browser (AGENT-013).
//!
//! The MCP endpoint dispatches a call of a headless Live tool as a request
//! to `POST /__live/agent`, the way it dispatches every tool: through the
//! router, the application's `LiveRouteGuard` middleware and the Live
//! completion middleware, with the caller's user signed in on the tool's
//! guard. This handler then runs the action through the code a browser's
//! click runs:
//!
//! 1. Without a handle, it checks the mount properties and the arguments
//!    field by field, asks the Gate `live:{component}.mount` for the caller
//!    before anything is loaded, fills the properties the application
//!    resolves itself, and mounts the component on the server, which signs
//!    and records its snapshot as a document mount does.
//! 2. With a handle, it opens the sealed snapshot the previous call
//!    returned. The seal is bound to the caller's Live scope, so another
//!    caller's handle does not open.
//! 3. Either way it builds a Live update request invoking the action on that
//!    snapshot and serves it through the browser endpoint's own service:
//!    request parsing, snapshot verification against the caller's scope and
//!    expiry, the ledger claim, argument preparation, the
//!    `live:{component}.{action}` authorization, validation, the
//!    transaction port and response sealing.
//! 4. It answers the outcome, the agent-visible fields of the successor
//!    snapshot and a new sealed handle, never the HTML, events, effects,
//!    flash or URL intents a browser applies.
//!
//! # What stands in for each check a browser request passes
//!
//! | Browser check | Headless call |
//! |---|---|
//! | POST, Live media type | The route is POST only; the handler builds the request and its media type itself. |
//! | Request size | The MCP endpoint's body cap, then the Live request limit on the body here. |
//! | Proxy normalization | The same: the dispatched request carries the client's forwarded headers and address. |
//! | Session | None: agent traffic carries no cookie. Closed as `StatelessRequest` for a marked call only; the catalog entry requires the session absent; the caller scope binds identity instead. |
//! | Origin (`Sec-Fetch-Site`) | The MCP endpoint's `Origin` check and the call mark nothing off the wire can carry. Closed as `TrustedInternalOrigin` for a marked call only. |
//! | CSRF | A bearer token is no ambient credential and the call forwards no cookie. Closed as `StatelessCsrfPolicy` for a marked call only. |
//! | Principal | The guard's `AuthMiddleware`, if any, runs on the signed-in acting user; with none, closed as `AnonymousPrincipal` for a marked call only. |
//! | Tenant | The guard's tenant middleware runs unchanged; its fact feeds the caller scope. Closed as `TenantlessRoute` only when none ran. |
//! | Rate limit | The endpoint's per-caller tool budget, plus the guard's throttle. Closed as `UpstreamRateLimited` only when the guard records none. |
//! | Middleware completion, attestation order and expiry | The same Live completion middleware and context validation. |
//! | Mount selection | Must be the agent route, the tool's own component and its slot. |
//! | Absence closure by mount kind | Not used: the agent catalog entry is refused on the browser endpoint. |
//! | Context, snapshot, document key, ledger, arguments, authorization, validation, transaction, sealing | The browser endpoint's service, unchanged, with the caller scope as the context scope. |
//! | Session-state commit and flash | No session: components with session fields are refused at boot, and flash is dropped. |

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use serde_json::{Map, Value, json};
use suprnova_live::canonical::{CanonicalValue, parse_canonical_value};
use suprnova_live::endpoint::{
    LIVE_MEDIA_TYPE_V2, LiveEndpointRequest, ParsedLiveMediaType, RequestCachePolicy,
};
use suprnova_live::host::{
    MountCatalogEntry, MountScopeRequirements, MountSelection, PolicyReason, ScopeRequirement,
};
use suprnova_live::identity::{BuildId, IslandSlot, ScopeFingerprint};
use suprnova_live::limits::InputLimits;
use suprnova_live::mount::{DocumentMountKey, DocumentMountScope, MountFlags, PrivateMountRequest};
use suprnova_live::protocol::DOCUMENT_KEY_EXTENSION_V1;
use suprnova_live::registry::ComponentDescriptor;
use suprnova_live::snapshot::{ComponentContract as SnapshotContract, ExpectedSeedV1};
use suprnova_live::state::FieldIssue;

use super::attestation::SecurityCheck;
use super::runtime::LiveRuntime;
use crate::agents::{LiveMountRequest, LiveTarget, ToolCaller};
use crate::crypto::{Crypt, CryptPurpose};
use crate::{FrameworkError, HttpResponse, Request, Response, Router};

/// The media type of a headless Live tool's answer: the MCP endpoint turns
/// a body of this type into the tool result as it stands, `isError` and
/// structured content together, so a validation outcome keeps the new
/// handle it consumed a revision for.
pub(crate) const RESULT_MEDIA_TYPE: &str = "application/vnd.suprnova.live-tool-result+json";

/// The document key of every component mounted for headless calls: the
/// one key its catalog entry admits.
const AGENT_DOCUMENT_KEY: &str = "agent";

/// The handler of `POST /__live/agent`.
pub(crate) async fn handle(request: Request) -> Response {
    Ok(Box::pin(serve(request)).await)
}

async fn serve(mut request: Request) -> HttpResponse {
    // Only a call the MCP endpoint dispatched for a headless Live tool
    // reaches anything below. The mark lives in a private field of the
    // request no header or cookie can set, so a request off the wire is
    // answered as if the route did not exist.
    let Some(target) = marked_target(&request) else {
        return refuse(404, "Not Found");
    };
    let Some(caller) = ToolCaller::current() else {
        return refuse(404, "Not Found");
    };
    close_marked_absences(&mut request);
    let runtime = match LiveRuntime::bind() {
        Ok(runtime) => runtime,
        Err(error) => return failed(&error),
    };
    let max_request_bytes = runtime.config().max_request_bytes();
    let request = match request.buffer_body(max_request_bytes).await {
        Ok(request) => request,
        Err(error) if error.status_code() == 413 => {
            return refuse(413, "The tool call is larger than Live accepts.");
        }
        Err(error) => return failed(&error),
    };
    let body = request.cached_body().cloned().unwrap_or_default();
    let input = match AgentInput::parse(&body) {
        Ok(input) => input,
        Err(errors) => return invalid(errors),
    };
    let Some(descriptor) = runtime.component_descriptor(&target.component) else {
        return failed(&FrameworkError::internal(format!(
            "the Live component `{}` of the tool `{}` is not registered",
            target.component.as_str(),
            target.tool
        )));
    };
    let Some(action) = descriptor
        .metadata()
        .actions()
        .iter()
        .find(|action| action.name().as_str() == target.action)
    else {
        return failed(&FrameworkError::internal(format!(
            "the Live component `{}` does not register the action of the tool `{}`",
            target.component.as_str(),
            target.tool
        )));
    };
    let limits = InputLimits::default();
    let mut errors = Map::new();
    collect(
        &mut errors,
        "arguments",
        action.arguments().field_issues(&input.arguments, &limits),
    );
    let scope = match super::context::agent_scope(&request, &caller) {
        Ok(scope) => scope,
        Err(error) => return failed(&error),
    };
    let fresh = input.handle.is_none();
    let envelope = match (input.mount, input.handle) {
        (Some(_), Some(_)) => {
            errors.insert(
                "handle".to_owned(),
                json!(["cannot be sent with mount: send a handle to act on a component, or mount to start one"]),
            );
            return invalid(errors);
        }
        (None, Some(handle)) => {
            if !errors.is_empty() {
                return invalid(errors);
            }
            match unseal(&handle, &scope) {
                Some(envelope) => envelope,
                None => return refuse(409, INVALID_HANDLE),
            }
        }
        (mount, None) => {
            let mount = mount.unwrap_or_else(|| CanonicalValue::Object(BTreeMap::new()));
            let resolved = target.resolved_properties();
            collect(
                &mut errors,
                "mount",
                descriptor
                    .parameter_schema()
                    .field_issues(&mount, &resolved, &limits),
            );
            if !errors.is_empty() {
                return invalid(errors);
            }
            if let Err(response) = authorize_mount(&target, &caller, &mount).await {
                return response;
            }
            match mount_component(
                &runtime,
                &request,
                &descriptor,
                &target,
                &caller,
                scope.clone(),
                mount,
            )
            .await
            {
                Ok(envelope) => envelope,
                Err(response) => return response,
            }
        }
    };
    run_action(
        &runtime,
        &request,
        &descriptor,
        &target,
        scope,
        envelope,
        input.arguments,
        fresh,
    )
    .await
}

/// The headless Live tool a request's call mark names, if any.
fn marked_target(request: &Request) -> Option<Arc<LiveTarget>> {
    request
        .agent_mark()
        .and_then(|mark| mark.as_call())
        .and_then(|call| call.live.clone())
        .filter(|target| target.headless && target.tool == call_name(request))
}

/// The tool a request's call mark names.
fn call_name(request: &Request) -> &'static str {
    request
        .agent_mark()
        .and_then(|mark| mark.as_call())
        .map_or("", |call| call.tool)
}

/// Close the checks a browser request proves and a dispatched call stands
/// in for, each only when no middleware recorded it: the origin, the CSRF
/// proof and the session, which agent traffic never has, and the principal,
/// tenant and rate limit when the application's guard records none. The
/// context validator still checks every closure against the catalog entry,
/// which requires the session absent.
fn close_marked_absences(request: &mut Request) {
    for (check, reason) in [
        (SecurityCheck::Session, PolicyReason::StatelessRequest),
        (SecurityCheck::Origin, PolicyReason::TrustedInternalOrigin),
        (SecurityCheck::Csrf, PolicyReason::StatelessCsrfPolicy),
        (SecurityCheck::Principal, PolicyReason::AnonymousPrincipal),
        (SecurityCheck::Tenant, PolicyReason::TenantlessRoute),
        (SecurityCheck::RateLimit, PolicyReason::UpstreamRateLimited),
    ] {
        if request
            .live_security_attestation()
            .disposition(check)
            .is_none()
        {
            request.record_live_security_not_required(check, reason);
        }
    }
}

/// What an agent sends a headless Live tool.
struct AgentInput {
    mount: Option<CanonicalValue>,
    handle: Option<String>,
    arguments: CanonicalValue,
}

impl AgentInput {
    /// The input in `body`, or the fields whose shape is wrong.
    fn parse(body: &[u8]) -> Result<Self, Map<String, Value>> {
        let limits = InputLimits::default();
        let mut errors = Map::new();
        let value = match parse_canonical_value(body, &limits) {
            Ok(CanonicalValue::Object(value)) => value,
            _ => {
                errors.insert("input".to_owned(), json!(["must be an object"]));
                return Err(errors);
            }
        };
        let mut input = Self {
            mount: None,
            handle: None,
            arguments: CanonicalValue::Object(BTreeMap::new()),
        };
        for (name, value) in value {
            match (name.as_str(), value) {
                ("mount", CanonicalValue::Null) | ("handle", CanonicalValue::Null) => {}
                ("mount", value @ CanonicalValue::Object(_)) => input.mount = Some(value),
                ("handle", CanonicalValue::String(handle)) => input.handle = Some(handle),
                ("arguments", CanonicalValue::Null) => {}
                ("arguments", value @ CanonicalValue::Object(_)) => input.arguments = value,
                ("mount" | "arguments", _) => {
                    errors.insert(name, json!(["must be an object"]));
                }
                ("handle", _) => {
                    errors.insert(name, json!(["must be a string"]));
                }
                (_, _) => {
                    errors.insert(name, json!(["is not a field this tool accepts"]));
                }
            }
        }
        if errors.is_empty() {
            Ok(input)
        } else {
            Err(errors)
        }
    }
}

/// Add each issue to `errors` under `{prefix}.{field}`.
fn collect(errors: &mut Map<String, Value>, prefix: &str, issues: Vec<FieldIssue>) {
    for issue in issues {
        let field = if issue.field().is_empty() {
            prefix.to_owned()
        } else {
            format!("{prefix}.{}", issue.field())
        };
        errors.insert(field, json!([issue.kind().message()]));
    }
}

/// Ask the Gate whether `caller` may mount the component from `mount`,
/// before anything is loaded (AGENT-013). An ability no policy defines is
/// refused, so a `Gate::before` hook that allows everything cannot mount a
/// component nobody decided an agent may mount.
async fn authorize_mount(
    target: &LiveTarget,
    caller: &ToolCaller,
    mount: &CanonicalValue,
) -> Result<(), HttpResponse> {
    let ability = format!("live:{}.mount", target.component.as_str());
    if !crate::authorization::Gate::has::<ToolCaller, LiveMountRequest>(&ability) {
        tracing::warn!(
            ability = %ability,
            tool = target.tool,
            "a headless Live tool was called, but no policy defines its mount ability: define \
             it with Gate::define::<ToolCaller, LiveMountRequest>"
        );
        return Err(refuse(403, "This action is unauthorized."));
    }
    let properties = match to_json(mount) {
        Value::Object(properties) => properties,
        _ => Map::new(),
    };
    let request = LiveMountRequest::new(target.component.as_str(), properties);
    if crate::authorization::Gate::allows_async(&ability, caller, &request).await {
        Ok(())
    } else {
        Err(refuse(403, "This action is unauthorized."))
    }
}

/// Mount the component on the server from `mount` and the resolved
/// properties, as a document mounts an identity-bound island, and return
/// its signed snapshot.
async fn mount_component(
    runtime: &LiveRuntime,
    request: &Request,
    descriptor: &ComponentDescriptor,
    target: &LiveTarget,
    caller: &ToolCaller,
    scope: ScopeFingerprint,
    mount: CanonicalValue,
) -> Result<Vec<u8>, HttpResponse> {
    let CanonicalValue::Object(mut properties) = mount else {
        return Err(refuse(422, "The mount properties must be an object."));
    };
    // The application fills these from the caller; the schema leaves them
    // out and the check above refused any the agent sent.
    for resolver in target.resolvers {
        match (resolver.resolve)(caller).await {
            Ok(value) => {
                properties.insert(resolver.property.to_owned(), value);
            }
            Err(error) => {
                return Err(match error.status_code() {
                    status @ 400..=499 => HttpResponse::from(error).status(status),
                    _ => failed(&error),
                });
            }
        }
    }
    // An agent writes a 64-bit integer as a number or a decimal string;
    // the mount's snapshot schema takes the lossless tag a template's
    // parameters carry.
    let properties = descriptor
        .parameter_schema()
        .snapshot_parameters(&CanonicalValue::Object(properties), &InputLimits::default())
        .map_err(|_| refuse(422, "The mount properties are not valid."))?;
    let selection = agent_selection(descriptor).map_err(|error| failed(&error))?;
    let context = runtime
        .validate_request_context_with_scope(request, selection, Some(scope), None)
        .map_err(|error| failed(&error))?;
    let key = DocumentMountKey::parse(AGENT_DOCUMENT_KEY).map_err(|_| {
        failed(&FrameworkError::internal(
            "the agent document key was refused",
        ))
    })?;
    let mut document = DocumentMountScope::new();
    let output = runtime
        .mount_private_component(
            &mut document,
            PrivateMountRequest::new(key, properties, MountFlags::empty()),
            &context,
        )
        .await
        .map_err(|error| failed(&error))?;
    Ok(output.metadata().signed_snapshot().to_vec())
}

/// Invoke the action on `envelope` through the browser endpoint's service,
/// and answer what the agent may see of the outcome.
#[allow(
    clippy::too_many_arguments,
    reason = "each authority the call runs under is passed explicitly"
)]
async fn run_action(
    runtime: &LiveRuntime,
    request: &Request,
    descriptor: &ComponentDescriptor,
    target: &LiveTarget,
    scope: ScopeFingerprint,
    envelope: Vec<u8>,
    arguments: CanonicalValue,
    fresh: bool,
) -> HttpResponse {
    let limits = InputLimits::default();
    let refused = |status: u16, message: &str| {
        if fresh {
            refuse(status, message)
        } else {
            refuse(409, INVALID_HANDLE)
        }
    };
    let Ok(snapshot) = parse_canonical_value(&envelope, &limits) else {
        return refused(500, "The tool failed.");
    };
    let Some(revision) = body_field(&snapshot, "revision").and_then(text) else {
        return refused(500, "The tool failed.");
    };
    // A handle past its snapshot's expiry is refused as an invalid one,
    // before it runs; the endpoint's own verification refuses it anyway,
    // as it refuses an expired browser snapshot, but answers a browser
    // with a refresh, which would read to an agent as a stale handle.
    if !fresh {
        let expires_at = body_field(&snapshot, "expires_at")
            .and_then(text)
            .and_then(|value| value.parse::<u64>().ok());
        match (expires_at, runtime.now()) {
            (Some(expires_at), Ok(now)) if now.get() < expires_at => {}
            _ => return refuse(409, INVALID_HANDLE),
        }
    }
    let update = update_request(target, &revision, snapshot, arguments);
    let Ok(update) = serde_json::to_vec(&update) else {
        return failed(&FrameworkError::internal(
            "the headless Live request could not be encoded",
        ));
    };
    let Ok(media) = ParsedLiveMediaType::parse(LIVE_MEDIA_TYPE_V2) else {
        return failed(&FrameworkError::internal("the Live media type was refused"));
    };
    let Ok(selection) = runtime.inspect_mount(&update, media) else {
        return refused(500, "The tool failed.");
    };
    // The snapshot must be one this route mounted, of the tool's own
    // component: a browser island's snapshot, or another component's,
    // never runs here.
    let expected = match agent_selection(descriptor) {
        Ok(expected) => expected,
        Err(error) => return failed(&error),
    };
    if selection.route() != expected.route()
        || selection.slot() != expected.slot()
        || selection.component() != &target.component
    {
        return refused(409, INVALID_HANDLE);
    }
    let Ok(context) =
        runtime.validate_request_context_with_scope(request, selection, Some(scope.clone()), None)
    else {
        return refused(500, "The tool failed.");
    };
    let endpoint_request = match LiveEndpointRequest::try_new(
        hyper::Method::POST,
        media,
        Bytes::from(update),
        Some(context),
        RequestCachePolicy::Bypass,
    ) {
        Ok(endpoint_request) => endpoint_request,
        Err(_) => return refused(500, "The tool failed."),
    };
    let observer = super::action::RefreshObserver::default();
    let (service, _completion) = runtime.endpoint_service_with(
        None,
        super::action::KernelExtras {
            refresh_observer: Some(observer.clone()),
            tool_claim: None,
        },
    );
    // A flash the action sets would go to the browser's session, which a
    // headless call has none of: the completion that commits it is
    // dropped, and with it the flash.
    let ((response, failure), _panic_reported) =
        crate::reporting::capture_engine_reports(service.handle_reported(endpoint_request)).await;
    if let Some(error) = failure {
        tracing::warn!(
            tool = target.tool,
            kind = ?error.kind(),
            "a headless Live tool call was refused by the Live endpoint"
        );
        return refused(500, "The tool failed.");
    }
    let Ok(answer) = parse_canonical_value(&response.body, &InputLimits::ceiling()) else {
        return refused(500, "The tool failed.");
    };
    let outcome = top_field(&answer, "outcome")
        .and_then(text)
        .unwrap_or_default();
    match outcome.as_str() {
        "accepted" => accepted(descriptor, &scope, &answer),
        "refresh_required" => match observer.cause() {
            Some(suprnova_live::execution::ExecutionFailure::Action(
                suprnova_live::component::ActionExecutionErrorKind::Action(
                    suprnova_live::action::ActionErrorKind::AuthorizationDenied,
                ),
            )) => refuse(403, "This action is unauthorized."),
            Some(suprnova_live::execution::ExecutionFailure::Action(
                suprnova_live::component::ActionExecutionErrorKind::Action(
                    suprnova_live::action::ActionErrorKind::InvalidArguments,
                ),
            )) => refuse(422, "The arguments are not valid."),
            _ if fresh => refuse(500, "The tool failed."),
            _ => refuse(409, STALE_HANDLE),
        },
        "rejected" => refuse(409, "The component is busy: call again."),
        _ => refused(500, "The tool failed."),
    }
}

/// The answer of an accepted action: its outcome, the agent-visible fields
/// of the successor snapshot, any validation issues, and the successor
/// sealed as the next handle.
fn accepted(
    descriptor: &ComponentDescriptor,
    scope: &ScopeFingerprint,
    answer: &CanonicalValue,
) -> HttpResponse {
    let Some(snapshot) = top_field(answer, "snapshot") else {
        return failed(&FrameworkError::internal(
            "an accepted headless Live call answered no snapshot",
        ));
    };
    let Ok(envelope) =
        suprnova_live::canonical::to_canonical_bytes(snapshot, &InputLimits::ceiling())
    else {
        return failed(&FrameworkError::internal(
            "the successor snapshot could not be encoded",
        ));
    };
    let handle = match seal(&envelope, scope) {
        Ok(handle) => handle,
        Err(error) => return failed(&error),
    };
    let state = body_field(snapshot, "state")
        .cloned()
        .unwrap_or_else(|| CanonicalValue::Object(BTreeMap::new()));
    let fields = suprnova_live::metadata::agent_fields(descriptor.metadata(), &state);
    let expires_at = body_field(snapshot, "expires_at")
        .and_then(text)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let validation = validation_of(answer);
    let redirect = top_field(answer, "redirect").is_some();
    let no_render = top_field(answer, "render")
        .and_then(|render| match render {
            CanonicalValue::Object(render) => render.get("kind"),
            _ => None,
        })
        .and_then(text)
        .is_some_and(|kind| kind == "no_render");
    let outcome = if !validation.is_empty() {
        "invalid"
    } else if redirect {
        "redirect"
    } else if no_render {
        "no_render"
    } else {
        "accepted"
    };
    let mut structured = Map::new();
    structured.insert("outcome".to_owned(), json!(outcome));
    structured.insert("fields".to_owned(), to_json(&fields));
    if !validation.is_empty() {
        structured.insert("validation".to_owned(), Value::Object(validation));
    }
    structured.insert("handle".to_owned(), json!(handle));
    structured.insert("expiresAt".to_owned(), json!(expires_at));
    let body = json!({
        "isError": outcome == "invalid",
        "structuredContent": Value::Object(structured),
    });
    HttpResponse::bytes_body(Bytes::from(body.to_string()), RESULT_MEDIA_TYPE)
}

/// Each validation issue's path with the text the user would read, or its
/// message identity when it has no text.
fn validation_of(answer: &CanonicalValue) -> Map<String, Value> {
    let Some(CanonicalValue::Object(issues)) = top_field(answer, "validation") else {
        return Map::new();
    };
    issues
        .iter()
        .map(|(path, issue)| {
            let message = match issue {
                CanonicalValue::String(id) => id.clone(),
                CanonicalValue::Object(issue) => issue
                    .get("text")
                    .or_else(|| issue.get("id"))
                    .and_then(text)
                    .unwrap_or_default(),
                _ => String::new(),
            };
            (path.clone(), json!([message]))
        })
        .collect()
}

/// The Live update request a browser would send to invoke the action on
/// `snapshot`, in the protocol the shipped runtime speaks: a fresh
/// correlation id and idempotency key, the snapshot's revision, no model
/// proposal, and the agent document key.
fn update_request(
    target: &LiveTarget,
    revision: &str,
    snapshot: CanonicalValue,
    arguments: CanonicalValue,
) -> Value {
    json!({
        "protocol_version": 2,
        "runtime_contract_version": 2,
        "snapshot_schema_version": 1,
        "correlation_id": random_identity(),
        "idempotency_key": random_identity(),
        "component": target.component.as_str(),
        "base_revision": revision,
        "snapshot": { "kind": "instance", "envelope": snapshot },
        "child_parameters": null,
        "model_proposals": {},
        "operations": [{
            "kind": "invoke_action",
            "name": target.action,
            "arguments": arguments,
        }],
        "extensions": { DOCUMENT_KEY_EXTENSION_V1: AGENT_DOCUMENT_KEY },
    })
}

/// 16 random bytes as unpadded base64url: the form of a correlation id and
/// an idempotency key.
fn random_identity() -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(uuid::Uuid::new_v4().as_bytes())
}

/// `value` as JSON, an integral number as an integer: through its
/// canonical bytes, which write a number as JSON does.
fn to_json(value: &CanonicalValue) -> Value {
    suprnova_live::canonical::to_canonical_bytes(value, &InputLimits::ceiling())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

/// A field of a top-level object.
fn top_field<'a>(value: &'a CanonicalValue, name: &str) -> Option<&'a CanonicalValue> {
    match value {
        CanonicalValue::Object(fields) => fields.get(name),
        _ => None,
    }
}

/// A field of a snapshot envelope's body.
fn body_field<'a>(envelope: &'a CanonicalValue, name: &str) -> Option<&'a CanonicalValue> {
    top_field(envelope, "body").and_then(|body| top_field(body, name))
}

/// A string value.
fn text(value: &CanonicalValue) -> Option<String> {
    match value {
        CanonicalValue::String(text) => Some(text.clone()),
        _ => None,
    }
}

/// The refusal of a handle that does not open for this caller, is not one,
/// or has expired: one answer, so a caller learns nothing about which.
const INVALID_HANDLE: &str = "The handle is not valid for this call: call again without one to \
     start a new component.";

/// The refusal of a handle a later call already moved past.
const STALE_HANDLE: &str = "The handle is out of date: call again with the latest handle, or \
     without one.";

/// The context a handle is sealed under: the caller's Live scope.
fn seal_context(scope: &ScopeFingerprint) -> String {
    scope.to_base64url()
}

/// Seal a snapshot envelope as a handle for the caller of `scope`.
fn seal(envelope: &[u8], scope: &ScopeFingerprint) -> Result<String, FrameworkError> {
    let envelope = std::str::from_utf8(envelope)
        .map_err(|_| FrameworkError::internal("a Live snapshot is not UTF-8"))?;
    Crypt::encrypt_string_for(
        CryptPurpose::LiveAgentHandle,
        &seal_context(scope),
        envelope,
    )
}

/// The snapshot envelope a handle seals, when it opens for the caller of
/// `scope`.
fn unseal(handle: &str, scope: &ScopeFingerprint) -> Option<Vec<u8>> {
    Crypt::decrypt_string_for(CryptPurpose::LiveAgentHandle, &seal_context(scope), handle)
        .ok()
        .map(String::into_bytes)
}

/// The catalog selection of a component's headless calls: the agent route,
/// a slot named for the component, its contract and protocol.
fn agent_selection(descriptor: &ComponentDescriptor) -> Result<MountSelection, FrameworkError> {
    let metadata = descriptor.metadata();
    Ok(MountSelection::new(
        agent_route()?,
        agent_slot(descriptor)?,
        metadata.identity().clone(),
        descriptor.contract_digest().clone(),
        metadata.versions().minimum_protocol(),
    ))
}

/// The route identity of `/__live/agent`, derived as a document route's.
fn agent_route() -> Result<suprnova_live::identity::RouteIdentity, FrameworkError> {
    super::document::route_identity(super::LIVE_AGENT_PATH)
        .map_err(|_| FrameworkError::internal("the Live agent route identity was refused"))
}

/// The slot of a component's headless calls: its name, which the slot
/// grammar shares.
fn agent_slot(descriptor: &ComponentDescriptor) -> Result<IslandSlot, FrameworkError> {
    IslandSlot::parse(descriptor.metadata().identity().as_str())
        .map_err(|_| FrameworkError::internal("a Live component name is not a slot"))
}

/// Register one catalog entry for each component with a headless tool, so
/// the private mount and the endpoint find it: the agent route, the
/// component's slot, the `agent` document key, the session required
/// absent, and the principal and tenant optional, bound into the caller's
/// scope instead.
pub(crate) fn register_catalog_entries(router: &mut Router) -> Result<(), FrameworkError> {
    let mut registered = std::collections::BTreeSet::new();
    for record in inventory::iter::<crate::agents::LiveToolRecord>() {
        if !record.is_headless() {
            continue;
        }
        let registration = record.registration().map_err(|error| {
            FrameworkError::internal(format!(
                "the component of the Live tool `{}` does not register: {error}",
                record.name()
            ))
        })?;
        let descriptor = registration.descriptor();
        let component = descriptor.metadata().identity().clone();
        if !registered.insert(component.clone()) {
            continue;
        }
        let schemas = descriptor.snapshot_schemas().map_err(|_| {
            FrameworkError::internal(format!(
                "the Live component `{}` has no snapshot schema",
                component.as_str()
            ))
        })?;
        let contract = SnapshotContract::new(
            component.clone(),
            descriptor.contract_digest().clone(),
            schemas.state().version(),
            schemas.memo().version(),
            schemas.mount().version(),
        )
        .map_err(|_| FrameworkError::internal("a Live component contract was refused"))?;
        let build = BuildId::parse(concat!("suprnova-", env!("CARGO_PKG_VERSION")))
            .map_err(|_| FrameworkError::internal("the Live build identity was refused"))?;
        let selection = agent_selection(descriptor)?;
        let key = DocumentMountKey::parse(AGENT_DOCUMENT_KEY)
            .map_err(|_| FrameworkError::internal("the agent document key was refused"))?;
        let expected = ExpectedSeedV1::new(
            contract,
            build.clone(),
            selection.route().clone(),
            selection.slot().clone(),
            schemas,
        );
        router.register_live_mount_entry(super::runtime::LiveMountRegistration::agent(
            MountCatalogEntry::new(
                expected,
                MountScopeRequirements::new(
                    ScopeRequirement::Absent,
                    ScopeRequirement::Optional,
                    ScopeRequirement::Optional,
                ),
            )
            .with_document_key(key.clone()),
            selection,
            key,
            build,
        ))?;
    }
    Ok(())
}

/// A refusal the MCP endpoint turns into a tool error with `message`.
fn refuse(status: u16, message: &str) -> HttpResponse {
    HttpResponse::json(json!({ "message": message })).status(status)
}

/// A 422 naming each field, which the MCP endpoint reports field by field.
fn invalid(errors: Map<String, Value>) -> HttpResponse {
    HttpResponse::json(json!({
        "message": "The input is not valid.",
        "errors": Value::Object(errors),
    }))
    .status(422)
}

/// A 500 that names nothing internal; the error is logged.
fn failed(error: &FrameworkError) -> HttpResponse {
    tracing::error!(
        error = %crate::error::render_error_chain(error),
        "a headless Live tool call failed"
    );
    refuse(500, "The tool failed.")
}
