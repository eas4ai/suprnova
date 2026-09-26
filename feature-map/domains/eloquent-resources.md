# Feature map: `manual/eloquent-resources.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 93 checked.

## Rust API: suprnova

### `suprnova::resources::builder`

- [ ] fn `suprnova::resources::render_resource_object` · framework/src/resources/builder.rs:239 (also `suprnova::resources::builder::render_resource_object`)
- [ ] struct `suprnova::IncludedSink` · framework/src/resources/builder.rs:17 (also `suprnova::resources::IncludedSink`, `suprnova::resources::builder::IncludedSink`)
  - [ ] fn `suprnova::IncludedSink::new` · framework/src/resources/builder.rs:24
  - [ ] fn `suprnova::IncludedSink::push` · framework/src/resources/builder.rs:30
  - [ ] fn `suprnova::IncludedSink::into_items` · framework/src/resources/builder.rs:52
  - [ ] fn `suprnova::IncludedSink::items` · framework/src/resources/builder.rs:57
  - [ ] fn `suprnova::IncludedSink::is_empty` · framework/src/resources/builder.rs:63
- [ ] struct `suprnova::JsonApiBuilder` · framework/src/resources/builder.rs:71 (also `suprnova::resources::JsonApiBuilder`, `suprnova::resources::builder::JsonApiBuilder`)
  - [ ] fn `suprnova::JsonApiBuilder::with_meta` · framework/src/resources/builder.rs:115
  - [ ] fn `suprnova::JsonApiBuilder::with_meta_kv` · framework/src/resources/builder.rs:121
  - [ ] fn `suprnova::JsonApiBuilder::with_meta_map` · framework/src/resources/builder.rs:126
  - [ ] fn `suprnova::JsonApiBuilder::with_link` · framework/src/resources/builder.rs:134
  - [ ] fn `suprnova::JsonApiBuilder::with_link_value` · framework/src/resources/builder.rs:141
  - [ ] fn `suprnova::JsonApiBuilder::with_additional` · framework/src/resources/builder.rs:149
  - [ ] fn `suprnova::JsonApiBuilder::with_additional_map` · framework/src/resources/builder.rs:155
  - [ ] fn `suprnova::JsonApiBuilder::with_jsonapi` · framework/src/resources/builder.rs:163
  - [ ] fn `suprnova::JsonApiBuilder::build` · framework/src/resources/builder.rs:202

### `suprnova::resources::fieldset`

- [ ] fn `suprnova::current_fieldset` · framework/src/resources/fieldset.rs:68 (also `suprnova::resources::current_fieldset`, `suprnova::resources::fieldset::current_fieldset`)
- [ ] fn `suprnova::scope_fieldset` · framework/src/resources/fieldset.rs:74 (also `suprnova::resources::fieldset::scope_fieldset`, `suprnova::resources::scope_fieldset`)
- [ ] struct `suprnova::RequestFieldsetSet` · framework/src/resources/fieldset.rs:8 (also `suprnova::resources::RequestFieldsetSet`, `suprnova::resources::fieldset::RequestFieldsetSet`)
  - [ ] fn `suprnova::RequestFieldsetSet::from_query` · framework/src/resources/fieldset.rs:21
  - [ ] fn `suprnova::RequestFieldsetSet::fields_for` · framework/src/resources/fieldset.rs:47
  - [ ] fn `suprnova::RequestFieldsetSet::is_empty` · framework/src/resources/fieldset.rs:55
- [ ] static `suprnova::resources::REQUEST_FIELDSET` · framework/src/resources/fieldset.rs:60 (also `suprnova::resources::fieldset::REQUEST_FIELDSET`)

### `suprnova::resources::include_tree`

- [ ] fn `suprnova::current_max_relationship_depth` · framework/src/resources/include_tree.rs:76 (also `suprnova::resources::current_max_relationship_depth`, `suprnova::resources::include_tree::current_max_relationship_depth`)
- [ ] fn `suprnova::max_relationship_depth` · framework/src/resources/include_tree.rs:66 (also `suprnova::resources::include_tree::max_relationship_depth`, `suprnova::resources::max_relationship_depth`)
- [ ] struct `suprnova::IncludeTree` · framework/src/resources/include_tree.rs:30 (also `suprnova::resources::IncludeTree`, `suprnova::resources::include_tree::IncludeTree`)
  - Public fields: `children`
  - [ ] fn `suprnova::IncludeTree::from_include_set` · framework/src/resources/include_tree.rs:87
  - [ ] fn `suprnova::IncludeTree::is_empty` · framework/src/resources/include_tree.rs:105
  - [ ] fn `suprnova::IncludeTree::subtree` · framework/src/resources/include_tree.rs:111
  - [ ] fn `suprnova::IncludeTree::iter` · framework/src/resources/include_tree.rs:116
