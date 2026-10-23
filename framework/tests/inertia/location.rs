//! PAR-049: `location` answers as the request needs - `409` with
//! `X-Inertia-Location` to an Inertia visit, a `302` redirect to anything
//! else - and takes a URL or a redirect. Laravel's reference is
//! `ResponseFactory::location` in inertia-laravel 3.5.1.

use suprnova::{
    Inertia, InertiaHeadersMiddleware, InertiaResponse, MiddlewareRegistry, Redirect, Request,
    Response, Router,
};

use crate::protocol_harness::{Client, serve};

fn router() -> Router {
    Router::new()
        .get("/out", |_req: Request| async {
            let response: Response = Ok(InertiaResponse::location("/x"));
            response
        })
        .get("/out-redirect", |_req: Request| async {
            let redirect = Redirect::away("https://billing.example/portal").status(303);
            let response: Response = Ok(Inertia::location(redirect));
            response
        })
        .into()
}

async fn inertia_stack() -> Client {
    Client::new(
        serve(
            router(),
            MiddlewareRegistry::new().append(InertiaHeadersMiddleware::new()),
        )
        .await,
    )
}

#[tokio::test]
async fn inp_location_answers_a_plain_visit_with_a_302() {
    // A hard navigation into an OAuth or SSO bounce carries no
    // `X-Inertia`; a bare 409 gives that browser nowhere to go.
    let reply = inertia_stack().await.send("GET", "/out", &[]).await;
    assert_eq!(reply.status, 302);
    assert_eq!(reply.header("location"), Some("/x"));
    assert_eq!(reply.header("x-inertia-location"), None);
}

#[tokio::test]
async fn inp_location_answers_an_inertia_visit_with_a_409() {
    let reply = inertia_stack()
        .await
        .send("GET", "/out", &[("X-Inertia", "true")])
        .await;
    assert_eq!(reply.status, 409);
    assert_eq!(reply.header("x-inertia-location"), Some("/x"));
}

#[tokio::test]
async fn inp_location_without_the_inertia_middleware_still_tells_the_visits_apart() {
    // The server scopes the visit's facts for every request, so a route
    // outside the Inertia stack answers as the request needs too: a plain
    // visit is never handed the 409 (PAR-049).
    let addr = serve(router(), MiddlewareRegistry::new()).await;
    let plain = Client::new(addr).send("GET", "/out", &[]).await;
    assert_eq!(plain.status, 302, "{plain:?}");
    assert_eq!(plain.header("location"), Some("/x"));
    let visit = Client::new(addr)
        .send("GET", "/out", &[("X-Inertia", "true")])
        .await;
    assert_eq!(visit.status, 409, "{visit:?}");
    assert_eq!(visit.header("x-inertia-location"), Some("/x"));
}

#[tokio::test]
async fn inp_location_takes_a_redirect_and_keeps_it_for_a_plain_visit() {
    // Laravel hands a plain visit the redirect itself, status included,
    // and an Inertia visit its target.
    let mut client = inertia_stack().await;
    let plain = client.send("GET", "/out-redirect", &[]).await;
    assert_eq!(plain.status, 303);
    assert_eq!(
        plain.header("location"),
        Some("https://billing.example/portal")
    );

    let visit = client
        .send("GET", "/out-redirect", &[("X-Inertia", "true")])
        .await;
    assert_eq!(visit.status, 409);
    assert_eq!(
        visit.header("x-inertia-location"),
        Some("https://billing.example/portal")
    );
    assert_eq!(visit.header("location"), None);
}
