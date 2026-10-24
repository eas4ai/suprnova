//! The browser half, on `wasm32-unknown-unknown`: registration with
//! `document.modelContext` and server tool calls through `fetch`.
//!
//! web-sys has no binding for WebMCP, so the model context is reached with
//! `Reflect`, as the Live runtime's `modelContextOf` reaches it: a
//! `modelContext` on the document, else on the navigator, with a
//! `registerTool` function.

use std::cell::Cell;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::rc::Rc;
use std::task::Poll;

use js_sys::{Function, JSON, Object, Promise, Reflect, Uint8Array};
use serde_json::Value;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, future_to_promise};
use web_sys::{
    AbortController, AbortSignal, Document, DomException, Headers, HtmlDocument,
    ReadableStreamDefaultReader, RequestCache, RequestCredentials, RequestInit, RequestRedirect,
    Response, ResponseType, Url, Window,
};

use crate::LocalFuture;
use crate::csrf;
use crate::error::{Unsupported, WebMcpError};
use crate::result::{self, Answer, ToolResult};
use crate::server::{self, Limits, ServerTool};

/// The header that marks a request as a browser tool call, which the
/// server checks and counts against the caller's tool budget.
const TOOL_CALL_HEADER: &str = "Suprnova-Tool-Call";

/// Where the server writes the page's server tools. Only the head is read,
/// so markup a user wrote into the body cannot pose as the element.
const PAGE_TOOLS: &str = "head > script#suprnova-agent-tools[type=\"application/json\"]";

/// The agent's call was aborted through the signal WebMCP passed.
pub(crate) struct Aborted;

/// What answers one call of a registered tool: the input as JSON text and
/// the agent's abort signal, if WebMCP passed one.
type Execute = Rc<dyn Fn(String, Option<AbortSignal>) -> LocalFuture<Result<ToolResult, Aborted>>>;

/// A tool registered with the page's model context.
pub(crate) struct Registration {
    controller: AbortController,
    settled: Option<Promise>,
    // Declared last, so it is freed after `drop` has aborted the controller
    // and the browser no longer calls it.
    _execute: Closure<dyn FnMut(JsValue, JsValue) -> Promise>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        // Aborting the signal passed to `registerTool` is how WebMCP removes
        // a tool.
        self.controller.abort();
    }
}

impl Registration {
    pub(crate) async fn registered(&self, name: &str) -> Result<(), WebMcpError> {
        let Some(settled) = &self.settled else {
            return Ok(());
        };
        JsFuture::from(settled.clone())
            .await
            .map(drop)
            .map_err(|reason| WebMcpError::Rejected {
                name: name.to_string(),
                reason: describe(&reason),
            })
    }
}

pub(crate) fn available() -> Result<(), WebMcpError> {
    let (window, document) = page()?;
    model_context(&window, &document)
        .map(drop)
        .ok_or(WebMcpError::Unsupported(Unsupported::NoModelContext))
}

pub(crate) fn register_page_tool(
    name: &str,
    descriptor: &Value,
    execute: Rc<dyn Fn(String) -> LocalFuture<ToolResult>>,
) -> Result<Registration, WebMcpError> {
    register(
        name,
        descriptor,
        Rc::new(move |input, _signal| {
            let run = execute(input);
            Box::pin(async move { Ok(run.await) })
        }),
    )
}

pub(crate) fn register_server_tool(
    tool: &ServerTool,
    limits: Limits,
    on_write: Rc<dyn Fn() -> LocalFuture<()>>,
) -> Result<Registration, WebMcpError> {
    let shared = Rc::new(tool.clone());
    register(
        &tool.name,
        &tool.descriptor(),
        Rc::new(move |input, signal| {
            let tool = Rc::clone(&shared);
            let on_write = Rc::clone(&on_write);
            Box::pin(async move {
                let arguments = match serde_json::from_str::<Value>(&input) {
                    Ok(arguments) => arguments,
                    Err(error) => return Ok(result::invalid_input(&error.to_string())),
                };
                let called = call(&tool, limits, &arguments, signal).await?;
                if called.answered && tool.refreshes() {
                    // Only the server knows whether a refused or failed call
                    // changed anything, so any answer refreshes. The result
                    // stands whatever the refresh does.
                    let _ = within(on_write(), limits.call_timeout_ms).await;
                }
                Ok(called.result)
            })
        }),
    )
}

pub(crate) fn page_tools() -> Result<Option<String>, WebMcpError> {
    let (_, document) = page()?;
    Ok(document
        .query_selector(PAGE_TOOLS)
        .ok()
        .flatten()
        .map(|element| element.text_content().unwrap_or_default()))
}

