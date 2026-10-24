#![forbid(unsafe_code)]

use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "tests.long-name", view = "live/agents/tool.html")]
pub struct Component { count: u64 }

#[live]
impl Component {
    #[suprnova::tool(name = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", description = "A tool.", callers = [public])]
    #[action]
    pub fn valid(&mut self) {}
}

fn main() {}
