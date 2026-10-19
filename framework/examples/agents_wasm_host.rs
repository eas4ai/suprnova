//! Real Suprnova server for the `agents_browser` suite's WebAssembly tests
//! (AGENT-016): the page a Rust front end built with `suprnova-webmcp`
//! boots in, and the server tool it calls.
//!
//! `suprnova-cli/tests/agents_browser/wasm.rs` builds the front end
//! (`crates/suprnova-webmcp/examples/browser_fixture.rs`), binds it with
//! `wasm-bindgen`, starts this binary with `SUPRNOVA_WASM_PKG_DIR` naming
//! that output, and drives it in Chromium with WebMCP enabled. It serves:
//!
//! - `/app`, the page the front end boots in, whose head carries the CSRF
//!   meta tag and the `PageTools::element` listing `todos.save` for a
//!   signed-in viewer;
//! - `/pkg/{file}`, the front end's JavaScript and WebAssembly;
//! - `todos.save` at `POST /todos`, a `#[tool]` for signed-in users that
//!   answers with what it saw: the user and the session the page belongs
//!   to;
//! - `/login/{user}`, which signs the browser in as that user and writes
//!   the user into the session;
//! - `/test/schema`, the server's schema of the twin of the front end's
//!   typed tool input;
//! - `/test/requests`, every `/todos` request since `/test/reset`: its
//!   method, tool-call header, CSRF header, whether it carried the session
//!   cookie, and the status it got.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use serde_json::{Value, json};
use suprnova::agents::{__input_schema, AgentsConfig};
use suprnova::auth::Authenticatable;
use suprnova::csrf::csrf_meta_tag;
use suprnova::{
    App, Auth, CsrfMiddleware, FrameworkError, HttpResponse, Middleware, Next, PageTools, Request,
    Response, Router, Server, SessionConfig, SessionData, SessionMiddleware, SessionStore, handler,
    json_response, session, session_mut, tool, tool_input,
};

/// The input of `todos.save`.
#[tool_input]
pub struct SaveTodo {
    /// What to save.
    #[validate(length(min = 1, max = 80))]
    pub title: String,
}

// The server's twin of the front end's `todos.add` input: the same fields,
// renames, rules and doc comments, so the two derive one schema.

/// An item to add to the visible todo list.
#[allow(dead_code, reason = "only its schema is served")]
#[tool_input]
#[serde(rename_all = "camelCase")]
pub struct AddTodo {
    /// What to do.
    #[validate(length(min = 1, max = 80))]
    pub todo_title: String,
}

#[tool(
    name = "todos.save",
    description = "Save a todo for the signed-in user.",
    title = "Save todo",
    callers = [user],
    input = SaveTodo,
)]
#[handler]
pub async fn save_todo(form: SaveTodo) -> Response {
    let session_user = session().and_then(|session| session.get::<String>("signed_in_as"));
    json_response!({ "saved": form.title, "user": Auth::id(), "session": session_user })
}

/// A user known by its name.
struct User(String);

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.0.clone()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

/// Signs the request in as the user its `agents_user` cookie names, as an
/// application's guard would.
struct CookieUser;

#[async_trait]
impl Middleware for CookieUser {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let user = cookie(&request, "agents_user").filter(|user| !user.is_empty());
        if let Some(user) = user {
            Auth::set_user(Arc::new(User(user)));
        }
        next(request).await
    }
}

/// The value of the cookie `name` on `request`.
fn cookie(request: &Request, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    request.header("cookie").and_then(|cookies| {
        cookies
            .split(';')
            .map(str::trim)
            .find_map(|pair| pair.strip_prefix(prefix.as_str()))
            .map(str::to_string)
    })
}

/// Sessions held in memory, so the host needs no database.
#[derive(Default)]
struct MemorySessionStore(Mutex<HashMap<String, SessionData>>);

#[async_trait]
impl SessionStore for MemorySessionStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.0.lock().map_err(lock_error)?.get(id).cloned())
    }
    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .map_err(lock_error)?
            .insert(session.id.clone(), session.clone());
        Ok(())
    }
    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.0.lock().map_err(lock_error)?.remove(id);
        Ok(())
    }
    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.0.lock().map_err(lock_error)?;
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - sessions.len()) as u64)
    }
    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

fn lock_error<T>(_: T) -> FrameworkError {
    FrameworkError::internal("the session store lock was poisoned")
}

fn requests() -> &'static Mutex<Vec<Value>> {
    static REQUESTS: OnceLock<Mutex<Vec<Value>>> = OnceLock::new();
    REQUESTS.get_or_init(Mutex::default)
}

fn record<T>(update: impl FnOnce(&mut Vec<Value>) -> T) -> T {
    let mut guard = requests()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    update(&mut guard)
}