pub(crate) async fn call_tool(
    tool: &ServerTool,
    limits: Limits,
    arguments: &Value,
) -> Result<ToolResult, WebMcpError> {
    page()?;
    match call(tool, limits, arguments, None).await {
        Ok(called) => Ok(called.result),
        // Only an agent's signal aborts a call, and this one has none.
        Err(Aborted) => Ok(result::unreachable()),
    }
}

/// The page's window and document.
fn page() -> Result<(Window, Document), WebMcpError> {
    let missing = WebMcpError::Unsupported(Unsupported::NoDocument);
    let window = web_sys::window().ok_or_else(|| missing.clone())?;
    let document = window.document().ok_or(missing)?;
    Ok((window, document))
}

/// The page's model context and its `registerTool`.
fn model_context(window: &Window, document: &Document) -> Option<(JsValue, Function)> {
    let usable = |candidate: JsValue| -> Option<(JsValue, Function)> {
        if !candidate.is_object() {
            return None;
        }
        let register = Reflect::get(&candidate, &JsValue::from_str("registerTool"))
            .ok()?
            .dyn_into::<Function>()
            .ok()?;
        Some((candidate, register))
    };
    // The draft's home is the document; the navigator is read only where
    // the document has none, since Chromium warns that it is deprecated.
    Reflect::get(document, &JsValue::from_str("modelContext"))
        .ok()
        .and_then(usable)
        .or_else(|| {
            let navigator = Reflect::get(window, &JsValue::from_str("navigator")).ok()?;
            if !navigator.is_object() {
                return None;
            }
            Reflect::get(&navigator, &JsValue::from_str("modelContext"))
                .ok()
                .and_then(usable)
        })
}

/// Register `descriptor` with `execute` answering its calls, under an
/// abort controller of its own.
fn register(name: &str, descriptor: &Value, execute: Execute) -> Result<Registration, WebMcpError> {
    let (window, document) = page()?;
    let (context, register_tool) = model_context(&window, &document)
        .ok_or(WebMcpError::Unsupported(Unsupported::NoModelContext))?;
    let rejected = |reason: &JsValue| WebMcpError::Rejected {
        name: name.to_string(),
        reason: describe(reason),
    };
    let text = serde_json::to_string(descriptor)
        .map_err(|error| WebMcpError::Output(error.to_string()))?;
    let tool = JSON::parse(&text).map_err(|reason| rejected(&reason))?;
    let callback = Closure::<dyn FnMut(JsValue, JsValue) -> Promise>::new(
        move |input: JsValue, client: JsValue| answer(&execute, &input, &client),
    );
    Reflect::set(&tool, &JsValue::from_str("execute"), callback.as_ref())
        .map_err(|reason| rejected(&reason))?;
    let controller = AbortController::new().map_err(|reason| rejected(&reason))?;
    let options = Object::new();
    Reflect::set(&options, &JsValue::from_str("signal"), &controller.signal())
        .map_err(|reason| rejected(&reason))?;
    let returned = register_tool
        .call2(&context, &tool, &options)
        .map_err(|reason| rejected(&reason))?;
    Ok(Registration {
        controller,
        settled: returned.dyn_into::<Promise>().ok(),
        _execute: callback,
    })
}

/// The promise `execute` returns for one call: the tool result, or a
/// rejection with an `AbortError` once the agent's signal aborts, as the
/// Live runtime's calls reject.
fn answer(execute: &Execute, input: &JsValue, client: &JsValue) -> Promise {
    let signal = client
        .is_object()
        .then(|| Reflect::get(client, &JsValue::from_str("signal")).ok())
        .flatten()
        .and_then(|signal| signal.dyn_into::<AbortSignal>().ok());
    if signal.as_ref().is_some_and(AbortSignal::aborted) {
        return Promise::reject(&abort_error());
    }
    // A tool whose schema has no required field may be called with no
    // input at all.
    let text = if input.is_undefined() {
        Some("{}".to_string())
    } else {
        JSON::stringify(input)
            .ok()
            .and_then(|text| text.as_string())
    };
    let execute = Rc::clone(execute);
    future_to_promise(async move {
        let Some(text) = text else {
            return to_js(&result::invalid_input("the input is not JSON"));
        };
        let work = execute(text, signal.clone());
        let outcome = match &signal {
            Some(signal) => until_aborted(work, signal).await,
            None => work.await,
        };
        match outcome {
            Ok(result) => to_js(&result),
            Err(Aborted) => Err(abort_error()),
        }
    })
}

/// What a call came back with, and whether the server answered it, which
/// decides whether the page refreshes.
struct Called {
    result: ToolResult,
    answered: bool,
}

