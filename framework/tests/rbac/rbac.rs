use std::any::Any;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use serial_test::serial;
use std::convert::Infallible;
use suprnova::auth::request_state::request_state_scope_for_test;
use suprnova::rbac::migrations::CreateRbacTables;
use suprnova::testing::TestDatabase;
use suprnova::{
    Auth, Authenticatable, DB, FrameworkError, HasRoles, HttpResponse, Middleware, Next,
    PermissionMiddleware, Request, RoleMiddleware,
};

#[derive(Clone)]
struct User {
    id: i64,
}

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl HasRoles for User {}

struct TestMigrator;

impl sea_orm_migration::MigratorTrait for TestMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![Box::new(CreateRbacTables)]
    }
}

async fn setup() -> TestDatabase {
    let db = TestDatabase::fresh::<TestMigrator>().await.unwrap();
    suprnova::rbac::create_role("author").await.unwrap();
    suprnova::rbac::create_permission("articles.create")
        .await
        .unwrap();
    suprnova::rbac::create_permission("articles.publish")
        .await
        .unwrap();
    suprnova::rbac::give_permission_to_role("author", "articles.create")
        .await
        .unwrap();
    // Use the model's own discriminator (the fully-qualified type name)
    // so the seeded assignment matches what the trait checks and the
    // Role/Permission middleware query at runtime.
    suprnova::rbac::assign_role_to_model(&User { id: 7 }.rbac_model_type(), "7", "author")
        .await
        .unwrap();
    db
}

async fn request(path: &str) -> Request {
    request_with_inertia(path, false).await
}

async fn inertia_request(path: &str) -> Request {
    request_with_inertia(path, true).await
}

async fn request_with_inertia(path: &str, inertia: bool) -> Request {
    let path = path.to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<Request>();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));

    let tx_for_service = tx.clone();
    let server = tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let service = service_fn(move |hyper_req: hyper::Request<Incoming>| {
                let tx = tx_for_service.clone();
                async move {
                    let req = Request::new(hyper_req);
                    if let Some(tx) = tx.lock().unwrap().take() {
                        let _ = tx.send(req);
                    }
                    Ok::<_, Infallible>(
                        hyper::Response::builder()
                            .status(200)
                            .body(Full::new(Bytes::new()))
                            .unwrap(),
                    )
                }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await;
        }
    });

    let client = tokio::spawn(async move {
        let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
                .await
                .unwrap();
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let mut req = hyper::Request::builder()
            .method("GET")
            .uri(path)
            .header("Host", "localhost");
        if inertia {
            req = req.header("X-Inertia", "true");
        }
        let req = req.body(Full::new(Bytes::new())).unwrap();
        let _ = sender.send_request(req).await.unwrap();
    });

    let req = rx.await.unwrap();
    client.await.unwrap();
    server.await.unwrap();
    req
}

fn next_ok() -> Next {
    Arc::new(|_req| Box::pin(async { Ok(HttpResponse::text("ok")) }))
}

#[tokio::test]
#[serial]
async fn has_roles_reads_role_inherited_permissions_and_direct_permissions() {
    let _db = setup().await;
    let user = User { id: 7 };

    assert!(user.has_role("author").await.unwrap());
    assert!(user.has_permission_to("articles.create").await.unwrap());
    assert!(!user.has_permission_to("articles.publish").await.unwrap());

    user.give_permission_to("articles.publish").await.unwrap();
    assert!(user.has_permission_to("articles.publish").await.unwrap());
}

#[tokio::test]
#[serial]
async fn missing_roles_and_permissions_deny_by_default() {
    let _db = setup().await;
    let user = User { id: 8 };

    assert!(!user.has_role("author").await.unwrap());
    assert!(!user.has_permission_to("articles.create").await.unwrap());
}

