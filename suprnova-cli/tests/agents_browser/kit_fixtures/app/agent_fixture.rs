//! The AGENT-014 and AGENT-015 browser fixture, copied into a scaffolded
//! kit as `src/controllers/agent_fixture.rs`: two pages that offer server
//! tools, a page with two form tools, the form requests those tools submit,
//! and a recorder of every request the fixture's routes see.

use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

use suprnova::serde_json::{Value, json};
use suprnova::{
    HttpResponse, Inertia, InertiaProps, Middleware, Next, Request, Response, ToolCaller,
    async_trait, handler, inertia_response, json_response, tool, tool_input,
};

/// The number the first page shows and `fixture.bump` adds to.
static COUNTER: AtomicI64 = AtomicI64::new(0);
/// The notes the form tool saved, newest last.
static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Every request the fixture's routes saw, oldest first.
static REQUESTS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

/// Records each request under `/agent-fixture` with what a browser tool
/// call or an Inertia visit carries: the tool-call header, the Inertia
/// header, a CSRF header and the session cookie.
pub struct Recorder;

#[async_trait]
impl Middleware for Recorder {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let entry = json!({
            "method": request.method().as_str(),
            "path": request.path(),
            "tool": request.header("Suprnova-Tool-Call"),
            "inertia": request.header("X-Inertia").is_some(),
            "csrf": request.header("X-CSRF-TOKEN").is_some()
                || request.header("X-XSRF-TOKEN").is_some(),
            "cookie": request.header("Cookie").is_some(),
        });
        if let Ok(mut requests) = REQUESTS.lock() {
            requests.push(entry);
        }
        next(request).await
    }
}

#[derive(InertiaProps)]
pub struct AgentFixtureProps {
    pub counter: i64,
}

#[derive(InertiaProps)]
pub struct AgentFixtureFormProps {
    pub notes: Vec<String>,
}

fn counter() -> i64 {
    COUNTER.load(Ordering::SeqCst)
}

/// `GET /agent-fixture/first`: offers `fixture.bump`, `fixture.peek` and
/// `fixture.mine`, beside the group's `fixture.help`.
#[handler]
pub async fn first(req: Request) -> Response {
    inertia_response!(&req, "AgentFixture/First", AgentFixtureProps { counter: counter() })
}

/// `GET /agent-fixture/second`: offers `fixture.reset` and `fixture.help`.
#[handler]
pub async fn second(req: Request) -> Response {
    inertia_response!(&req, "AgentFixture/Second", AgentFixtureProps { counter: counter() })
}

/// `GET /agent-fixture/form`: the page with the two form tools.
#[handler]
pub async fn form(req: Request) -> Response {
    let notes = NOTES.lock().map(|notes| notes.clone()).unwrap_or_default();
    inertia_response!(&req, "AgentFixture/Form", AgentFixtureFormProps { notes })
}

/// What `fixture.bump` adds.
#[tool_input]
pub struct BumpInput {
    /// How much to add, from 1 to 10.
    #[validate(range(min = 1, max = 10))]
    pub by: i64,
}

#[tool(
    name = "fixture.bump",
    description = "Add to the counter the first page shows.",
    title = "Bump",
    callers = [public, user],
    input = BumpInput,
    annotations(read_only = false, destructive = false),
)]
#[handler]
pub async fn bump(caller: ToolCaller, form: BumpInput) -> Response {
    let counter = COUNTER.fetch_add(form.by, Ordering::SeqCst) + form.by;
    json_response!({
        "counter": counter,
        "browser": caller.via_browser(),
        "kind": caller.kind().as_str(),
        "user": caller.user_id(),
    })
}

#[tool(
    name = "fixture.peek",
    description = "Read the counter the first page shows.",
    callers = [public],
    annotations(read_only, untrusted_content),
)]
#[handler]
pub async fn peek() -> Response {
    json_response!({ "counter": counter() })
}

#[tool(
    name = "fixture.mine",
    description = "Read the signed-in user's id.",
    callers = [user],
)]
#[handler]
pub async fn mine(caller: ToolCaller) -> Response {
    json_response!({ "user": caller.user_id(), "browser": caller.via_browser() })
}

#[tool(
    name = "fixture.reset",
    description = "Set the counter back to zero.",
    callers = [public],
    annotations(destructive),
)]
#[handler]
pub async fn reset() -> Response {
    COUNTER.store(0, Ordering::SeqCst);
    json_response!({ "counter": 0 })
}

#[tool(
    name = "fixture.help",
    description = "Say what the fixture pages do.",
    callers = [public],
    annotations(read_only),
)]
#[handler]
pub async fn help() -> Response {
    json_response!({ "help": "The first page counts; the form page writes notes." })
}

/// The note form, the form tool `fixture.notes.create` submits.
#[tool_input]
pub struct FixtureNoteRequest {
    /// The note's title.
    #[validate(length(min = 3, max = 80, message = "Give the note a title of 3 to 80 characters."))]
    pub title: String,
    /// What the note says.
    #[serde(default)]
    pub body: Option<String>,
}

/// `POST /agent-fixture/notes`: save the note and go back to the form.
#[handler]
pub async fn store_note(form: FixtureNoteRequest) -> Response {
    if let Ok(mut notes) = NOTES.lock() {
        notes.push(form.title);
    }
    Inertia::back(302, Some("/agent-fixture/form")).into()
}

/// The destructive form, the form tool `fixture.notes.purge` fills.
#[tool_input]
pub struct PurgeRequest {
    /// The word the user types to confirm.
    #[validate(length(min = 1))]
    pub confirm: String,
}

/// `POST /agent-fixture/purge`: delete every note.
#[handler]
pub async fn purge(_form: PurgeRequest) -> Response {
    if let Ok(mut notes) = NOTES.lock() {
        notes.clear();
    }
    Inertia::back(302, Some("/agent-fixture/form")).into()
}

/// `GET /agent-fixture/schema`: the schema the server derives for the note
/// form, to compare with the one the page registered.
#[handler]
pub async fn schema() -> Response {
    Ok(HttpResponse::json(suprnova::agents::__input_schema::<FixtureNoteRequest>()))
}

/// `GET /agent-fixture/requests`: what the recorder saw.
#[handler]
pub async fn requests() -> Response {
    let requests = REQUESTS.lock().map(|requests| requests.clone()).unwrap_or_default();
    Ok(HttpResponse::json(json!({ "requests": requests })))
}
