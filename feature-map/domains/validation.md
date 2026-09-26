# Feature map: `manual/validation.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 91 checked.

## Rust API: suprnova

### Re-exported from other crates

- [ ] trait `suprnova::Validate` re-exports `validator::traits::Validate`
- [ ] proc derive `suprnova::Validate` re-exports `validator_derive::Validate`
- [ ] module `suprnova::validator` re-exports `validator`

### `suprnova::http::form_request` (private module; items are public through re-exports)

- [ ] trait `suprnova::FormRequest` · framework/src/http/form_request.rs:63 (also `suprnova::http::FormRequest`)
  - [ ] fn `suprnova::FormRequest::authorize` · framework/src/http/form_request.rs:70 (provided)
  - [ ] fn `suprnova::FormRequest::after_validation` · framework/src/http/form_request.rs:107 (provided)
  - [ ] fn `suprnova::FormRequest::after_validation_async` · framework/src/http/form_request.rs:157 (provided)
  - [ ] fn `suprnova::FormRequest::max_body_bytes` · framework/src/http/form_request.rs:186 (provided)
  - [ ] fn `suprnova::FormRequest::extract` · framework/src/http/form_request.rs:199 (provided)

### `suprnova::validation::message`

- [ ] struct `suprnova::ValidationMessage` · framework/src/validation/message.rs:29 (also `suprnova::validation::message::ValidationMessage`)
  - Public fields: `key`, `args`, `fallback`, `prefix`
  - [ ] fn `suprnova::ValidationMessage::keyed` · framework/src/validation/message.rs:47
  - [ ] fn `suprnova::ValidationMessage::arg` · framework/src/validation/message.rs:57
  - [ ] fn `suprnova::ValidationMessage::fallback` · framework/src/validation/message.rs:63
  - [ ] fn `suprnova::ValidationMessage::prefix` · framework/src/validation/message.rs:74
  - [ ] fn `suprnova::ValidationMessage::is_keyed` · framework/src/validation/message.rs:84
- [ ] type `suprnova::TranslateArgs` · framework/src/validation/message.rs:19 (also `suprnova::validation::message::TranslateArgs`)

### `suprnova::validation::rule::async_rules`

- [ ] struct `suprnova::Unique` · framework/src/validation/rule.rs:1970 (also `suprnova::async_rules::Unique`, `suprnova::validation::rule::Unique`, `suprnova::validation::rule::async_rules::Unique`)
  - Implements: `suprnova::AsyncRule`
  - [ ] fn `suprnova::Unique::new` · framework/src/validation/rule.rs:1980
  - [ ] fn `suprnova::Unique::ignore` · framework/src/validation/rule.rs:1996
  - [ ] fn `suprnova::Unique::ignore_with_column` · framework/src/validation/rule.rs:2004
  - [ ] fn `suprnova::Unique::where_eq` · framework/src/validation/rule.rs:2014
  - [ ] fn `suprnova::Unique::case_insensitive` · framework/src/validation/rule.rs:2022

### `suprnova::validation::rule::rules`

