use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "named", view = "live/example.html")]
pub struct Named {
    component: String,
}

fn main() {}
