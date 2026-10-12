use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "transient", view = "live/example.html")]
pub struct TransientWithoutModel {
    #[transient]
    value: String,
}

fn main() {}