/// Call `tool` with `arguments` (section 0.3 of the page tools contract):
/// a same-origin request, with the session cookie, the CSRF header and
/// the tool call header, stopped at the call timeout, its body read up to
/// the result bound.
async fn call(
    tool: &ServerTool,
    limits: Limits,
    arguments: &Value,
    signal: Option<AbortSignal>,
) -> Result<Called, Aborted> {
    let refused = |result: ToolResult| {
        Ok(Called {
            result,
            answered: false,
        })
    };
    let plan = match server::plan(tool, arguments) {
        Ok(plan) => plan,
        Err(result) => return refused(result),
    };
    let Ok((window, document)) = page() else {
        return refused(result::unreachable());
    };
    let Ok(origin) = window.location().origin() else {
        return refused(result::unreachable());
    };
    let url = format!("{origin}{}", plan.target);
    // Before any credential is attached: the request must stay on the
    // page's origin.
    if !Url::new(&url).is_ok_and(|parsed| parsed.origin() == origin) {
        return refused(ToolResult::error(
            "The tool's address is not on the page's origin, so the call was not sent.",
        ));
    }
    let Some(init) = request(tool, &plan, &document) else {
        return refused(ToolResult::error(
            "The tool call's request could not be built.",
        ));
    };
    let Ok(controller) = AbortController::new() else {
        return refused(ToolResult::error(
            "The tool call's request could not be built.",
        ));
    };
    init.set_signal(Some(&controller.signal()));
    let _linked = signal.as_ref().map(|signal| Link::new(signal, &controller));
    let timed_out = Rc::new(Cell::new(false));
    let _timer = Timer::start(&window, limits.call_timeout_ms, {
        let controller = controller.clone();
        let timed_out = Rc::clone(&timed_out);
        move || {
            timed_out.set(true);
            controller.abort();
        }
    });
    // Why the request or its body stopped: the timer, the agent, or the
    // network.
    let stopped = |answered: bool| {
        if timed_out.get() {
            Ok(Called {
                result: result::timed_out(limits.call_timeout_ms),
                answered,
            })
        } else if signal.as_ref().is_some_and(AbortSignal::aborted) {
            Err(Aborted)
        } else {
            Ok(Called {
                result: result::unreachable(),
                answered,
            })
        }
    };
    let response = match JsFuture::from(window.fetch_with_str_and_init(&url, &init)).await {
        Ok(response) => match response.dyn_into::<Response>() {
            Ok(response) => response,
            Err(_) => return stopped(false),
        },
        Err(_) => return stopped(false),
    };
    let content_type = response.headers().get("content-type").ok().flatten();
    let opaque_redirect = response.type_() == ResponseType::Opaqueredirect;
    let Ok(body) = read_body(&response, limits.max_result_bytes).await else {
        return stopped(true);
    };
    let answer = Answer {
        status: response.status(),
        opaque_redirect,
        content_type: content_type.as_deref(),
        body: &body,
    };
    Ok(Called {
        result: result::answer_result(&answer, limits.max_result_bytes),
        answered: true,
    })
}

/// The fetch options of a call: its method, headers and body. Redirects
/// are not followed, since fetch keeps custom headers across a redirect
/// and could carry the CSRF header to another origin.
fn request(tool: &ServerTool, plan: &server::Plan, document: &Document) -> Option<RequestInit> {
    let headers = Headers::new().ok()?;
    headers.set("Accept", "application/json").ok()?;
    if plan.body.is_some() {
        headers.set("Content-Type", "application/json").ok()?;
    }
    headers.set(TOOL_CALL_HEADER, &tool.name).ok()?;
    let meta = document
        .query_selector("meta[name=\"csrf-token\"]")
        .ok()
        .flatten()
        .and_then(|meta| meta.get_attribute("content"));
    let cookie = document
        .dyn_ref::<HtmlDocument>()
        .and_then(|document| document.cookie().ok())
        .unwrap_or_default();
    if let Some((name, value)) = csrf::header(meta.as_deref(), &cookie) {
        headers.set(name, &value).ok()?;
    }
    let init = RequestInit::new();
    init.set_method(&plan.method);
    init.set_headers(&headers);
    if let Some(body) = &plan.body {
        init.set_body(&JsValue::from_str(body));
    }
    init.set_credentials(RequestCredentials::SameOrigin);
    init.set_redirect(RequestRedirect::Manual);
    init.set_cache(RequestCache::NoStore);
    Some(init)
}

/// The response's body, read as a stream and cut one byte past `max`,
/// which is enough to tell that it is too large. `Err` when the stream
/// fails or is aborted.
async fn read_body(response: &Response, max: usize) -> Result<Vec<u8>, ()> {
    let Some(stream) = response.body() else {
        return Ok(Vec::new());
    };
    let reader = stream
        .get_reader()
        .dyn_into::<ReadableStreamDefaultReader>()
        .map_err(drop)?;
    let limit = max.saturating_add(1);
    let mut body = Vec::new();
    loop {
        let chunk = JsFuture::from(reader.read()).await.map_err(drop)?;
        if Reflect::get(&chunk, &JsValue::from_str("done"))
            .map_err(drop)?
            .is_truthy()
        {
            return Ok(body);
        }
        let bytes = Reflect::get(&chunk, &JsValue::from_str("value"))
            .map_err(drop)?
            .dyn_into::<Uint8Array>()
            .map_err(drop)?;
        let room = u32::try_from(limit - body.len()).unwrap_or(u32::MAX);
        let take = bytes.length().min(room);
        body.extend(bytes.subarray(0, take).to_vec());
        if body.len() >= limit {
            // The rest is not needed; the promise of the cancel is not
            // either.
            let _ = reader.cancel();
            return Ok(body);
        }
    }
}