#[tokio::test]
#[serial]
async fn removing_a_role_takes_that_role_and_nothing_else() {
    let _db = setup().await;
    let user = User { id: 7 };
    let model_type = user.rbac_model_type();

    suprnova::rbac::give_permission_to_role("reviewer", "articles.review")
        .await
        .unwrap();
    suprnova::rbac::assign_role_to_model(&model_type, "7", "reviewer")
        .await
        .unwrap();
    user.give_permission_to("articles.publish").await.unwrap();

    suprnova::rbac::remove_role_from_model(&model_type, "7", "author")
        .await
        .unwrap();

    assert!(!user.has_role("author").await.unwrap());
    assert!(
        !user.has_permission_to("articles.create").await.unwrap(),
        "the removed role was this model's only source of that permission"
    );
    assert!(
        user.has_role("reviewer").await.unwrap(),
        "the model's other role is not named by this call and must survive"
    );
    assert!(user.has_permission_to("articles.review").await.unwrap());
    assert!(
        user.has_permission_to("articles.publish").await.unwrap(),
        "the model's direct grant is not named by this call either"
    );
}

/// The case that catches a revocation implemented as a blunt delete.
/// `has_permission_for_model` resolves a direct grant *or* a role-inherited
/// one, so taking the role away must leave the direct grant answering.
#[tokio::test]
#[serial]
async fn removing_a_role_leaves_a_permission_the_model_also_holds_directly() {
    let _db = setup().await;
    let user = User { id: 7 };
    let model_type = user.rbac_model_type();

    user.give_permission_to("articles.create").await.unwrap();

    suprnova::rbac::remove_role_from_model(&model_type, "7", "author")
        .await
        .unwrap();

    assert!(!user.has_role("author").await.unwrap());
    assert!(
        user.has_permission_to("articles.create").await.unwrap(),
        "the direct grant is a second, independent source and must survive"
    );

    // Ending effective access takes both calls, which is what the docs say.
    user.remove_permission_to("articles.create").await.unwrap();
    assert!(!user.has_permission_to("articles.create").await.unwrap());
}

#[tokio::test]
#[serial]
async fn removing_a_permission_from_a_role_spares_other_sources_and_other_permissions() {
    let _db = setup().await;
    let through_role = User { id: 7 };
    let directly = User { id: 8 };

    suprnova::rbac::give_permission_to_role("author", "articles.publish")
        .await
        .unwrap();
    directly
        .give_permission_to("articles.create")
        .await
        .unwrap();

    suprnova::rbac::remove_permission_from_role("author", "articles.create")
        .await
        .unwrap();

    assert!(
        through_role.has_role("author").await.unwrap(),
        "the role itself is not named by this call"
    );
    assert!(
        !through_role
            .has_permission_to("articles.create")
            .await
            .unwrap()
    );
    assert!(
        through_role
            .has_permission_to("articles.publish")
            .await
            .unwrap(),
        "the role's other permission is not named by this call"
    );
    assert!(
        directly.has_permission_to("articles.create").await.unwrap(),
        "a direct grant of the same permission is a separate source"
    );
}

/// Both guards hold the same role name assigned to the same model, so what
/// is under test is the delete's guard scoping rather than the lookup's.
#[tokio::test]
#[serial]
async fn removing_a_role_on_one_guard_leaves_the_same_name_on_the_other() {
    let _db = setup().await;
    let user = User { id: 7 };
    let model_type = user.rbac_model_type();

    for guard in ["web", "api"] {
        suprnova::rbac::give_permission_to_role_on_guard("admin", "users.manage", guard)
            .await
            .unwrap();
        suprnova::rbac::assign_role_to_model_on_guard(&model_type, "7", "admin", guard)
            .await
            .unwrap();
        assert!(
            suprnova::rbac::has_role_for_model_on_guard(&model_type, "7", "admin", guard)
                .await
                .unwrap(),
            "both guards must start from the same state for this test to mean anything"
        );
    }

    suprnova::rbac::remove_role_from_model_on_guard(&model_type, "7", "admin", "api")
        .await
        .unwrap();

    assert!(
        !suprnova::rbac::has_role_for_model_on_guard(&model_type, "7", "admin", "api")
            .await
            .unwrap()
    );
    assert!(
        suprnova::rbac::has_role_for_model_on_guard(&model_type, "7", "admin", "web")
            .await
            .unwrap(),
        "the web guard's membership is a different row and must survive"
    );
    assert!(
        suprnova::rbac::has_permission_for_model_on_guard(&model_type, "7", "users.manage", "web")
            .await
            .unwrap()
    );
    assert!(
        !suprnova::rbac::has_permission_for_model_on_guard(&model_type, "7", "users.manage", "api")
            .await
            .unwrap()
    );
}

