//! BIND-015: an `#[authorize]` parameter target must bind or be a primitive
//! path value. A form request is neither, so this crate must fail to
//! compile.

use suprnova::{Response, handler, request};

#[request]
pub struct UpdatePost {
    pub title: String,
}

#[handler]
#[suprnova::authorize("update", form)]
pub async fn update(form: UpdatePost) -> Response {
    suprnova::http::text(form.title)
}
