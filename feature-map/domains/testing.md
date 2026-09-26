# Feature map: `manual/testing.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 36 checked.

## Rust API: suprnova

### `suprnova::testing::expect` (private module; items are public through re-exports)

- [ ] fn `suprnova::testing::set_current_test_name` · framework/src/testing/expect.rs:13
- [ ] struct `suprnova::testing::Expect` · framework/src/testing/expect.rs:34
  - [ ] fn `suprnova::testing::Expect::new` · framework/src/testing/expect.rs:41
  - [ ] fn `suprnova::testing::Expect::to_equal` · framework/src/testing/expect.rs:57
  - [ ] fn `suprnova::testing::Expect::to_not_equal` · framework/src/testing/expect.rs:77
  - [ ] fn `suprnova::testing::Expect::to_be_true` · framework/src/testing/expect.rs:99
  - [ ] fn `suprnova::testing::Expect::to_be_false` · framework/src/testing/expect.rs:116
  - [ ] fn `suprnova::testing::Expect::to_be_some` · framework/src/testing/expect.rs:136
  - [ ] fn `suprnova::testing::Expect::to_be_none` · framework/src/testing/expect.rs:153
  - [ ] fn `suprnova::testing::Expect::to_contain_value` · framework/src/testing/expect.rs:173
  - [ ] fn `suprnova::testing::Expect::to_be_ok` · framework/src/testing/expect.rs:205
  - [ ] fn `suprnova::testing::Expect::to_be_err` · framework/src/testing/expect.rs:223
  - [ ] fn `suprnova::testing::Expect::to_contain` · framework/src/testing/expect.rs:244
  - [ ] fn `suprnova::testing::Expect::to_start_with` · framework/src/testing/expect.rs:263
  - [ ] fn `suprnova::testing::Expect::to_end_with` · framework/src/testing/expect.rs:282
  - [ ] fn `suprnova::testing::Expect::to_have_length` · framework/src/testing/expect.rs:301
  - [ ] fn `suprnova::testing::Expect::to_be_empty` · framework/src/testing/expect.rs:323
  - [ ] fn `suprnova::testing::Expect::to_contain` · framework/src/testing/expect.rs:337
  - [ ] fn `suprnova::testing::Expect::to_start_with` · framework/src/testing/expect.rs:349
  - [ ] fn `suprnova::testing::Expect::to_end_with` · framework/src/testing/expect.rs:361
  - [ ] fn `suprnova::testing::Expect::to_have_length` · framework/src/testing/expect.rs:373
  - [ ] fn `suprnova::testing::Expect::to_be_empty` · framework/src/testing/expect.rs:388
  - [ ] fn `suprnova::testing::Expect::to_have_length` · framework/src/testing/expect.rs:409
  - [ ] fn `suprnova::testing::Expect::to_contain` · framework/src/testing/expect.rs:431
  - [ ] fn `suprnova::testing::Expect::to_be_empty` · framework/src/testing/expect.rs:450
  - [ ] fn `suprnova::testing::Expect::to_be_greater_than` · framework/src/testing/expect.rs:470
  - [ ] fn `suprnova::testing::Expect::to_be_less_than` · framework/src/testing/expect.rs:488
  - [ ] fn `suprnova::testing::Expect::to_be_greater_than_or_equal` · framework/src/testing/expect.rs:506
  - [ ] fn `suprnova::testing::Expect::to_be_less_than_or_equal` · framework/src/testing/expect.rs:524

### `suprnova::testing`

- [ ] fn `suprnova::testing::install_test_encryption_key` · framework/src/testing/mod.rs:49 (feature: `testing`)
- [ ] fn `suprnova::testing::install_test_encryption_keyring` · framework/src/testing/mod.rs:75 (feature: `testing`)

### `suprnova`

- [ ] macro `suprnova::expect` · framework/src/lib.rs:751

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::describe` · suprnova-macros/src/lib.rs:606 (re-exported as `suprnova::describe`)
  - Form: function-like `describe!(...)`
- [ ] proc macro `suprnova_macros::suprnova_test` · suprnova-macros/src/lib.rs:497 (re-exported as `suprnova::suprnova_test`)
  - Form: attribute `#[suprnova_test]`
  - [ ] argument `migrator = Type` · suprnova-macros/src/suprnova_test.rs:23
- [ ] proc macro `suprnova_macros::test` · suprnova-macros/src/lib.rs:651 (re-exported as `suprnova::test`)
  - Form: function-like `test!(...)`
