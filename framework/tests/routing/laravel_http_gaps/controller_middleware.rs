//! PAR-114: a resource controller declares its own middleware, scoped to
//! some of its actions, as Laravel's `HasMiddleware` does, and a route
//! leaves out a middleware its group gives it, as Laravel's
//! `withoutMiddleware` does.
//!
//! Every middleware here writes its tag into a trace when a request passes
//! it, so a test reads which middleware ran on a request, and in which
//! order. Names in the process-wide route table and the middleware alias
//! table are unique to each test, so the tests can share one process.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Mutex};

use suprnova::http::text;
use suprnova::middleware::{
    register_middleware_alias, register_middleware_alias_with_args, register_middleware_group,
};
use suprnova::routing::{ControllerMiddleware, ResourceAction, ResourceController};
use suprnova::{
    Middleware, MiddlewareRegistry, Next, Request, Response, Router, any, get, group, resource,
    route,
};

use crate::http_wire::request;
use crate::laravel_delta::{server, server_with};

/// The tags of the middleware a request passed, in the order it passed them.
type Trace = Arc<Mutex<Vec<String>>>;

fn trace() -> Trace {
    Arc::new(Mutex::new(Vec::new()))
}

/// Take what the trace holds, leaving it empty for the next request.
fn taken(trace: &Trace) -> Vec<String> {
    std::mem::take(&mut *trace.lock().unwrap())
}

/// Middleware types that write their tag into a trace and pass the request
/// on. Distinct types, so a test can leave one out by type.
macro_rules! tracing_middleware {
    ($($name:ident => $tag:literal),* $(,)?) => {$(
        #[derive(Clone)]
        struct $name(Trace);

        #[suprnova::async_trait]
        impl Middleware for $name {
            async fn handle(&self, request: Request, next: Next) -> Response {
                self.0.lock().unwrap().push($tag.to_string());
                next(request).await
            }
        }
    )*};
}

tracing_middleware! {
    Auth => "auth",
    EnsureJson => "json",
    GroupGate => "group",
    Registered => "registered",
    Declared => "declared",
}

/// A middleware an alias builds, tagged with the alias's arguments.
struct Tagged {
    tag: String,
    trace: Trace,
}

#[suprnova::async_trait]
impl Middleware for Tagged {
    async fn handle(&self, request: Request, next: Next) -> Response {
        self.trace.lock().unwrap().push(self.tag.clone());
        next(request).await
    }
}

/// Register `name` as an alias whose middleware writes `tag:<arguments>`.
fn register_tagged(name: &'static str, trace: &Trace) {
    let trace = trace.clone();
    register_middleware_alias_with_args(name, move |arguments| {
        Ok(Tagged {
            tag: format!("{name}:{}", arguments.join(",")),
            trace: trace.clone(),
        })
    });
}

/// A controller that answers every action with its name and declares the
/// middleware it is built with.
struct Ctl(Vec<ControllerMiddleware>);

fn answer(action: &'static str) -> Pin<Box<dyn Future<Output = Response> + Send>> {
    Box::pin(async move { text(action) })
}

impl ResourceController for Ctl {
    fn index(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        answer("index")
    }
    fn create(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        answer("create")
    }
    fn store(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        answer("store")
    }
    fn show(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        answer("show")
    }
    fn edit(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        answer("edit")
    }
    fn update(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        answer("update")
    }
    fn destroy(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        answer("destroy")
    }
    fn middleware(&self) -> Vec<ControllerMiddleware> {
        self.0.clone()
    }
}

/// Send `method path` and return the tags the trace collected for it,
/// after checking that the action answered.
async fn ran(addr: std::net::SocketAddr, trace: &Trace, method: &str, path: &str) -> Vec<String> {
    let (status, _, body) = request(addr, method, path, &[]).await;
    assert_eq!(status, 200, "{method} {path}: {body}");
    taken(trace)
}

#[tokio::test]
async fn a_controller_middleware_scoped_with_only_runs_on_those_actions_alone() {
    let auth = trace();
    let router = Router::new()
        .resource(
            "h2only",
            Ctl(vec![
                ControllerMiddleware::new(Auth(auth.clone())).only(&[ResourceAction::Store]),
            ]),
        )
        .unnamed()
        .register();
    let addr = server(router).await;

    assert!(ran(addr, &auth, "GET", "/h2only").await.is_empty());
    assert_eq!(ran(addr, &auth, "POST", "/h2only").await, ["auth"]);
    for (method, path) in [
        ("GET", "/h2only/create"),
        ("GET", "/h2only/1"),
        ("GET", "/h2only/1/edit"),
        ("PUT", "/h2only/1"),
        ("PATCH", "/h2only/1"),
        ("DELETE", "/h2only/1"),
    ] {
        assert!(
            ran(addr, &auth, method, path).await.is_empty(),
            "{method} {path} is not `store`"
        );
    }
}

