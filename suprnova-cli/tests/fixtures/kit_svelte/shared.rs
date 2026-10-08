use serde::Serialize;
use suprnova::InertiaProps;

#[derive(Serialize)]
pub struct UserInfo {
    pub id: u64,
    pub name: String,
    pub email: String,
}

#[derive(Serialize)]
pub struct Auth {
    pub user: Option<UserInfo>,
}

#[derive(InertiaProps, Serialize)]
#[inertia_props(shared)]
pub struct SharedData {
    pub auth: Auth,
}
