//! One struct cannot be both the shared props and the flash data.

use suprnova::InertiaProps;

#[derive(InertiaProps)]
#[inertia_props(shared)]
#[inertia_props(flash)]
pub struct AppShared {
    pub app_name: String,
}

fn main() {
    let _ = AppShared {
        app_name: String::new(),
    };
}
