use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "tests.argument-description", view = "live/agents/tool.html")]
pub struct Component { count: u64 }

#[live]
impl Component {
    #[suprnova::tool(name = "add_to_counter", description = "Add.", callers = [public], arguments(typo = "How much to add."))]
    #[action]
    pub fn add(&mut self, amount: u64) {}
}

fn main() {}
