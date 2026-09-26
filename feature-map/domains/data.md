# Feature map: `manual/data.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 63 checked.

## Rust API: suprnova

### Re-exported from other crates

- [ ] module `suprnova::serde` re-exports `serde`

### `suprnova::data::error` (private module; items are public through re-exports)

- [ ] enum `suprnova::IncludeError` · framework/src/data/error.rs:8 (also `suprnova::data::IncludeError`)
  - Variants: `UnknownInclude`
  - [ ] fn `suprnova::IncludeError::into_framework_error` · framework/src/data/error.rs:22

### `suprnova::data::field` (private module; items are public through re-exports)

- [ ] enum `suprnova::Field` · framework/src/data/field.rs:26 (also `suprnova::data::Field`)
  - Variants: `Absent`, `Null`, `Value`
  - [ ] fn `suprnova::Field::is_absent` · framework/src/data/field.rs:38
  - [ ] fn `suprnova::Field::is_null` · framework/src/data/field.rs:43
  - [ ] fn `suprnova::Field::is_value` · framework/src/data/field.rs:48
  - [ ] fn `suprnova::Field::as_value` · framework/src/data/field.rs:53
  - [ ] fn `suprnova::Field::into_value` · framework/src/data/field.rs:61
  - [ ] fn `suprnova::Field::into_option_or_null` · framework/src/data/field.rs:73

### `suprnova::data::include_set` (private module; items are public through re-exports)

- [ ] fn `suprnova::current_include_set` · framework/src/data/include_set.rs:305 (also `suprnova::data::current_include_set`)
- [ ] fn `suprnova::scope_include_set` · framework/src/data/include_set.rs:323 (also `suprnova::data::scope_include_set`)
- [ ] fn `suprnova::with_include_overrides` · framework/src/data/include_set.rs:357 (also `suprnova::data::with_include_overrides`)
- [ ] struct `suprnova::RequestIncludeSet` · framework/src/data/include_set.rs:23 (also `suprnova::data::RequestIncludeSet`)
  - Public fields: `include`, `exclude`, `only`, `except`
  - [ ] fn `suprnova::RequestIncludeSet::from_query` · framework/src/data/include_set.rs:56
  - [ ] fn `suprnova::RequestIncludeSet::is_empty` · framework/src/data/include_set.rs:82
  - [ ] fn `suprnova::RequestIncludeSet::includes` · framework/src/data/include_set.rs:95
  - [ ] fn `suprnova::RequestIncludeSet::is_excluded` · framework/src/data/include_set.rs:103
  - [ ] fn `suprnova::RequestIncludeSet::is_excepted` · framework/src/data/include_set.rs:112
  - [ ] fn `suprnova::RequestIncludeSet::is_only_listed` · framework/src/data/include_set.rs:122
  - [ ] fn `suprnova::RequestIncludeSet::is_visible` · framework/src/data/include_set.rs:142
  - [ ] fn `suprnova::RequestIncludeSet::include` · framework/src/data/include_set.rs:158
  - [ ] fn `suprnova::RequestIncludeSet::exclude` · framework/src/data/include_set.rs:170
  - [ ] fn `suprnova::RequestIncludeSet::only` · framework/src/data/include_set.rs:185
  - [ ] fn `suprnova::RequestIncludeSet::except` · framework/src/data/include_set.rs:198
  - [ ] fn `suprnova::RequestIncludeSet::include_when` · framework/src/data/include_set.rs:213
  - [ ] fn `suprnova::RequestIncludeSet::exclude_when` · framework/src/data/include_set.rs:228
  - [ ] fn `suprnova::RequestIncludeSet::only_when` · framework/src/data/include_set.rs:243
  - [ ] fn `suprnova::RequestIncludeSet::except_when` · framework/src/data/include_set.rs:254
  - [ ] fn `suprnova::RequestIncludeSet::merge` · framework/src/data/include_set.rs:267
