use suprnova::live::{LiveComponent, live};
use suprnova::{Response, handler, json_response, tool};

#[tool(name = "add_to_counter", description = "Add over HTTP.", callers = [user])]
#[handler]
pub async fn add_over_http() -> Response {
    json_response!({ "ok": true })
}

#[derive(LiveComponent)]
#[live(name = "fail_tool", view = "live/agents/tool.html")]
pub struct Counter {
    count: u64,
}

#[live]
impl Counter {
    #[tool(name = "add_to_counter", description = "Add to the counter.", callers = [user])]
    #[action]
    pub fn add(&mut self, amount: u64) {
        self.count += amount;
    }
}

fn main() {}
