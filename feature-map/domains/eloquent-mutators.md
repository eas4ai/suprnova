# Feature map: `manual/eloquent-mutators.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 35 checked.

## Rust API: suprnova

### `suprnova::eloquent::casts::encrypted`

- [ ] struct `suprnova::AsEncrypted` · framework/src/eloquent/casts/encrypted.rs:41 (also `suprnova::eloquent::AsEncrypted`, `suprnova::eloquent::casts::AsEncrypted`, `suprnova::eloquent::casts::encrypted::AsEncrypted`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsEncryptedArray` · framework/src/eloquent/casts/encrypted.rs:107 (also `suprnova::eloquent::AsEncryptedArray`, `suprnova::eloquent::casts::AsEncryptedArray`, `suprnova::eloquent::casts::encrypted::AsEncryptedArray`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsEncryptedCollection` · framework/src/eloquent/casts/encrypted.rs:261 (also `suprnova::eloquent::AsEncryptedCollection`, `suprnova::eloquent::casts::AsEncryptedCollection`, `suprnova::eloquent::casts::encrypted::AsEncryptedCollection`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsEncryptedObject` · framework/src/eloquent/casts/encrypted.rs:183 (also `suprnova::eloquent::AsEncryptedObject`, `suprnova::eloquent::casts::AsEncryptedObject`, `suprnova::eloquent::casts::encrypted::AsEncryptedObject`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsHashed` · framework/src/eloquent/casts/encrypted.rs:348 (also `suprnova::eloquent::AsHashed`, `suprnova::eloquent::casts::AsHashed`, `suprnova::eloquent::casts::encrypted::AsHashed`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`

### `suprnova::eloquent::casts::enum_cast`

- [ ] struct `suprnova::AsEnum` · framework/src/eloquent/casts/enum_cast.rs:31 (also `suprnova::eloquent::AsEnum`, `suprnova::eloquent::casts::AsEnum`, `suprnova::eloquent::casts::enum_cast::AsEnum`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`

### `suprnova::eloquent::casts::primitive`

- [ ] struct `suprnova::AsBool` · framework/src/eloquent/casts/primitive.rs:33 (also `suprnova::eloquent::AsBool`, `suprnova::eloquent::casts::AsBool`, `suprnova::eloquent::casts::primitive::AsBool`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsDecimal` · framework/src/eloquent/casts/primitive.rs:237 (also `suprnova::eloquent::AsDecimal`, `suprnova::eloquent::casts::AsDecimal`, `suprnova::eloquent::casts::primitive::AsDecimal`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsFloat` · framework/src/eloquent/casts/primitive.rs:150 (also `suprnova::eloquent::AsFloat`, `suprnova::eloquent::casts::AsFloat`, `suprnova::eloquent::casts::primitive::AsFloat`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsInt` · framework/src/eloquent/casts/primitive.rs:97 (also `suprnova::eloquent::AsInt`, `suprnova::eloquent::casts::AsInt`, `suprnova::eloquent::casts::primitive::AsInt`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsString` · framework/src/eloquent/casts/primitive.rs:195 (also `suprnova::eloquent::AsString`, `suprnova::eloquent::casts::AsString`, `suprnova::eloquent::casts::primitive::AsString`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`

### `suprnova::eloquent::casts::structured`

- [ ] struct `suprnova::AsArray` · framework/src/eloquent/casts/structured.rs:39 (also `suprnova::eloquent::AsArray`, `suprnova::eloquent::casts::AsArray`, `suprnova::eloquent::casts::structured::AsArray`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsArrayObject` · framework/src/eloquent/casts/structured.rs:306 (also `suprnova::eloquent::AsArrayObject`, `suprnova::eloquent::casts::AsArrayObject`, `suprnova::eloquent::casts::structured::AsArrayObject`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsCollection` · framework/src/eloquent/casts/structured.rs:173 (also `suprnova::eloquent::AsCollection`, `suprnova::eloquent::casts::AsCollection`, `suprnova::eloquent::casts::structured::AsCollection`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsJson` · framework/src/eloquent/casts/structured.rs:239 (also `suprnova::eloquent::AsJson`, `suprnova::eloquent::casts::AsJson`, `suprnova::eloquent::casts::structured::AsJson`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsObject` · framework/src/eloquent/casts/structured.rs:108 (also `suprnova::eloquent::AsObject`, `suprnova::eloquent::casts::AsObject`, `suprnova::eloquent::casts::structured::AsObject`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`

### `suprnova::eloquent::casts::temporal`

- [ ] struct `suprnova::AsDate` · framework/src/eloquent/casts/temporal.rs:33 (also `suprnova::eloquent::AsDate`, `suprnova::eloquent::casts::AsDate`, `suprnova::eloquent::casts::temporal::AsDate`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsDateTime` · framework/src/eloquent/casts/temporal.rs:88 (also `suprnova::eloquent::AsDateTime`, `suprnova::eloquent::casts::AsDateTime`, `suprnova::eloquent::casts::temporal::AsDateTime`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsImmutableDate` · framework/src/eloquent/casts/temporal.rs:154 (also `suprnova::eloquent::AsImmutableDate`, `suprnova::eloquent::casts::AsImmutableDate`, `suprnova::eloquent::casts::temporal::AsImmutableDate`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsImmutableDateTime` · framework/src/eloquent/casts/temporal.rs:181 (also `suprnova::eloquent::AsImmutableDateTime`, `suprnova::eloquent::casts::AsImmutableDateTime`, `suprnova::eloquent::casts::temporal::AsImmutableDateTime`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsOptionalDateTime` · framework/src/eloquent/casts/temporal.rs:215 (also `suprnova::eloquent::AsOptionalDateTime`, `suprnova::eloquent::casts::AsOptionalDateTime`, `suprnova::eloquent::casts::temporal::AsOptionalDateTime`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsTimestamp` · framework/src/eloquent/casts/temporal.rs:275 (also `suprnova::eloquent::AsTimestamp`, `suprnova::eloquent::casts::AsTimestamp`, `suprnova::eloquent::casts::temporal::AsTimestamp`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`

### `suprnova::eloquent::casts`

- [ ] trait `suprnova::Cast` · framework/src/eloquent/casts/mod.rs:37 (also `suprnova::eloquent::Cast`, `suprnova::eloquent::casts::Cast`)
  - Implemented here by: `AsArray`, `AsArrayObject`, `AsBool`, `AsCollection`, `AsDate`, `AsDateTime`, `AsDecimal`, `AsEncrypted`, `AsEncryptedArray`, `AsEncryptedCollection`, `AsEncryptedObject`, `AsEnum`, `AsFloat`, `AsHashed`, `AsImmutableDate`, `AsImmutableDateTime`, `AsInt`, `AsJson`, `AsObject`, `AsOptionalDateTime`, `AsString`, `AsTimestamp`
  - [ ] type `suprnova::Cast::Runtime` · framework/src/eloquent/casts/mod.rs:39
  - [ ] type `suprnova::Cast::Storage` · framework/src/eloquent/casts/mod.rs:41
  - [ ] fn `suprnova::Cast::to_storage` · framework/src/eloquent/casts/mod.rs:44 (required)
  - [ ] fn `suprnova::Cast::from_storage` · framework/src/eloquent/casts/mod.rs:46 (required)
- [ ] trait `suprnova::DynCast` · framework/src/eloquent/casts/mod.rs:64 (also `suprnova::eloquent::DynCast`, `suprnova::eloquent::casts::DynCast`)
  - [ ] fn `suprnova::DynCast::from_storage_json` · framework/src/eloquent/casts/mod.rs:67 (required)
  - [ ] fn `suprnova::DynCast::to_storage_json` · framework/src/eloquent/casts/mod.rs:73 (required)
- [ ] trait `suprnova::IntoDynCast` · framework/src/eloquent/casts/mod.rs:79 (also `suprnova::eloquent::IntoDynCast`, `suprnova::eloquent::casts::IntoDynCast`)
  - Implemented here by: `AsArray`, `AsArrayObject`, `AsBool`, `AsCollection`, `AsDate`, `AsDateTime`, `AsDecimal`, `AsEncrypted`, `AsEncryptedArray`, `AsEncryptedCollection`, `AsEncryptedObject`, `AsEnum`, `AsFloat`, `AsHashed`, `AsImmutableDate`, `AsImmutableDateTime`, `AsInt`, `AsJson`, `AsObject`, `AsOptionalDateTime`, `AsString`, `AsTimestamp`
  - [ ] fn `suprnova::IntoDynCast::into_dyn` · framework/src/eloquent/casts/mod.rs:81 (required)

### `suprnova`

- [ ] macro `suprnova::casts` · framework/src/eloquent/casts/mod.rs:117

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::accessor` · suprnova-macros/src/lib.rs:883 (re-exported as `suprnova::accessor`)
  - Form: attribute `#[accessor]`
- [ ] proc macro `suprnova_macros::mutator` · suprnova-macros/src/lib.rs:927 (re-exported as `suprnova::mutator`)
  - Form: attribute `#[mutator]`
