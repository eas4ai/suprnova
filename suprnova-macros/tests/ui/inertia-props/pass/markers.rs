//! Both markers compile, and the derive writes the same JSON with them as
//! without them: the markers only tell `suprnova generate-types` which
//! struct types `sharedPageProps` and which types `flashDataType`.

use suprnova::InertiaProps;

#[derive(InertiaProps)]
#[inertia_props(shared)]
pub struct AppShared {
    pub app_name: String,
}

#[derive(InertiaProps)]
#[inertia_props(flash)]
#[serde(rename_all = "camelCase")]
pub struct Toast {
    pub toast_message: String,
}

#[derive(InertiaProps)]
pub struct Unmarked {
    pub id: i64,
}

fn main() {
    let shared = serde_json::to_value(AppShared {
        app_name: "Suprnova".into(),
    })
    .expect("shared props serialize");
    assert_eq!(shared, serde_json::json!({ "app_name": "Suprnova" }));

    let flash = serde_json::to_value(Toast {
        toast_message: "Saved".into(),
    })
    .expect("flash props serialize");
    assert_eq!(flash, serde_json::json!({ "toastMessage": "Saved" }));

    let plain = serde_json::to_value(Unmarked { id: 7 }).expect("props serialize");
    assert_eq!(plain, serde_json::json!({ "id": 7 }));
}
