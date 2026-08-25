use suprnova::{handler, inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps)]
pub struct HomeProps {
    /// The app name, interpolated into the frontend's
    /// `welcome = Welcome to { $app }!` Fluent message (see
    /// `lang/en/app.ftl`) as `$app` - not a full sentence. The page
    /// builds the actual translated headline via `t('welcome', { app: title })`.
    pub title: String,
    pub message: String,
}

#[handler]
pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Suprnova".to_string(),
        message: "Your Inertia app is ready.".to_string(),
    })
}