#[tokio::test]
#[serial]
async fn removing_a_role_permission_on_one_guard_leaves_the_same_pair_on_the_other() {
    let _db = setup().await;
    let user = User { id: 7 };
    let model_type = user.rbac_model_type();

    for guard in ["web", "api"] {
        suprnova::rbac::give_permission_to_role_on_guard("admin", "users.manage", guard)
            .await
            .unwrap();
    }
    suprnova::rbac::assign_role_to_model_on_guard(&model_type, "7", "admin", "api")
        .await
        .unwrap();

    suprnova::rbac::remove_permission_from_role_on_guard("admin", "users.manage", "web")
        .await
        .unwrap();

    assert!(
        suprnova::rbac::has_permission_for_model_on_guard(&model_type, "7", "users.manage", "api")
            .await
            .unwrap(),
        "the api guard's grant is a different row and must survive"
    );
}

/// The decision: a name that exists, held by nobody, converges rather than
/// failing, so an incident responder's retry is safe.
#[tokio::test]
#[serial]
async fn removing_what_was_never_granted_succeeds_and_repeats_safely() {
    let _db = setup().await;
    let user = User { id: 7 };
    let other = User { id: 8 };
    let model_type = user.rbac_model_type();

    suprnova::rbac::assign_role_to_model(&model_type, "8", "author")
        .await
        .unwrap();
    suprnova::rbac::create_role("reviewer").await.unwrap();

    // The role exists; this model never held it.
    suprnova::rbac::remove_role_from_model(&model_type, "7", "reviewer")
        .await
        .unwrap();
    // The permission exists; this model was never given it directly.
    user.remove_permission_to("articles.publish").await.unwrap();
    // And the same revocation twice, which is what a retry looks like.
    user.remove_role("author").await.unwrap();
    user.remove_role("author").await.unwrap();

    assert!(!user.has_role("author").await.unwrap());
    assert!(
        other.has_role("author").await.unwrap(),
        "another model's membership is not named by any of those calls"
    );
}

/// The other half of the decision: a name that exists on no such guard is a
/// caller mistake - a typo, or a call aimed at the wrong guard - and is
/// refused rather than reported as a successful revocation.
#[tokio::test]
#[serial]
async fn a_name_that_exists_on_no_such_guard_is_refused_and_changes_nothing() {
    let _db = setup().await;
    let user = User { id: 7 };
    let model_type = user.rbac_model_type();

    let error = suprnova::rbac::remove_role_from_model(&model_type, "7", "nosuchrole")
        .await
        .expect_err("a role name that names nothing must not report success");
    let message = error.to_string();
    assert!(
        !message.contains("nosuchrole"),
        "an error never carries a role name: {message}"
    );
    assert!(
        !message.contains(&model_type),
        "an error never carries a model identity: {message}"
    );

    // "author" exists on "web" only, so this is the wrong-guard mistake.
    suprnova::rbac::remove_role_from_model_on_guard(&model_type, "7", "author", "api")
        .await
        .expect_err("a call aimed at the wrong guard must not report success");

    let error = suprnova::rbac::remove_permission_from_role("author", "nosuchpermission")
        .await
        .expect_err("a permission name that names nothing must not report success");
    assert!(
        !error.to_string().contains("nosuchpermission"),
        "an error never carries a permission name"
    );

    let error = user
        .remove_permission_to("nosuchpermission")
        .await
        .expect_err("the trait method refuses on the same terms");
    assert!(!error.to_string().contains("nosuchpermission"));

    // Access after four refusals is exactly what it was before them.
    assert!(user.has_role("author").await.unwrap());
    assert!(user.has_permission_to("articles.create").await.unwrap());
}

