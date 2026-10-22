//! The markers describe a whole struct; on a field they fail the build.

use suprnova::InertiaProps;

#[derive(InertiaProps)]
pub struct AppShared {
    #[inertia_props(shared)]
    pub app_name: String,
}

fn main() {
    let _ = AppShared {
        app_name: String::new(),
    };
}
