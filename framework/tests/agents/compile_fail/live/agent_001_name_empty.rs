use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "tests.name", view = "live/agents/tool.html")]
pub struct Component { count: u64 }

#[live]
impl Component {
    #[tool(name = "", description = "A tool.", callers = [public])]
    #[action]
    pub fn invalid(&mut self) {}
}

fn main() {}
