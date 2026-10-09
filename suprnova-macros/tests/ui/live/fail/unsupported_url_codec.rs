use suprnova::live::LiveComponent;

#[derive(LiveComponent)]
#[live(name = "search", view = "live/search.html")]
pub struct Search {
    #[url]
    filters: Vec<String>,
}

fn main() {}
