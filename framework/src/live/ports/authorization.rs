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
            let Some(principal) = crate::auth::guard::Auth::id() else {
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
