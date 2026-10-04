#![allow(dead_code)]

//! Parameter names the generated code also uses for its own locals:
//! `target`, `arguments`, `authorization`, `request`, `issues`,
//! `parameters` and `context`. Each must reach its method as declared.

use suprnova::live::{AuthorizedAction, LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "names.locals", view = "live/counter.html")]
pub struct Locals {
    count: u64,
}

#[live]
impl Locals {
    #[mount]
    pub fn mount(parameters: u64, context: u64) -> Self {
        Self {
            count: parameters + context,
        }
    }

    #[action]
    pub fn record(&mut self, target: u64, arguments: String) {
        self.count = target + arguments.len() as u64;
    }

    #[action(authorize = "current")]
    pub async fn guarded(&mut self, authorization: &AuthorizedAction, target: u64) {
        let _ = authorization;
        self.count = target;
    }

    #[action(validate = "all")]
    pub fn checked(&mut self, target: u64, request: u64, issues: u64) {
        self.count = target + request + issues;
    }

    #[validate]
    pub fn validate_locals(&self) {}

    #[validate(action = "checked")]
    pub fn validate_checked(&self, target: u64, request: u64, issues: u64) {
        let _ = (target, request, issues);
    }
}

fn main() {
    let descriptor = <Locals as ::suprnova::live::__private::metadata::LiveComponentContract>::descriptor()
        .expect("generated metadata must be valid");
    assert_eq!(descriptor.metadata().actions().len(), 3);
}
