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
        let Some(principal) = crate::auth::guard::Auth::id() else {
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
            let Some(principal) = crate::auth::guard::Auth::id() else {
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
