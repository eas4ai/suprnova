use suprnova::live::{LiveComponent, live};

suprnova::tool_scopes! {
    /// The scopes of this application.
    pub mod scope {
        COUNTER_READ = "counter:read" => "Read your counters";
    }
}

#[derive(LiveComponent)]
#[live(name = "fail.tool", view = "live/agents/tool.html")]
pub struct Counter {
    count: u64,
}

#[live]
impl Counter {
    #[tool(
        name = "counter.add",
        description = "Add to the counter.",
        callers = [user],
        scopes = [scope::COUNTER_WRITE],
    )]
    #[action]
    pub fn add(&mut self, amount: u64) {
        self.count += amount;
    }
}

fn main() {}