- [ ] const `suprnova::DEFAULT_MAX_RELATIONSHIP_DEPTH` · framework/src/resources/include_tree.rs:38 (also `suprnova::resources::DEFAULT_MAX_RELATIONSHIP_DEPTH`, `suprnova::resources::include_tree::DEFAULT_MAX_RELATIONSHIP_DEPTH`)

### `suprnova::resources::jsonapi_info`

- [ ] struct `suprnova::JsonApiInfo` · framework/src/resources/jsonapi_info.rs:10 (also `suprnova::resources::JsonApiInfo`, `suprnova::resources::jsonapi_info::JsonApiInfo`)
  - Public fields: `version`, `ext`, `profile`, `meta`
  - [ ] fn `suprnova::JsonApiInfo::new` · framework/src/resources/jsonapi_info.rs:23
  - [ ] fn `suprnova::JsonApiInfo::with_version` · framework/src/resources/jsonapi_info.rs:28
  - [ ] fn `suprnova::JsonApiInfo::with_ext` · framework/src/resources/jsonapi_info.rs:34
  - [ ] fn `suprnova::JsonApiInfo::with_profile` · framework/src/resources/jsonapi_info.rs:40
  - [ ] fn `suprnova::JsonApiInfo::with_meta` · framework/src/resources/jsonapi_info.rs:46
  - [ ] fn `suprnova::JsonApiInfo::is_empty` · framework/src/resources/jsonapi_info.rs:52
  - [ ] fn `suprnova::JsonApiInfo::to_value` · framework/src/resources/jsonapi_info.rs:62

### `suprnova::resources::maybe`

- [ ] fn `suprnova::insert_maybe` · framework/src/resources/maybe.rs:224 (also `suprnova::resources::insert_maybe`, `suprnova::resources::maybe::insert_maybe`)
- [ ] fn `suprnova::strip_missing_values` · framework/src/resources/maybe.rs:194 (also `suprnova::resources::maybe::strip_missing_values`, `suprnova::resources::strip_missing_values`)
- [ ] enum `suprnova::Maybe` · framework/src/resources/maybe.rs:51 (also `suprnova::resources::Maybe`, `suprnova::resources::maybe::Maybe`)
  - Variants: `Present`, `Missing`
  - [ ] fn `suprnova::Maybe::present` · framework/src/resources/maybe.rs:65
  - [ ] fn `suprnova::Maybe::missing` · framework/src/resources/maybe.rs:70
  - [ ] fn `suprnova::Maybe::when` · framework/src/resources/maybe.rs:76
  - [ ] fn `suprnova::Maybe::unless` · framework/src/resources/maybe.rs:85
  - [ ] fn `suprnova::Maybe::when_with` · framework/src/resources/maybe.rs:90
  - [ ] fn `suprnova::Maybe::is_missing` · framework/src/resources/maybe.rs:99
  - [ ] fn `suprnova::Maybe::is_present` · framework/src/resources/maybe.rs:104
  - [ ] fn `suprnova::Maybe::map` · framework/src/resources/maybe.rs:109
  - [ ] fn `suprnova::Maybe::into_option` · framework/src/resources/maybe.rs:117
  - [ ] fn `suprnova::Maybe::as_ref` · framework/src/resources/maybe.rs:125
- [ ] type `suprnova::MissingValue` · framework/src/resources/maybe.rs:61 (also `suprnova::resources::MissingValue`, `suprnova::resources::maybe::MissingValue`)

### `suprnova::resources::response`

- [ ] struct `suprnova::JsonApi` · framework/src/resources/response.rs:283 (also `suprnova::resources::JsonApi`, `suprnova::resources::response::JsonApi`)
  - [ ] fn `suprnova::JsonApi::single` · framework/src/resources/response.rs:288
  - [ ] fn `suprnova::JsonApi::collection` · framework/src/resources/response.rs:294
  - [ ] fn `suprnova::JsonApi::paginated` · framework/src/resources/response.rs:300
