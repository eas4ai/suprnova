# Live component authoring and metadata

This document records the Iteration 002 authoring contract. The examples use
the final application namespace required from Iteration 005's public Suprnova
integration. They are not instructions to depend on the internal engine or its
development macro package directly, and they do not claim that facade complete.

## Application-facing authoring

Application code imports macros and runtime types from `suprnova::live`:

```rust
use suprnova::live::{LiveComponent, live};

#[derive(Default, LiveComponent)]
#[live(
    name = "catalog.search",
    view = "live/catalog/search.html",
    minimum_protocol_version = 2,
    refresh_on_promote
)]
pub struct Search {
    title: String,
    #[public]
    prompt: String,
    #[model(debounce = 250)]
    #[url(key = "q", mode = "reflect", omit_default)]
    query: String,
    #[model(transient)]
    upload_token: String,
    #[locked]
    owner_id: i64,
    #[session]
    locale: String,
    #[secret]
    csrf_secret: Vec<u8>,
}

#[live]
impl Search {
    #[mount]
    pub fn mount() -> Self {
        Self::default()
    }

    #[action(
        name = "save",
        authorize = "current",
        validate = "all",
        transaction = "required"
    )]
    pub async fn save(&mut self) {
        self.title = self.query.clone();
    }

    #[validate]
    pub fn validate_search(&self) {}

    #[teardown]
    pub fn teardown(&mut self) {}
}
```

The derive accepts the component name, checked view, independent component,
state-schema, action-schema, checker-contract, and minimum-protocol versions,
`refresh_on_promote`, and registered event/effect payload types. Field helpers
are `#[public]`, `#[model]`, `#[model(transient)]`, `#[locked]`,
`#[server_only]`, `#[session]`, and `#[secret]`; an unannotated field is ordinary
instance-only state. The inherent `#[live]` implementation consumes
`#[mount]`, `#[action]`, `#[computed]`, `#[validate]`, `#[hydrate]`,
`#[rendering]`, `#[rendered]`, `#[dehydrate]`, `#[teardown]`,
`#[params_changed]`, and `#[lazy_complete]` helpers.

Unknown, duplicate, conflicting, misplaced, inaccessible, generic, borrowed,
or unsupported declarations fail at compile time. Action methods are generated
into a closed dispatch table; browser text never names a Rust type or arbitrary
method.

## Generated metadata and registration

Every component produces immutable metadata containing its stable component and
view identities, five independent contract versions, ordered field and action
metadata, registered events and effects, URL/binding policy, and the
`refresh_on_promote` declaration. Its canonical contract digest includes
semantic metadata but excludes Rust type paths, source paths, addresses, build
order, and registration order.

Startup code explicitly registers component contracts with
`LiveRegistry::builder()` and then builds an immutable `LiveRegistry`.
Duplicate component, view, or action identities and contract conflicts fail
deterministically. There is no `inventory` submission, linker section, global
mutable registry, or browser-selected Rust path. A mount is also matched against
the immutable route/slot catalog before it can reach component execution.

Generated runtime references are absolute `::suprnova::live` or
`::suprnova::live::__private` paths. The hidden namespace is generated-code
plumbing, not an application extension surface. Application-authored event and
effect contracts use the public `EventPayloadMetadata` and
`EffectPayloadMetadata` traits; authorized action parameters use the public
`AuthorizedAction` proof type.

## Internal standalone machinery

This historical section name identifies machinery built before the cutover; the
packages themselves now live only in the integrated internal crate authority.

`suprnova-macros/src/live` is the sole production implementation of the Live
procedural macros. `crates/suprnova-live-macro-fixture` impersonates the
required `suprnova` facade only in engine-level compile fixtures so generated
final paths remain proved without introducing a framework dependency.
`crates/suprnova-live-test-support` contains synthetic trusted contexts and the
browserless harness. These packages are unpublished and are not application
dependencies.

The host-neutral document adapter, endpoint types, and test host ports are
conformance machinery inside the integrated internal crate. This conformance
machinery does not claim registered Suprnova integration. The real
`suprnova::live` facade, router registration, middleware adapters,
session/auth/tenant services, and remaining framework integration work stay
within Iteration 005 until their framework tests pass.