- [ ] static `suprnova::data::REQUEST_INCLUDE_SET` · framework/src/data/include_set.rs:297

### `suprnova::data::middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::IncludeMiddleware` · framework/src/data/middleware.rs:49 (also `suprnova::data::IncludeMiddleware`)
  - Implements: `suprnova::Middleware`

### `suprnova::data::registry`

- [ ] fn `suprnova::data::registry::allowed_for` · framework/src/data/registry.rs:107
- [ ] fn `suprnova::data::registry::is_allowed` · framework/src/data/registry.rs:93
- [ ] fn `suprnova::data::registry::register` · framework/src/data/registry.rs:84
- [ ] struct `suprnova::data::registry::AllowedIncludes` · framework/src/data/registry.rs:45
  - Public fields: `struct_name`, `fields`

### `suprnova::data::route_params`

- [ ] fn `suprnova::data::route_params::parse_bool` · framework/src/data/route_params.rs:80
- [ ] fn `suprnova::data::route_params::parse_f32` · framework/src/data/route_params.rs:72
- [ ] fn `suprnova::data::route_params::parse_f64` · framework/src/data/route_params.rs:64
- [ ] fn `suprnova::data::route_params::parse_i128` · framework/src/data/route_params.rs:50
- [ ] fn `suprnova::data::route_params::parse_i32` · framework/src/data/route_params.rs:33
- [ ] fn `suprnova::data::route_params::parse_i64` · framework/src/data/route_params.rs:19
- [ ] fn `suprnova::data::route_params::parse_u128` · framework/src/data/route_params.rs:58
- [ ] fn `suprnova::data::route_params::parse_u32` · framework/src/data/route_params.rs:40
- [ ] fn `suprnova::data::route_params::parse_u64` · framework/src/data/route_params.rs:26
- [ ] fn `suprnova::data::route_params::pass_string` · framework/src/data/route_params.rs:89

### `suprnova::data::when_loaded` (private module; items are public through re-exports)

- [ ] trait `suprnova::IsRelationLoaded` · framework/src/data/when_loaded.rs:41 (also `suprnova::data::IsRelationLoaded`)
  - [ ] fn `suprnova::IsRelationLoaded::is_relation_loaded` · framework/src/data/when_loaded.rs:45 (required)

### `suprnova`

- [ ] macro `suprnova::when_loaded` · framework/src/data/when_loaded.rs:75

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::Data` · suprnova-macros/src/lib.rs:114 (re-exported as `suprnova::Data`)
  - Form: derive `#[derive(Data)]`
  - Helper attributes: `#[data]`, `#[json_resource]`
  - [ ] argument struct `#[data(allow_unknown_fields)]` · suprnova-macros/src/data.rs:194
  - [ ] argument struct `#[data(deny_unknown_fields)]` · suprnova-macros/src/data.rs:216
  - [ ] argument struct `#[data(authorize)]` · suprnova-macros/src/data.rs:184
  - [ ] argument struct `#[data(custom_authorize)]` · suprnova-macros/src/data.rs:222
  - [ ] argument struct `#[data(auto_lazy)]` · suprnova-macros/src/data.rs:182
  - [ ] argument struct `#[data(id_field)]` · suprnova-macros/src/data.rs:308
  - [ ] argument struct `#[data(max_body_bytes)]` · suprnova-macros/src/data.rs:196
  - [ ] argument struct `#[json_resource("type")]` · suprnova-macros/src/data.rs:237
  - [ ] argument field `#[data(input_only)]` · suprnova-macros/src/data.rs:728
  - [ ] argument field `#[data(output_only)]` · suprnova-macros/src/data.rs:730
  - [ ] argument field `#[data(allow_include)]` · suprnova-macros/src/data.rs:732
  - [ ] argument field `#[data(from_route_param)]` · suprnova-macros/src/data.rs:755
  - [ ] argument field `#[data(lazy = "inertia" | "deferred" | "closure" | "when_loaded")]` · suprnova-macros/src/data.rs:734
