//! Suprnova Gate adaptation for registered Live actions.

use suprnova_live::action::{
    ActionAuthorizationPort, ActionAuthorizationRequest, ActionFuture, AuthorizationDecision,
};

pub(crate) struct SuprnovaActionAuthorization;

impl ActionAuthorizationPort for SuprnovaActionAuthorization {
    fn authorize<'a>(
        &'a self,
        request: ActionAuthorizationRequest<'a>,
    ) -> ActionFuture<'a, Result<AuthorizationDecision, suprnova_live::action::ActionError>> {
        let ability = format!(
            "live:{}.{}",
            request.component().as_str(),
            request.action().as_str()
        );
        let resource = format!(
            "{}::{}",
            request.component().as_str(),
            request.action().as_str()
        );
        Box::pin(async move {
            let Some(principal) = super::route_principal().await else {
                return Ok(AuthorizationDecision::Deny);
            };
            let allowed =
                crate::authorization::Gate::allows_async(&ability, &principal, &resource).await;
            Ok(if allowed {
                AuthorizationDecision::Allow
            } else {
                AuthorizationDecision::Deny
            })
        })
    }
}

// `TestContainer::fake` belongs to the `testing` feature, which the minimal
// profile checked by scripts/check-feature-matrix.sh leaves off.
#[cfg(all(test, feature = "testing"))]
mod tests {
    use std::sync::{Arc, Mutex};

    use suprnova_live::identity::{ActionName, ComponentName};

    use super::*;
    use crate::auth::{Auth, AuthConfig, AuthManager, Authenticatable, GuardConfig, UserProvider};
    use crate::container::testing::TestContainer;
    use crate::error::FrameworkError;

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

    /// The principals the action Gate below was asked about.
    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Signs `(guard, id)` pairs in, names the route's guard as an
    /// `AuthMiddleware` that passed the request on would, and asks the action
    /// port to authorize one registered action.
    async fn authorize_as(
        signed_in: &[(&'static str, &'static str)],
        route_guard: Option<&str>,
    ) -> AuthorizationDecision {
        let component = ComponentName::parse("tests.route-guard-action").expect("component");
        let action = ActionName::parse("publish").expect("action");
        crate::auth::request_state::scope(async {
            for &(guard, id) in signed_in {
                Auth::guard(guard)
                    .unwrap()
                    .set_user(Arc::new(Named(id)))
                    .await;
            }
            crate::auth::request_state::set_route_guard(route_guard.map(str::to_owned));
            SuprnovaActionAuthorization
                .authorize(ActionAuthorizationRequest::for_test(&component, &action))
                .await
                .expect("the port answers")
        })
        .await
    }

    // A Live action on a route behind a second guard is authorized for that
    // guard's principal, `admin:9`; the default guard's user in the same
    // session never stands in for it. A route of the default guard keeps
    // the bare id.
    #[tokio::test]
    async fn an_action_is_authorized_for_the_route_guards_principal() {
        let _scope = TestContainer::fake();
        let config = AuthConfig::new("web").guard("admin", GuardConfig::session("admins"));
        TestContainer::singleton(AuthManager::new(config));
        Auth::register_provider("users", Arc::new(NoUsers)).unwrap();
        Auth::register_provider("admins", Arc::new(NoUsers)).unwrap();
        crate::authorization::Gate::define::<String, String>(
            "live:tests.route-guard-action.publish",
            |principal, _| {
                SEEN.lock().unwrap().push(principal.clone());
                true
            },
        );
        let seen = || std::mem::take(&mut *SEEN.lock().unwrap());

        let both = [("web", "7"), ("admin", "9")];
        let decision = authorize_as(&both, Some("admin")).await;
        assert_eq!(decision, AuthorizationDecision::Allow);
        assert_eq!(seen(), vec!["admin:9".to_owned()]);

        let refused = authorize_as(&[("web", "7")], Some("admin")).await;
        assert_eq!(refused, AuthorizationDecision::Deny);
        assert!(seen().is_empty(), "web user 7 never reaches the Gate");

        let decision = authorize_as(&both, None).await;
        assert_eq!(decision, AuthorizationDecision::Allow);
        assert_eq!(seen(), vec!["7".to_owned()]);
    }
}
