use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "duplicate.url", view = "live/example.html")]
pub struct DuplicateUrlQuery {
    #[model]
    #[url(key = "q")]
    query: String,
    #[public]
    #[url(key = "q")]
    category: String,
}

fn main() {}
