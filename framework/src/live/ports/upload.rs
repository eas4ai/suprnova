//! Current Suprnova authorization for registered upload fields.

use suprnova_live::identity::{ComponentName, ModelField};
use suprnova_live::upload::{
    UploadAuthorizationDecision, UploadAuthorizationPort, UploadAuthorizationRequest, UploadError,
    UploadErrorKind, UploadFuture,
};

pub(crate) struct SuprnovaUploadAuthorization;

impl SuprnovaUploadAuthorization {
    pub(crate) async fn authorize_registered(
        &self,
        component: &ComponentName,
        field: &ModelField,
        control: suprnova_live::upload::UploadControlKind,
    ) -> Result<(), UploadError> {
        let ability = ability(component, field, control);
        let resource = resource(component, field);
        let Some(principal) = super::route_principal().await else {
            return Err(UploadError::new(UploadErrorKind::AuthorizationDenied));
        };
        if crate::authorization::Gate::allows_async(&ability, &principal, &resource).await {
            Ok(())
        } else {
            Err(UploadError::new(UploadErrorKind::AuthorizationDenied))
        }
    }
}

impl UploadAuthorizationPort for SuprnovaUploadAuthorization {
    fn authorize<'a>(
        &'a self,
        request: UploadAuthorizationRequest<'a>,
    ) -> UploadFuture<'a, Result<UploadAuthorizationDecision, UploadError>> {
        let ability = ability(request.component(), request.field(), request.control());
        let resource = resource(request.component(), request.field());
        Box::pin(async move {
            let Some(principal) = super::route_principal().await else {
                return Ok(UploadAuthorizationDecision::Deny);
            };
            let allowed =
                crate::authorization::Gate::allows_async(&ability, &principal, &resource).await;
            Ok(if allowed {
                UploadAuthorizationDecision::Allow
            } else {
                UploadAuthorizationDecision::Deny
            })
        })
    }
}

fn ability(
    component: &ComponentName,
    field: &ModelField,
    control: suprnova_live::upload::UploadControlKind,
) -> String {
    format!(
        "live:{}.upload.{}.{control:?}",
        component.as_str(),
        field.as_str(),
    )
}

fn resource(component: &ComponentName, field: &ModelField) -> String {
    format!("{}::{}", component.as_str(), field.as_str())
}

// `TestContainer::fake` belongs to the `testing` feature, which the minimal
// profile checked by scripts/check-feature-matrix.sh leaves off.
#[cfg(all(test, feature = "testing"))]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::auth::{Auth, AuthConfig, AuthManager, Authenticatable, GuardConfig, UserProvider};
    use crate::container::testing::TestContainer;
    use crate::error::FrameworkError;
    use suprnova_live::upload::UploadControlKind;

    /// A user known by its id alone.
    struct Named(&'static str);

    impl Authenticatable for Named {
        fn get_auth_identifier(&self) -> String {
            self.0.to_owned()
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn into_arc_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
            self
        }
    }

    /// Resolves nobody: the users below are signed in through `set_user`.
    struct NoUsers;

    #[async_trait::async_trait]
    impl UserProvider for NoUsers {
        async fn retrieve_by_id(
            &self,
            _id: &str,
        ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
            Ok(None)
        }
    }

    /// The principals the upload Gate below was asked about.
    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Signs `(guard, id)` pairs in, names the route's guard as an
    /// `AuthMiddleware` that passed the request on would, and asks the
    /// upload port to authorize one control.
    async fn authorize_as(
        signed_in: &[(&'static str, &'static str)],
        route_guard: Option<&str>,
    ) -> Result<(), UploadError> {
        let component = ComponentName::parse("tests.route-guard-upload").expect("component");
        let field = ModelField::parse("avatar").expect("field");
        crate::auth::request_state::scope(async {
            for &(guard, id) in signed_in {
                Auth::guard(guard)
                    .unwrap()
                    .set_user(Arc::new(Named(id)))
                    .await;
            }
            crate::auth::request_state::set_route_guard(route_guard.map(str::to_owned));
            SuprnovaUploadAuthorization
                .authorize_registered(&component, &field, UploadControlKind::Create)
                .await
        })
        .await
    }

    // An upload on a route behind a second guard is authorized for that
    // guard's principal, `admin:9`; the default guard's user in the same
    // session never stands in for it. A route of the default guard keeps
    // the bare id.
    #[tokio::test]
    async fn an_upload_is_authorized_for_the_route_guards_principal() {
        let _scope = TestContainer::fake();
        let config = AuthConfig::new("web").guard("admin", GuardConfig::session("admins"));
        TestContainer::singleton(AuthManager::new(config));
        Auth::register_provider("users", Arc::new(NoUsers)).unwrap();
        Auth::register_provider("admins", Arc::new(NoUsers)).unwrap();
        crate::authorization::Gate::define::<String, String>(
            "live:tests.route-guard-upload.upload.avatar.Create",
            |principal, _| {
                SEEN.lock().unwrap().push(principal.clone());
                true
            },
        );
        let seen = || std::mem::take(&mut *SEEN.lock().unwrap());

        let both = [("web", "7"), ("admin", "9")];
        authorize_as(&both, Some("admin")).await.unwrap();
        assert_eq!(seen(), vec!["admin:9".to_owned()]);

        let refused = authorize_as(&[("web", "7")], Some("admin")).await;
        assert!(
            refused.is_err(),
            "no user on the route's guard is a refusal"
        );
        assert!(seen().is_empty(), "web user 7 never reaches the Gate");

        authorize_as(&both, None).await.unwrap();
        assert_eq!(seen(), vec!["7".to_owned()]);
    }
}
