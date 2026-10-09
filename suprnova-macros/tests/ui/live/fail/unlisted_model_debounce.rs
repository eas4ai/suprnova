use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "timing.unlisted", view = "live/example.html")]
pub struct UnlistedModelDebounce {
    #[model(debounce = 300)]
    query: String,
}

fn main() {}
