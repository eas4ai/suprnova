//! A marker other than `shared` or `flash` fails the build and names the
//! two the derive accepts.

use suprnova::InertiaProps;

#[derive(InertiaProps)]
#[inertia_props(global)]
pub struct AppShared {
    pub app_name: String,
}

fn main() {
    let _ = AppShared {
        app_name: String::new(),
    };
}
