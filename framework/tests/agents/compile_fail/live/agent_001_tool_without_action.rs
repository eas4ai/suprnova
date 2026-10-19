use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "fail.tool", view = "live/agents/tool.html")]
pub struct Counter {
    count: u64,
}

#[live]
impl Counter {
    #[tool(name = "counter.add", description = "Add to the counter.", callers = [user])]
    pub fn add(&mut self, amount: u64) {
        self.count += amount;
    }
}

fn main() {}