#[tokio::test]
async fn except_leaves_the_listed_actions_out_and_update_covers_put_and_patch() {
    let auth = trace();
    let router = Router::new()
        .api_resource(
            "h2except",
            Ctl(vec![
                ControllerMiddleware::new(Auth(auth.clone())).except(&[ResourceAction::Index]),
            ]),
        )
        .unnamed()
        .register();
    let addr = server(router).await;

    assert!(ran(addr, &auth, "GET", "/h2except").await.is_empty());
    for (method, path) in [
        ("POST", "/h2except"),
        ("GET", "/h2except/1"),
        ("PUT", "/h2except/1"),
        ("PATCH", "/h2except/1"),
        ("DELETE", "/h2except/1"),
    ] {
        assert_eq!(
            ran(addr, &auth, method, path).await,
            ["auth"],
            "{method} {path}"
        );
    }
}

#[tokio::test]
async fn only_and_except_together_and_an_empty_only() {
    let auth = trace();
    let never = trace();
    let router = Router::new()
        .api_resource(
            "h2both",
            Ctl(vec![
                // Laravel keeps an action that `only` lists and `except` does not.
                ControllerMiddleware::new(Auth(auth.clone()))
                    .only(&[ResourceAction::Show, ResourceAction::Update])
                    .except(&[ResourceAction::Update]),
                // And `only([])` runs on no action at all.
                ControllerMiddleware::new(Auth(never.clone())).only(&[]),
            ]),
        )
        .unnamed()
        .register();
    let addr = server(router).await;

    assert_eq!(ran(addr, &auth, "GET", "/h2both/1").await, ["auth"]);
    assert!(ran(addr, &auth, "PUT", "/h2both/1").await.is_empty());
    assert!(ran(addr, &auth, "PATCH", "/h2both/1").await.is_empty());
    assert!(ran(addr, &auth, "GET", "/h2both").await.is_empty());
    assert!(taken(&never).is_empty(), "`only(&[])` ran on an action");
}

#[tokio::test]
async fn an_alias_in_the_list_is_resolved_with_its_arguments_and_a_group_name_too() {
    let tags = trace();
    register_tagged("h2-alias-tag", &tags);
    let plain = tags.clone();
    register_middleware_alias("h2-alias-plain", move || Registered(plain.clone()));
    register_middleware_group(
        "h2-alias-group",
        ["h2-alias-tag:g".to_string(), "h2-alias-plain".to_string()],
    );
    let router = Router::new()
        .api_resource(
            "h2alias",
            Ctl(vec![
                ControllerMiddleware::named("h2-alias-tag:60, 1").only(&[ResourceAction::Index]),
                ControllerMiddleware::named("h2-alias-group").only(&[ResourceAction::Show]),
            ]),
        )
        .unnamed()
        .register();
    let addr = server(router).await;

    assert_eq!(
        ran(addr, &tags, "GET", "/h2alias").await,
        ["h2-alias-tag:60,1"]
    );
    assert_eq!(
        ran(addr, &tags, "GET", "/h2alias/1").await,
        ["h2-alias-tag:g", "registered"]
    );
    assert!(ran(addr, &tags, "POST", "/h2alias").await.is_empty());
}

