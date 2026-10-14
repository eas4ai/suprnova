//! Password reset pages.
//!
//! `/forgot-password` mounts the form that asks for a link, and
//! `/reset-password` the form that takes the new password; their actions
//! are in `live::forgot_password_form` and `live::reset_password_form`.
//! Both pages are for someone who cannot sign in.

use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveDocument, MountFlags};
use suprnova::{FrameworkError, HttpResponse, Request, Response, handler};

use super::auth::no_parameters;
use crate::live::mounts;
use crate::views::{self, ForgotPasswordPage, Frame, Nav, ResetPasswordPage};

/// `GET /forgot-password`: the form that asks for the address.
#[handler]
pub async fn forgot(req: Request) -> Response {
    Ok(forgot_page(&req).await?)
}

async fn forgot_page(req: &Request) -> Result<HttpResponse, FrameworkError> {
    let mut document = LiveDocument::from_request(req)?;
    let form = document
        .mount(
            &mounts()?.forgot_password,
            no_parameters(),
            MountFlags::empty(),
        )
        .await?;
    let frame = Frame::live(&mut document, "Forgot your password?", Nav::None).await?;
    let page = ForgotPasswordPage {
        frame,
        form: form.html().clone(),
    };
    views::render(document, "live/pages/forgot-password.html", &page)
}

/// `GET /reset-password?token=...`: the form that takes the new password.
/// The token from the mailed link becomes the form's mount parameter.
#[handler]
pub async fn reset(req: Request) -> Response {
    Ok(reset_page(&req).await?)
}

async fn reset_page(req: &Request) -> Result<HttpResponse, FrameworkError> {
    let token = req.query_param("token").unwrap_or_default();
    let parameters = CanonicalValue::Object(BTreeMap::from([(
        "token".to_owned(),
        CanonicalValue::String(token),
    )]));
    let mut document = LiveDocument::from_request(req)?;
    let form = document
        .mount(&mounts()?.reset_password, parameters, MountFlags::empty())
        .await?;
    let frame = Frame::live(&mut document, "Reset your password", Nav::None).await?;
    let page = ResetPasswordPage {
        frame,
        form: form.html().clone(),
    };
    views::render(document, "live/pages/reset-password.html", &page)
}