- [ ] struct `suprnova::Alpha` · framework/src/validation/rule.rs:397 (also `suprnova::rules::Alpha`, `suprnova::validation::rule::rules::Alpha`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::AlphaDash` · framework/src/validation/rule.rs:428 (also `suprnova::rules::AlphaDash`, `suprnova::validation::rule::rules::AlphaDash`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::AlphaNum` · framework/src/validation/rule.rs:412 (also `suprnova::rules::AlphaNum`, `suprnova::validation::rule::rules::AlphaNum`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::ArrayKeys` · framework/src/validation/rule.rs:1465 (also `suprnova::rules::ArrayKeys`, `suprnova::validation::rule::rules::ArrayKeys`)
  - Public tuple fields: 1
  - Implements: `suprnova::ValueRule`
- [ ] struct `suprnova::Between` · framework/src/validation/rule.rs:297 (also `suprnova::rules::Between`, `suprnova::validation::rule::rules::Between`)
  - Public tuple fields: 2
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Boolean` · framework/src/validation/rule.rs:375 (also `suprnova::rules::Boolean`, `suprnova::validation::rule::rules::Boolean`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Confirmed` · framework/src/validation/rule.rs:1419 (also `suprnova::rules::Confirmed`, `suprnova::validation::rule::rules::Confirmed`)
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Contains` · framework/src/validation/rule.rs:1609 (also `suprnova::rules::Contains`, `suprnova::validation::rule::rules::Contains`)
  - Public tuple fields: 1
  - Implements: `suprnova::ValueRule`
- [ ] struct `suprnova::Different` · framework/src/validation/rule.rs:1367 (also `suprnova::rules::Different`, `suprnova::validation::rule::rules::Different`)
  - Public fields: `other`
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Distinct` · framework/src/validation/rule.rs:1514 (also `suprnova::rules::Distinct`, `suprnova::validation::rule::rules::Distinct`)
  - Public fields: `ignore_case`, `strict`
  - Implements: `suprnova::ValueRule`
- [ ] struct `suprnova::DoesntContain` · framework/src/validation/rule.rs:1650 (also `suprnova::rules::DoesntContain`, `suprnova::validation::rule::rules::DoesntContain`)
  - Public tuple fields: 1
  - Implements: `suprnova::ValueRule`
- [ ] struct `suprnova::Email` · framework/src/validation/rule.rs:251 (also `suprnova::rules::Email`, `suprnova::validation::rule::rules::Email`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Gt` · framework/src/validation/rule.rs:1770 (also `suprnova::rules::Gt`, `suprnova::validation::rule::rules::Gt`)
  - Public tuple fields: 1
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Gte` · framework/src/validation/rule.rs:1787 (also `suprnova::rules::Gte`, `suprnova::validation::rule::rules::Gte`)
  - Public tuple fields: 1
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::HibpVerifier` · framework/src/validation/rule.rs:1116 (also `suprnova::rules::HibpVerifier`, `suprnova::validation::rule::rules::HibpVerifier`)
  - Implements: `suprnova::UncompromisedVerifier`
- [ ] struct `suprnova::HttpUrl` · framework/src/validation/rule.rs:677 (also `suprnova::rules::HttpUrl`, `suprnova::validation::rule::rules::HttpUrl`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::In` · framework/src/validation/rule.rs:317 (also `suprnova::rules::In`, `suprnova::validation::rule::rules::In`)
  - Public tuple fields: 1
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::InArray` · framework/src/validation/rule.rs:1579 (also `suprnova::rules::InArray`, `suprnova::validation::rule::rules::InArray`)
  - Public tuple fields: 1
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Integer` · framework/src/validation/rule.rs:346 (also `suprnova::rules::Integer`, `suprnova::validation::rule::rules::Integer`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Lt` · framework/src/validation/rule.rs:1804 (also `suprnova::rules::Lt`, `suprnova::validation::rule::rules::Lt`)
  - Public tuple fields: 1
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Lte` · framework/src/validation/rule.rs:1821 (also `suprnova::rules::Lte`, `suprnova::validation::rule::rules::Lte`)
  - Public tuple fields: 1
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Max` · framework/src/validation/rule.rs:282 (also `suprnova::rules::Max`, `suprnova::validation::rule::rules::Max`)
  - Public tuple fields: 1
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Min` · framework/src/validation/rule.rs:266 (also `suprnova::rules::Min`, `suprnova::validation::rule::rules::Min`)
  - Public tuple fields: 1
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::NotIn` · framework/src/validation/rule.rs:332 (also `suprnova::rules::NotIn`, `suprnova::validation::rule::rules::NotIn`)
  - Public tuple fields: 1
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Numeric` · framework/src/validation/rule.rs:361 (also `suprnova::rules::Numeric`, `suprnova::validation::rule::rules::Numeric`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Password` · framework/src/validation/rule.rs:788 (also `suprnova::rules::Password`, `suprnova::validation::rule::rules::Password`)
  - Implements: `suprnova::AsyncRule`, `suprnova::Rule`
  - [ ] fn `suprnova::Password::min` · framework/src/validation/rule.rs:805
  - [ ] fn `suprnova::Password::max` · framework/src/validation/rule.rs:820
  - [ ] fn `suprnova::Password::letters` · framework/src/validation/rule.rs:826
  - [ ] fn `suprnova::Password::mixed_case` · framework/src/validation/rule.rs:833
  - [ ] fn `suprnova::Password::numbers` · framework/src/validation/rule.rs:839
  - [ ] fn `suprnova::Password::symbols` · framework/src/validation/rule.rs:848
  - [ ] fn `suprnova::Password::uncompromised` · framework/src/validation/rule.rs:871
  - [ ] fn `suprnova::Password::uncompromised_with_threshold` · framework/src/validation/rule.rs:882
  - [ ] fn `suprnova::Password::verifier` · framework/src/validation/rule.rs:892
  - [ ] fn `suprnova::Password::defaults_with` · framework/src/validation/rule.rs:910
  - [ ] fn `suprnova::Password::defaults` · framework/src/validation/rule.rs:922
- [ ] struct `suprnova::Required` · framework/src/validation/rule.rs:238 (also `suprnova::rules::Required`, `suprnova::validation::rule::rules::Required`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::RequiredIf` · framework/src/validation/rule.rs:1234 (also `suprnova::rules::RequiredIf`, `suprnova::validation::rule::rules::RequiredIf`)
  - Public fields: `other`, `value`
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::RequiredUnless` · framework/src/validation/rule.rs:1321 (also `suprnova::rules::RequiredUnless`, `suprnova::validation::rule::rules::RequiredUnless`)
  - Public fields: `other`, `value`
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::RequiredWith` · framework/src/validation/rule.rs:1263 (also `suprnova::rules::RequiredWith`, `suprnova::validation::rule::rules::RequiredWith`)
  - Public fields: `others`
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::RequiredWithAll` · framework/src/validation/rule.rs:1291 (also `suprnova::rules::RequiredWithAll`, `suprnova::validation::rule::rules::RequiredWithAll`)
  - Public fields: `others`
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Same` · framework/src/validation/rule.rs:1349 (also `suprnova::rules::Same`, `suprnova::validation::rule::rules::Same`)
  - Public fields: `other`
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Url` · framework/src/validation/rule.rs:575 (also `suprnova::rules::Url`, `suprnova::validation::rule::rules::Url`)
  - Implements: `suprnova::Rule`
  - [ ] fn `suprnova::Url::protocols` · framework/src/validation/rule.rs:600
- [ ] struct `suprnova::UrlProtocols` · framework/src/validation/rule.rs:643 (also `suprnova::rules::UrlProtocols`, `suprnova::validation::rule::rules::UrlProtocols`)
  - Public tuple fields: 1
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Uuid` · framework/src/validation/rule.rs:694 (also `suprnova::rules::Uuid`, `suprnova::validation::rule::rules::Uuid`)
  - Implements: `suprnova::Rule`
- [ ] enum `suprnova::CompareWith` · framework/src/validation/rule.rs:1692 (also `suprnova::rules::CompareWith`, `suprnova::validation::rule::rules::CompareWith`)
  - Variants: `Number`, `NumericField`, `LengthField`
- [ ] trait `suprnova::UncompromisedVerifier` · framework/src/validation/rule.rs:1054 (also `suprnova::rules::UncompromisedVerifier`, `suprnova::validation::rule::rules::UncompromisedVerifier`)
  - Implemented here by: `HibpVerifier`
  - [ ] fn `suprnova::UncompromisedVerifier::verify` · framework/src/validation/rule.rs:1078 (required)

### `suprnova::validation::rule`

- [ ] trait `suprnova::AsyncRule` · framework/src/validation/rule.rs:1873 (also `suprnova::validation::rule::AsyncRule`)
  - Implemented here by: `Password`, `Unique`
  - [ ] fn `suprnova::AsyncRule::passes` · framework/src/validation/rule.rs:1881 (required)
  - [ ] fn `suprnova::AsyncRule::check_async` · framework/src/validation/rule.rs:1906 (provided)
- [ ] trait `suprnova::ContextualRule` · framework/src/validation/rule.rs:163 (also `suprnova::validation::rule::ContextualRule`)
  - Implemented here by: `Confirmed`, `Different`, `Gt`, `Gte`, `Lt`, `Lte`, `RequiredIf`, `RequiredUnless`, `RequiredWith`, `RequiredWithAll`, `Same`
  - [ ] fn `suprnova::ContextualRule::passes` · framework/src/validation/rule.rs:178 (required)
  - [ ] fn `suprnova::ContextualRule::check` · framework/src/validation/rule.rs:190 (provided)
  - [ ] fn `suprnova::ContextualRule::check_named` · framework/src/validation/rule.rs:205 (provided)
- [ ] trait `suprnova::Rule` · framework/src/validation/rule.rs:97 (also `suprnova::validation::rule::Rule`)
  - Implemented here by: `Alpha`, `AlphaDash`, `AlphaNum`, `Between`, `Boolean`, `Email`, `HttpUrl`, `In`, `InArray`, `Integer`, `Max`, `Min`, `NotIn`, `Numeric`, `Password`, `Required`, `Url`, `UrlProtocols`, `Uuid`
  - [ ] fn `suprnova::Rule::passes` · framework/src/validation/rule.rs:107 (required)
  - [ ] fn `suprnova::Rule::check` · framework/src/validation/rule.rs:116 (provided)
- [ ] trait `suprnova::ValueRule` · framework/src/validation/rule.rs:139 (also `suprnova::validation::rule::ValueRule`)
  - Implemented here by: `ArrayKeys`, `Contains`, `Distinct`, `DoesntContain`
  - [ ] fn `suprnova::ValueRule::passes` · framework/src/validation/rule.rs:142 (required)
  - [ ] fn `suprnova::ValueRule::check` · framework/src/validation/rule.rs:145 (provided)
- [ ] type `suprnova::FormContext` · framework/src/validation/rule.rs:154 (also `suprnova::validation::rule::FormContext`)

### `suprnova`

- [ ] macro `suprnova::validate` · framework/src/validation/rule.rs:2226

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::FormRequest` · suprnova-macros/src/lib.rs:414 (re-exported as `suprnova::FormRequestDerive`)
  - Form: derive `#[derive(FormRequest)]`
  - Helper attributes: `#[form_request]`
  - [ ] argument `#[form_request(max_body_bytes)]` · suprnova-macros/src/request.rs:38
  - [ ] argument `#[form_request(custom_hooks)]` · suprnova-macros/src/request.rs:43
- [ ] proc macro `suprnova_macros::request` · suprnova-macros/src/lib.rs:449 (re-exported as `suprnova::request`)
  - Form: attribute `#[request]`