/// `work`'s output, or `Err(Aborted)` as soon as `signal` aborts.
async fn until_aborted<T>(
    work: LocalFuture<Result<T, Aborted>>,
    signal: &AbortSignal,
) -> Result<T, Aborted> {
    let mut work = work;
    let mut abort = pin!(JsFuture::from(on_abort(signal)));
    poll_fn(|context| {
        if let Poll::Ready(output) = work.as_mut().poll(context) {
            return Poll::Ready(output);
        }
        if abort.as_mut().poll(context).is_ready() {
            return Poll::Ready(Err(Aborted));
        }
        Poll::Pending
    })
    .await
}

/// `work`'s completion, or `None` after `ms` milliseconds.
async fn within(work: LocalFuture<()>, ms: u64) -> Option<()> {
    let window = web_sys::window()?;
    let mut work = work;
    let mut timer = pin!(JsFuture::from(sleep(&window, ms)));
    poll_fn(|context| {
        if work.as_mut().poll(context).is_ready() {
            return Poll::Ready(Some(()));
        }
        if timer.as_mut().poll(context).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await
}

/// A promise that resolves when `signal` aborts, at once if it has.
fn on_abort(signal: &AbortSignal) -> Promise {
    let signal = signal.clone();
    Promise::new(&mut move |resolve, _reject| {
        if signal.aborted() {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        } else {
            let _ = signal.add_event_listener_with_callback("abort", &resolve);
        }
    })
}

/// A promise that resolves after `ms` milliseconds.
fn sleep(window: &Window, ms: u64) -> Promise {
    let window = window.clone();
    Promise::new(&mut move |resolve, _reject| {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, delay(ms));
    })
}

/// `ms` as a `setTimeout` delay, which is a signed 32-bit count.
fn delay(ms: u64) -> i32 {
    i32::try_from(ms).unwrap_or(i32::MAX)
}

/// The agent's abort signal passed on to a call's own controller, so the
/// request stops too, for as long as the call lasts.
struct Link {
    signal: AbortSignal,
    listener: Closure<dyn FnMut()>,
}

impl Link {
    fn new(signal: &AbortSignal, controller: &AbortController) -> Self {
        let listener = Closure::<dyn FnMut()>::new({
            let controller = controller.clone();
            move || controller.abort()
        });
        let _ = signal.add_event_listener_with_callback("abort", listener.as_ref().unchecked_ref());
        if signal.aborted() {
            controller.abort();
        }
        Self {
            signal: signal.clone(),
            listener,
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let _ = self
            .signal
            .remove_event_listener_with_callback("abort", self.listener.as_ref().unchecked_ref());
    }
}

/// The call timer, cleared when the call ends first.
struct Timer {
    window: Window,
    handle: Option<i32>,
    _fire: Closure<dyn FnMut()>,
}

impl Timer {
    fn start(window: &Window, ms: u64, fire: impl FnMut() + 'static) -> Self {
        let fire = Closure::<dyn FnMut()>::new(fire);
        let handle = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                fire.as_ref().unchecked_ref(),
                delay(ms),
            )
            .ok();
        Self {
            window: window.clone(),
            handle,
            _fire: fire,
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle {
            self.window.clear_timeout_with_handle(handle);
        }
    }
}

/// `result` as the plain object WebMCP expects.
fn to_js(result: &ToolResult) -> Result<JsValue, JsValue> {
    let text =
        serde_json::to_string(result).map_err(|error| JsValue::from_str(&error.to_string()))?;
    JSON::parse(&text)
}

/// The `AbortError` a call rejects with when the agent aborts it.
fn abort_error() -> JsValue {
    DomException::new_with_message_and_name("The tool call was aborted.", "AbortError")
        .map_or_else(|error| error, JsValue::from)
}

/// What a JavaScript error says: its `message`, or the value as text.
fn describe(value: &JsValue) -> String {
    if value.is_object()
        && let Some(message) = Reflect::get(value, &JsValue::from_str("message"))
            .ok()
            .and_then(|message| message.as_string())
    {
        return message;
    }
    value
        .as_string()
        .unwrap_or_else(|| "the browser gave no reason".to_string())
}
