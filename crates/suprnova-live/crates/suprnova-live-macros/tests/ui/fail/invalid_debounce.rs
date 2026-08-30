use suprnova_live_macros::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "timing.invalid", view = "live/example.html")]
pub struct InvalidDebounce {
    #[model(debounce = 0)]
    query: String,
}

fn main() {}
