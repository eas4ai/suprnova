use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "timing.conflict", view = "live/example.html")]
pub struct ConflictingModelTiming {
    #[model(blur, change)]
    query: String,
}

fn main() {}