#[test]
fn an_unknown_alias_fails_the_registration() {
    let error = match Router::new()
        .resource(
            "h2unknown",
            Ctl(vec![ControllerMiddleware::named("h2-never-registered")]),
        )
        .unnamed()
        .try_register()
    {
        Ok(_) => panic!("a resource whose controller names an unknown alias registered"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("h2-never-registered"),
        "the error names the alias: {error}"
    );

    let error = match resource!("h2unknowndef", plain_posts, except = [create, edit])
        .unnamed()
        .middleware(ControllerMiddleware::named("h2-never-registered-either"))
        .try_register(Router::new())
    {
        Ok(_) => panic!("a resource given an unknown alias registered"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("h2-never-registered-either"));
}

#[test]
#[should_panic(expected = "h2-panics-at-boot")]
fn register_panics_on_an_unknown_alias() {
    let _ = Router::new()
        .resource(
            "h2panics",
            Ctl(vec![ControllerMiddleware::named("h2-panics-at-boot")]),
        )
        .unnamed()
        .register();
}

#[tokio::test]
async fn resource_routes_middleware_scopes_as_the_controllers_list_and_runs_before_it() {
    let tags = trace();
    let router = Router::new()
        .api_resource(
            "h2registered",
            Ctl(vec![
                ControllerMiddleware::new(Declared(tags.clone())).only(&[ResourceAction::Store]),
            ]),
        )
        .middleware(
            ControllerMiddleware::new(Registered(tags.clone())).only(&[ResourceAction::Store]),
        )
        .unnamed()
        .register();
    let addr = server(router).await;

    assert!(ran(addr, &tags, "GET", "/h2registered").await.is_empty());
    assert!(ran(addr, &tags, "GET", "/h2registered/1").await.is_empty());
    assert_eq!(
        ran(addr, &tags, "POST", "/h2registered").await,
        ["registered", "declared"]
    );
}

/// A resource module that declares no middleware.
pub mod plain_posts {
    use super::*;

    pub async fn index(_req: Request) -> Response {
        text("index")
    }
    pub async fn store(_req: Request) -> Response {
        text("store")
    }
    pub async fn show(_req: Request) -> Response {
        text("show")
    }
    pub async fn update(_req: Request) -> Response {
        text("update")
    }
    pub async fn destroy(_req: Request) -> Response {
        text("destroy")
    }
}

/// What the middleware `declaring_posts` declares writes to.
static DECLARING: LazyLock<Trace> = LazyLock::new(trace);

/// A resource module that declares its middleware, as `HasMiddleware` does.
pub mod declaring_posts {
    use super::*;

    pub fn middleware() -> Vec<ControllerMiddleware> {
        vec![ControllerMiddleware::new(Declared(DECLARING.clone())).only(&[ResourceAction::Show])]
    }

    pub async fn index(_req: Request) -> Response {
        text("index")
    }
    pub async fn show(_req: Request) -> Response {
        text("show")
    }
}

#[tokio::test]
async fn resource_macro_picks_up_the_modules_pub_fn_middleware() {
    let router = resource!("h2declaring", declaring_posts, only = [index, show])
        .unnamed()
        .register(Router::new());
    let addr = server(router).await;

    assert!(
        ran(addr, &DECLARING, "GET", "/h2declaring")
            .await
            .is_empty()
    );
    assert_eq!(
        ran(addr, &DECLARING, "GET", "/h2declaring/1").await,
        ["declared"]
    );
}

#[tokio::test]
async fn resource_def_middleware_scopes_the_same_way_on_a_module_without_one() {
    let auth = trace();
    let router = resource!("h2def", plain_posts, except = [create, edit])
        .middleware(ControllerMiddleware::new(Auth(auth.clone())).except(&[ResourceAction::Index]))
        .unnamed()
        .register(Router::new());
    let addr = server(router).await;

    assert!(ran(addr, &auth, "GET", "/h2def").await.is_empty());
    assert_eq!(ran(addr, &auth, "GET", "/h2def/1").await, ["auth"]);
    assert_eq!(ran(addr, &auth, "PATCH", "/h2def/1").await, ["auth"]);
}

/// What the middleware of `grouped_posts` and the group around it write to.
static GROUPED: LazyLock<Trace> = LazyLock::new(trace);

/// A resource module registered inside a group.
pub mod grouped_posts {
    use super::*;

    pub fn middleware() -> Vec<ControllerMiddleware> {
        vec![ControllerMiddleware::new(Declared(GROUPED.clone()))]
    }

    pub async fn index(_req: Request) -> Response {
        text("index")
    }
    pub async fn show(_req: Request) -> Response {
        text("show")
    }
}

#[tokio::test]
async fn the_controllers_middleware_runs_after_the_groups_and_the_registrations() {
    let router = group!("/h2-admin", {
        resource!("h2grouped", grouped_posts, only = [index, show])
            .middleware(ControllerMiddleware::new(Registered(GROUPED.clone())).only(&[ResourceAction::Show])),
    })
    .middleware(GroupGate(GROUPED.clone()))
    .name("h2admin.")
    .register(Router::new());
    let addr = server(router).await;

    assert_eq!(
        ran(addr, &GROUPED, "GET", "/h2-admin/h2grouped/7").await,
        ["group", "registered", "declared"]
    );
    assert_eq!(
        ran(addr, &GROUPED, "GET", "/h2-admin/h2grouped").await,
        ["group", "declared"]
    );
    // The group's prefix and name prefix reach the resource's routes.
    assert_eq!(
        route("h2admin.h2grouped.show", &[("h2grouped", "7")]).as_deref(),
        Some("/h2-admin/h2grouped/7")
    );
    let (status, _, _) = request(addr, "GET", "/h2grouped/7", &[]).await;
    assert_eq!(
        status, 404,
        "the resource lives under the group's prefix only"
    );
}

async fn ok(_req: Request) -> Response {
    text("ok")
}

#[tokio::test]
async fn a_route_leaves_out_its_groups_middleware_by_type_and_keeps_the_global() {
    let group_json = trace();
    let global_json = trace();
    let router = group!("/h2-json", {
        get!("/plain", ok).without_middleware::<EnsureJson>(),
        get!("/kept", ok),
    })
    .middleware(EnsureJson(group_json.clone()))
    .middleware(GroupGate(group_json.clone()))
    .register(Router::new());
    let addr = server_with(
        router,
        MiddlewareRegistry::new().append(EnsureJson(global_json.clone())),
    )
    .await;

    assert_eq!(
        ran(addr, &group_json, "GET", "/h2-json/plain").await,
        ["group"],
        "the route left the group's EnsureJson out and kept the rest"
    );
    assert_eq!(
        taken(&global_json),
        ["json"],
        "the global EnsureJson still ran"
    );
    assert_eq!(
        ran(addr, &group_json, "GET", "/h2-json/kept").await,
        ["json", "group"],
        "the sibling route still runs the group's EnsureJson"
    );
    assert_eq!(taken(&global_json), ["json"]);
}

#[tokio::test]
async fn a_route_leaves_out_an_alias_with_its_arguments_or_a_whole_group() {
    let tags = trace();
    register_tagged("h2-named-tag", &tags);
    let plain = tags.clone();
    register_middleware_alias("h2-named-json", move || EnsureJson(plain.clone()));
    register_middleware_group(
        "h2-named-bundle",
        ["h2-named-tag:b".to_string(), "h2-named-json".to_string()],
    );
    let router = group!("/h2-named", {
        get!("/one", ok).without_middleware_named("h2-named-tag:1"),
        get!("/json", ok).without_middleware_named("h2-named-json"),
        get!("/bundle", ok).without_middleware_named("h2-named-bundle"),
        get!("/all", ok),
    })
    .middleware_named("h2-named-tag:1")
    .middleware_named("h2-named-tag:2")
    .middleware_named("h2-named-bundle")
    .register(Router::new());
    let addr = server(router).await;

    assert_eq!(
        ran(addr, &tags, "GET", "/h2-named/one").await,
        ["h2-named-tag:2", "h2-named-tag:b", "json"],
        "leaving out `h2-named-tag:1` keeps `h2-named-tag:2`"
    );
    assert_eq!(
        ran(addr, &tags, "GET", "/h2-named/json").await,
        ["h2-named-tag:1", "h2-named-tag:2", "h2-named-tag:b"]
    );
    assert_eq!(
        ran(addr, &tags, "GET", "/h2-named/bundle").await,
        ["h2-named-tag:1", "h2-named-tag:2"],
        "a group name leaves out every middleware of the group"
    );
    assert_eq!(
        ran(addr, &tags, "GET", "/h2-named/all").await,
        ["h2-named-tag:1", "h2-named-tag:2", "h2-named-tag:b", "json"]
    );
}

#[tokio::test]
async fn the_fluent_route_and_an_any_route_leave_middleware_out() {
    let json = trace();
    let router: Router = Router::new()
        .get("/h2-fluent", ok)
        .middleware(EnsureJson(json.clone()))
        .middleware(GroupGate(json.clone()))
        .without_middleware::<EnsureJson>()
        .into();
    let router = group!("/h2-any", {
        any!("/out", ok).without_middleware::<EnsureJson>(),
        any!("/in", ok),
    })
    .middleware(EnsureJson(json.clone()))
    .register(router);
    let addr = server(router).await;

    assert_eq!(ran(addr, &json, "GET", "/h2-fluent").await, ["group"]);
    for method in ["GET", "POST", "PUT", "PATCH", "DELETE", "QUERY"] {
        assert!(
            ran(addr, &json, method, "/h2-any/out").await.is_empty(),
            "{method} left EnsureJson out"
        );
        assert_eq!(ran(addr, &json, method, "/h2-any/in").await, ["json"]);
    }
}

#[test]
fn an_unknown_name_cannot_be_left_out() {
    let error = match get!("/h2-unknown-out", ok).try_without_middleware_named("h2-no-such-name") {
        Ok(_) => panic!("leaving out an unregistered name was accepted"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("h2-no-such-name"), "{error}");

    let error = match Router::new()
        .get("/h2-unknown-fluent", ok)
        .try_without_middleware_named("h2-no-such-name-either")
    {
        Ok(_) => panic!("leaving out an unregistered name was accepted"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("h2-no-such-name-either"));
}
