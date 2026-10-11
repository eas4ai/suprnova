//! UI-015: the `suprnova.` component namespace is reserved for the shipped
//! library. The framework's own crate is a library crate, so a component it
//! compiles registers under the prefix; the refusal for a foreign crate is
//! proved from the dogfood application, whose crate name is not a library.

use suprnova::live::{LiveComponent, LiveRegistry, live};

#[derive(LiveComponent)]
#[live(
    name = "suprnova.namespace-probe",
    view = "live/tests/hardening-plain.html"
)]
pub struct NamespaceProbe {
    #[model]
    note: String,
}

#[live]
impl NamespaceProbe {
    #[mount]
    pub fn mount() -> Self {
        Self {
            note: String::new(),
        }
    }

    #[action]
    pub fn save(&mut self) {}
}

#[test]
fn a_library_crate_registers_under_the_reserved_prefix() {
    let registry = LiveRegistry::builder()
        .register::<NamespaceProbe>()
        .expect("the framework crate owns the suprnova. namespace")
        .build();
    assert_eq!(registry.len(), 1);
}
