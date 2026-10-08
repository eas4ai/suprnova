//! PAR-072 and PAR-073: an entry names the route that matched, by its
//! method as well as its pattern, and records the response the client
//! got when the page's document fails to render.

use suprnova::{HttpResponse, InertiaResponse, Redirect, Request, Response, Router};

use super::{client, devtools, entry_of};

fn router() -> Router {
    Router::new()
        .get("/indt-members", |req: Request| async move {
            InertiaResponse::new("Members")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .name("indt.members.index")
        .post("/indt-members", |_req: Request| async {
            let response: Response = Redirect::to("/indt-members").into();
            response
        })
        .name("indt.members.store")
        .into()
}

#[tokio::test]
async fn indt_each_method_on_one_path_is_recorded_with_its_own_route_name() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));

    let index = client.get("/indt-members").inertia().send().await;
    index.assert_ok();
    let store = client.post("/indt-members").inertia().send().await;
    store.assert_status(302);

    let index = entry_of(dir.path(), &index);
    let store = entry_of(dir.path(), &store);
    assert_eq!(
        index["route"]["name"], "indt.members.index",
        "{}",
        index["route"]
    );
    assert_eq!(
        store["route"]["name"], "indt.members.store",
        "{}",
        store["route"]
    );
    assert_eq!(index["route"]["uri"], "/indt-members");
    assert_eq!(store["route"]["uri"], "/indt-members");
}