/// The outermost middleware: records every `/todos` request as it arrived,
/// and the status it got, a CSRF refusal included.
struct Recorder;

#[async_trait]
impl Middleware for Recorder {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let path = request.path().to_string();
        if !path.starts_with("/todos") {
            return next(request).await;
        }
        let header = |name: &str| request.header(name).map(str::to_string);
        let csrf_header = if request.header("x-csrf-token").is_some() {
            Some("X-CSRF-TOKEN")
        } else if request.header("x-xsrf-token").is_some() {
            Some("X-XSRF-TOKEN")
        } else {
            None
        };
        let entry = json!({
            "method": request.method().as_str(),
            "path": path,
            "tool": header("suprnova-tool-call"),
            "csrfHeader": csrf_header,
            "sessionCookie": cookie(&request, &SessionConfig::default().cookie_name).is_some(),
            "accept": header("accept"),
            "contentType": header("content-type"),
        });
        let response = next(request).await;
        let status = match &response {
            Ok(response) | Err(response) => response.status_code(),
        };
        record(|requests| {
            let mut entry = entry;
            entry["status"] = json!(status);
            requests.push(entry);
        });
        response
    }
}

/// The page the front end boots in.
async fn app_page(request: Request) -> Response {
    let tools = PageTools::element(&request).map_err(HttpResponse::from)?;
    Ok(HttpResponse::html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>WebAssembly tools</title>{csrf}{tools}\
         <script type=\"module\">\
         import init, {{ drop_todo_tool }} from '/pkg/browser_fixture.js';\
         window.fixture = {{ dropTodoTool: drop_todo_tool }};\
         init().catch((error) => {{ document.documentElement.dataset.fixture = 'failed: ' + error; }});\
         </script></head><body><p>WebAssembly front end</p></body></html>",
        csrf = csrf_meta_tag(),
    )))
}

/// One file of the bound front end, by a name of letters, digits, `_`, `-`
/// and `.` only, so no request reads outside the directory.
fn pkg_file(dir: &Path, name: &str) -> Response {
    let safe = !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if !safe {
        return Err(HttpResponse::text("not found").status(404));
    }
    let content_type = match Path::new(name).extension().and_then(|ext| ext.to_str()) {
        Some("js") => "text/javascript",
        Some("wasm") => "application/wasm",
        _ => return Err(HttpResponse::text("not found").status(404)),
    };
    match std::fs::read(dir.join(name)) {
        Ok(bytes) => Ok(HttpResponse::bytes_body(bytes, content_type)),
        Err(_) => Err(HttpResponse::text("not found").status(404)),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let port = std::env::var("SUPRNOVA_AGENTS_HOST_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(4179);
    let pkg: PathBuf = std::env::var("SUPRNOVA_WASM_PKG_DIR")
        .map(PathBuf::from)
        .map_err(|_| "set SUPRNOVA_WASM_PKG_DIR to the wasm-bindgen output directory")?;
    App::init();
    let router: Router = Router::new()
        .get("/health", |_request: Request| async {
            Ok(HttpResponse::text("ok"))
        })
        .get("/app", app_page)
        .with_tools(["todos.save"])
        .get("/pkg/{file}", move |request: Request| {
            let dir = pkg.clone();
            async move { pkg_file(&dir, request.param("file").unwrap_or("")) }
        })
        .post("/todos", save_todo)
        .get("/login/{user}", |request: Request| async move {
            let user = request.param("user").unwrap_or("").to_string();
            session_mut(|session| session.put("signed_in_as", user.clone()));
            Ok(HttpResponse::text("signed in").header(
                "Set-Cookie",
                format!("agents_user={user}; Path=/; SameSite=Lax"),
            ))
        })
        .get("/test/schema", |_request: Request| async {
            Ok(HttpResponse::json(__input_schema::<AddTodo>()))
        })
        .get("/test/requests", |_request: Request| async {
            let requests = record(|requests| requests.clone());
            Ok(HttpResponse::json(json!({ "requests": requests })))
        })
        .get("/test/reset", |_request: Request| async {
            record(Vec::clear);
            Ok(HttpResponse::text("reset"))
        })
        .into();
    let router = router.try_agents(AgentsConfig::new())?;
    let mut session = SessionConfig::default();
    session.cookie_secure = false;
    Server::new(router)
        .middleware(Recorder)
        .middleware(SessionMiddleware::with_store(
            session,
            Arc::new(MemorySessionStore::default()),
        ))
        .middleware(CsrfMiddleware::new())
        .middleware(CookieUser)
        .host("127.0.0.1")
        .port(port)
        .run()
        .await
}
