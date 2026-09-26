# Feature map: `manual/eloquent-factories.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 29 checked.

## Rust API: suprnova

### Re-exported from other crates

- [ ] proc derive `suprnova::Dummy` re-exports `dummy::Dummy`
- [ ] trait `suprnova::Dummy` re-exports `fake::Dummy`
- [ ] trait `suprnova::Fake` re-exports `fake::Fake`
- [ ] struct `suprnova::Faker` re-exports `fake::Faker`

### `suprnova::factory::persist` (private module; items are public through re-exports)

- [ ] fn `suprnova::persist_via_seaorm` · framework/src/factory/persist.rs:80 (also `suprnova::factory::persist_via_seaorm`)
- [ ] trait `suprnova::Persistable` · framework/src/factory/persist.rs:49 (also `suprnova::factory::Persistable`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] fn `suprnova::Persistable::persist` · framework/src/factory/persist.rs:53 (required)

### `suprnova::factory::sequence` (private module; items are public through re-exports)

- [ ] struct `suprnova::Sequence` · framework/src/factory/sequence.rs:36 (also `suprnova::factory::Sequence`)
  - [ ] fn `suprnova::Sequence::new` · framework/src/factory/sequence.rs:43
  - [ ] fn `suprnova::Sequence::next` · framework/src/factory/sequence.rs:51
  - [ ] fn `suprnova::Sequence::reset` · framework/src/factory/sequence.rs:56

### `suprnova::factory`

- [ ] struct `suprnova::FactoryBuilder` · framework/src/factory/mod.rs:107 (also `suprnova::factory::FactoryBuilder`)
  - [ ] fn `suprnova::FactoryBuilder::count` · framework/src/factory/mod.rs:117
  - [ ] fn `suprnova::FactoryBuilder::with` · framework/src/factory/mod.rs:126
  - [ ] fn `suprnova::FactoryBuilder::prepend` · framework/src/factory/mod.rs:141
  - [ ] fn `suprnova::FactoryBuilder::when` · framework/src/factory/mod.rs:161
  - [ ] fn `suprnova::FactoryBuilder::make` · framework/src/factory/mod.rs:172
  - [ ] fn `suprnova::FactoryBuilder::make_one` · framework/src/factory/mod.rs:186
  - [ ] fn `suprnova::FactoryBuilder::make_many` · framework/src/factory/mod.rs:193
  - [ ] fn `suprnova::FactoryBuilder::create` · framework/src/factory/mod.rs:221
  - [ ] fn `suprnova::FactoryBuilder::create_one` · framework/src/factory/mod.rs:230
  - [ ] fn `suprnova::FactoryBuilder::create_many` · framework/src/factory/mod.rs:238
- [ ] trait `suprnova::Factory` · framework/src/factory/mod.rs:59 (also `suprnova::factory::Factory`)
  - [ ] type `suprnova::Factory::Model` · framework/src/factory/mod.rs:61
  - [ ] fn `suprnova::Factory::definition` · framework/src/factory/mod.rs:67 (required)
  - [ ] fn `suprnova::Factory::new` · framework/src/factory/mod.rs:73 (provided)
  - [ ] fn `suprnova::Factory::times` · framework/src/factory/mod.rs:87 (provided)

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::Factory` · suprnova-macros/src/lib.rs:1164 (re-exported as `suprnova::Factory`)
  - Form: derive `#[derive(Factory)]`
  - Helper attributes: `#[factory]`
  - [ ] argument `#[factory(name = ...)]` · suprnova-macros/src/factory.rs:58