/// A revocation that fails leaves access exactly as it was, on the path
/// where the failure comes after the delete already landed.
#[tokio::test]
#[serial]
async fn a_revocation_rolled_back_with_its_transaction_leaves_access_intact() {
    let _db = setup().await;
    let user = User { id: 7 };
    let inside = user.clone();

    let outcome: Result<(), FrameworkError> = DB::transaction(move |_tx| {
        Box::pin(async move {
            inside.remove_role("author").await?;
            Err(FrameworkError::internal(
                "the bundle failed after the revocation landed",
            ))
        })
    })
    .await;

    assert!(outcome.is_err(), "the transaction must surface the failure");
    assert!(
        user.has_role("author").await.unwrap(),
        "a rolled-back revocation leaves the membership exactly where it was"
    );
    assert!(user.has_permission_to("articles.create").await.unwrap());
}

/// The application team's case: one role granted and another taken away in
/// the same step. The helpers resolve their executor through the
/// transaction in scope, so the bundle commits as one unit.
#[tokio::test]
#[serial]
async fn a_grant_and_a_revocation_commit_together_in_one_transaction() {
    let _db = setup().await;
    let user = User { id: 7 };
    let inside = user.clone();

    DB::transaction(move |_tx| {
        Box::pin(async move {
            inside.assign_role("reviewer").await?;
            inside.remove_role("author").await?;
            Ok::<(), FrameworkError>(())
        })
    })
    .await
    .unwrap();

    assert!(user.has_role("reviewer").await.unwrap());
    assert!(!user.has_role("author").await.unwrap());
}

#[tokio::test]
#[serial]
async fn middleware_allows_matching_role_and_permission() {
    let _db = setup().await;

    request_state_scope_for_test(async {
        Auth::set_user(Arc::new(User { id: 7 }));

        let role_response = match RoleMiddleware::<User>::new("author")
            .handle(request("/author").await, next_ok())
            .await
        {
            Ok(response) => response,
            Err(response) => panic!(
                "expected role middleware to pass, got {}",
                response.status_code()
            ),
        };
        assert_eq!(role_response.status_code(), 200);

        let permission_response = match PermissionMiddleware::<User>::new("articles.create")
            .handle(request("/articles").await, next_ok())
            .await
        {
            Ok(response) => response,
            Err(response) => panic!(
                "expected permission middleware to pass, got {}",
                response.status_code()
            ),
        };
        assert_eq!(permission_response.status_code(), 200);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn middleware_returns_forbidden_when_permission_is_missing() {
    let _db = setup().await;

    request_state_scope_for_test(async {
        Auth::set_user(Arc::new(User { id: 7 }));

        let response = match PermissionMiddleware::<User>::new("articles.delete")
            .handle(request("/articles/delete").await, next_ok())
            .await
        {
            Ok(response) => panic!(
                "expected permission middleware to deny, got {}",
                response.status_code()
            ),
            Err(response) => response,
        };
        assert_eq!(response.status_code(), 403);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn middleware_redirects_browser_denials_when_configured() {
    let _db = setup().await;

    request_state_scope_for_test(async {
        let response = match RoleMiddleware::<User>::redirect_to("admin", "/login")
            .handle(request("/admin").await, next_ok())
            .await
        {
            Ok(response) => panic!(
                "expected role middleware to deny, got {}",
                response.status_code()
            ),
            Err(response) => response,
        };
        assert_eq!(response.status_code(), 302);
        assert_eq!(response.header_value("Location"), Some("/login"));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn middleware_redirects_inertia_denials_with_conflict_location() {
    let _db = setup().await;

    request_state_scope_for_test(async {
        let response = match PermissionMiddleware::<User>::redirect_to("articles.delete", "/login")
            .handle(inertia_request("/articles/delete").await, next_ok())
            .await
        {
            Ok(response) => panic!(
                "expected permission middleware to deny, got {}",
                response.status_code()
            ),
            Err(response) => response,
        };
        assert_eq!(response.status_code(), 409);
        assert_eq!(response.header_value("X-Inertia-Location"), Some("/login"));
    })
    .await;
}
