use serde::Serialize;
use suprnova::InertiaProps;

#[derive(Serialize)]
pub struct Toast {
    pub kind: String,
    pub message: String,
}

#[derive(InertiaProps, Serialize)]
#[inertia_props(flash)]
pub struct Flash {
    pub toast: Option<Toast>,
}