- [ ] struct `suprnova::JsonApiResponse` · framework/src/resources/response.rs:22 (also `suprnova::resources::JsonApiResponse`, `suprnova::resources::response::JsonApiResponse`)
  - [ ] fn `suprnova::JsonApiResponse::status` · framework/src/resources/response.rs:39
  - [ ] fn `suprnova::JsonApiResponse::created` · framework/src/resources/response.rs:47
  - [ ] fn `suprnova::JsonApiResponse::with_meta` · framework/src/resources/response.rs:55
  - [ ] fn `suprnova::JsonApiResponse::meta` · framework/src/resources/response.rs:63
  - [ ] fn `suprnova::JsonApiResponse::with_meta_map` · framework/src/resources/response.rs:69
  - [ ] fn `suprnova::JsonApiResponse::with_link` · framework/src/resources/response.rs:78
  - [ ] fn `suprnova::JsonApiResponse::link` · framework/src/resources/response.rs:86
  - [ ] fn `suprnova::JsonApiResponse::with_link_value` · framework/src/resources/response.rs:93
  - [ ] fn `suprnova::JsonApiResponse::additional` · framework/src/resources/response.rs:104
  - [ ] fn `suprnova::JsonApiResponse::with_additional` · framework/src/resources/response.rs:112
  - [ ] fn `suprnova::JsonApiResponse::with_jsonapi` · framework/src/resources/response.rs:121
  - [ ] fn `suprnova::JsonApiResponse::render` · framework/src/resources/response.rs:132
- [ ] struct `suprnova::Resource` · framework/src/resources/response.rs:157 (also `suprnova::resources::Resource`, `suprnova::resources::response::Resource`)
  - [ ] fn `suprnova::Resource::single` · framework/src/resources/response.rs:162
  - [ ] fn `suprnova::Resource::collection` · framework/src/resources/response.rs:182
  - [ ] fn `suprnova::Resource::paginated` · framework/src/resources/response.rs:223

### `suprnova::resources::trait_def`

- [ ] struct `suprnova::IncludeResolutionError` · framework/src/resources/trait_def.rs:122 (also `suprnova::resources::IncludeResolutionError`, `suprnova::resources::trait_def::IncludeResolutionError`)
  - Public fields: `path`, `on_type`
- [ ] struct `suprnova::ResourceIdentifier` · framework/src/resources/trait_def.rs:92 (also `suprnova::resources::ResourceIdentifier`, `suprnova::resources::trait_def::ResourceIdentifier`)
  - Public fields: `resource_type`, `id`
  - [ ] fn `suprnova::ResourceIdentifier::new` · framework/src/resources/trait_def.rs:101
  - [ ] fn `suprnova::ResourceIdentifier::to_value` · framework/src/resources/trait_def.rs:110
- [ ] enum `suprnova::RelationshipValue` · framework/src/resources/trait_def.rs:80 (also `suprnova::resources::RelationshipValue`, `suprnova::resources::trait_def::RelationshipValue`)
  - Variants: `Single`, `Many`, `Null`
- [ ] trait `suprnova::IntoJsonResource` · framework/src/resources/trait_def.rs:14 (also `suprnova::resources::IntoJsonResource`, `suprnova::resources::trait_def::IntoJsonResource`)
  - [ ] fn `suprnova::IntoJsonResource::resource_type` · framework/src/resources/trait_def.rs:16 (required)
  - [ ] fn `suprnova::IntoJsonResource::resource_id` · framework/src/resources/trait_def.rs:22 (required)
  - [ ] fn `suprnova::IntoJsonResource::resource_attributes` · framework/src/resources/trait_def.rs:26 (required)
  - [ ] fn `suprnova::IntoJsonResource::resource_relationships` · framework/src/resources/trait_def.rs:32 (required)
  - [ ] fn `suprnova::IntoJsonResource::resource_included` · framework/src/resources/trait_def.rs:43 (required)
  - [ ] fn `suprnova::IntoJsonResource::resource_links` · framework/src/resources/trait_def.rs:54 (provided)
  - [ ] fn `suprnova::IntoJsonResource::resource_meta` · framework/src/resources/trait_def.rs:63 (provided)
  - [ ] fn `suprnova::IntoJsonResource::resource_top_level_meta` · framework/src/resources/trait_def.rs:73 (provided)

### `suprnova::resources`

- [ ] trait `suprnova::AsRelationshipValue` · framework/src/resources/mod.rs:76 (also `suprnova::resources::AsRelationshipValue`)
  - Implemented here by: `Option`, `Vec`
  - [ ] fn `suprnova::AsRelationshipValue::as_relationship_value` · framework/src/resources/mod.rs:80 (required)
- [ ] trait `suprnova::PushIncluded` · framework/src/resources/mod.rs:121 (also `suprnova::resources::PushIncluded`)
  - Implemented here by: `Option`, `Vec`
  - [ ] fn `suprnova::PushIncluded::push_included` · framework/src/resources/mod.rs:125 (required)
