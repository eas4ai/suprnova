use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "fail.tool", view = "live/agents/tool.html")]
pub struct Counter {
    count: u64,
}

#[live]
impl Counter {
    #[tool(name = "counter.apply", description = "Apply a change.", callers = [user])]
    #[action]
    pub fn apply(&mut self, change: serde_json::Value) {
        let _ = change;
    }
}

fn main() {}
