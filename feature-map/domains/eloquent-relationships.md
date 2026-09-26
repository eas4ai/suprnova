# Feature map: `manual/eloquent-relationships.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 201 checked.

## Rust API: suprnova

### `suprnova::eloquent::relations::belongs_to_many`

- [ ] struct `suprnova::BelongsToMany` · framework/src/eloquent/relations/belongs_to_many.rs:87 (also `suprnova::eloquent::BelongsToMany`, `suprnova::eloquent::relations::BelongsToMany`, `suprnova::eloquent::relations::belongs_to_many::BelongsToMany`, `suprnova::relations::BelongsToMany`, `suprnova::relations::belongs_to_many::BelongsToMany`)
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::BelongsToMany::with_pivot` · framework/src/eloquent/relations/belongs_to_many.rs:238
  - [ ] fn `suprnova::BelongsToMany::with_timestamps` · framework/src/eloquent/relations/belongs_to_many.rs:252
  - [ ] fn `suprnova::BelongsToMany::foreign_key` · framework/src/eloquent/relations/belongs_to_many.rs:258
  - [ ] fn `suprnova::BelongsToMany::related_key` · framework/src/eloquent/relations/belongs_to_many.rs:264
  - [ ] fn `suprnova::BelongsToMany::local_key` · framework/src/eloquent/relations/belongs_to_many.rs:271
  - [ ] fn `suprnova::BelongsToMany::related_pk` · framework/src/eloquent/relations/belongs_to_many.rs:287
  - [ ] fn `suprnova::BelongsToMany::where_pivot` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_op` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_op` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_in` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_in` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_not_in` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_not_in` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_null` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_null` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_not_null` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_not_null` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_between` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_between` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_not_between` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_not_between` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_group` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_group` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::attach` · framework/src/eloquent/relations/belongs_to_many.rs:314
  - [ ] fn `suprnova::BelongsToMany::attach_with` · framework/src/eloquent/relations/belongs_to_many.rs:338
  - [ ] fn `suprnova::BelongsToMany::detach` · framework/src/eloquent/relations/belongs_to_many.rs:410
  - [ ] fn `suprnova::BelongsToMany::sync` · framework/src/eloquent/relations/belongs_to_many.rs:474
  - [ ] fn `suprnova::BelongsToMany::get` · framework/src/eloquent/relations/belongs_to_many.rs:673
  - [ ] fn `suprnova::BelongsToMany::first` · framework/src/eloquent/relations/belongs_to_many.rs:782
  - [ ] fn `suprnova::BelongsToMany::count` · framework/src/eloquent/relations/belongs_to_many.rs:789
  - [ ] fn `suprnova::BelongsToMany::with_trashed` · framework/src/eloquent/relations/belongs_to_many.rs:865
  - [ ] fn `suprnova::BelongsToMany::only_trashed` · framework/src/eloquent/relations/belongs_to_many.rs:871

### `suprnova::eloquent::relations::belongs_to`

- [ ] struct `suprnova::BelongsTo` · framework/src/eloquent/relations/belongs_to.rs:43 (also `suprnova::eloquent::BelongsTo`, `suprnova::eloquent::relations::BelongsTo`, `suprnova::eloquent::relations::belongs_to::BelongsTo`, `suprnova::relations::BelongsTo`, `suprnova::relations::belongs_to::BelongsTo`)
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::BelongsTo::foreign_key` · framework/src/eloquent/relations/belongs_to.rs:143
  - [ ] fn `suprnova::BelongsTo::owner_key` · framework/src/eloquent/relations/belongs_to.rs:149
  - [ ] fn `suprnova::BelongsTo::with_default` · framework/src/eloquent/relations/belongs_to.rs:162
  - [ ] fn `suprnova::BelongsTo::first` · framework/src/eloquent/relations/belongs_to.rs:182
  - [ ] fn `suprnova::BelongsTo::with_trashed` · framework/src/eloquent/relations/belongs_to.rs:238
  - [ ] fn `suprnova::BelongsTo::only_trashed` · framework/src/eloquent/relations/belongs_to.rs:244

### `suprnova::eloquent::relations::eager_cache`

- [ ] struct `suprnova::EagerLoadCache` · framework/src/eloquent/relations/eager_cache.rs:35 (also `suprnova::eloquent::EagerLoadCache`, `suprnova::eloquent::relations::EagerLoadCache`, `suprnova::eloquent::relations::eager_cache::EagerLoadCache`, `suprnova::relations::EagerLoadCache`, `suprnova::relations::eager_cache::EagerLoadCache`)
  - [ ] fn `suprnova::EagerLoadCache::new` · framework/src/eloquent/relations/eager_cache.rs:59
  - [ ] fn `suprnova::EagerLoadCache::has` · framework/src/eloquent/relations/eager_cache.rs:66
  - [ ] fn `suprnova::EagerLoadCache::set_many` · framework/src/eloquent/relations/eager_cache.rs:71
  - [ ] fn `suprnova::EagerLoadCache::get_many` · framework/src/eloquent/relations/eager_cache.rs:85
  - [ ] fn `suprnova::EagerLoadCache::get_many_mut` · framework/src/eloquent/relations/eager_cache.rs:118
  - [ ] fn `suprnova::EagerLoadCache::get_one_mut` · framework/src/eloquent/relations/eager_cache.rs:133
  - [ ] fn `suprnova::EagerLoadCache::set_one` · framework/src/eloquent/relations/eager_cache.rs:143
  - [ ] fn `suprnova::EagerLoadCache::take_many` · framework/src/eloquent/relations/eager_cache.rs:155
  - [ ] fn `suprnova::EagerLoadCache::take_one` · framework/src/eloquent/relations/eager_cache.rs:166
  - [ ] fn `suprnova::EagerLoadCache::get_one` · framework/src/eloquent/relations/eager_cache.rs:182
  - [ ] fn `suprnova::EagerLoadCache::set_count` · framework/src/eloquent/relations/eager_cache.rs:196
  - [ ] fn `suprnova::EagerLoadCache::get_count` · framework/src/eloquent/relations/eager_cache.rs:204
  - [ ] fn `suprnova::EagerLoadCache::set_aggregate` · framework/src/eloquent/relations/eager_cache.rs:220
  - [ ] fn `suprnova::EagerLoadCache::get_aggregate` · framework/src/eloquent/relations/eager_cache.rs:230

### `suprnova::eloquent::relations::has_many`

- [ ] struct `suprnova::HasMany` · framework/src/eloquent/relations/has_many.rs:50 (also `suprnova::eloquent::HasMany`, `suprnova::eloquent::relations::HasMany`, `suprnova::eloquent::relations::has_many::HasMany`, `suprnova::relations::HasMany`, `suprnova::relations::has_many::HasMany`)
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::HasMany::foreign_key` · framework/src/eloquent/relations/has_many.rs:148
  - [ ] fn `suprnova::HasMany::local_key` · framework/src/eloquent/relations/has_many.rs:159
  - [ ] fn `suprnova::HasMany::filter` · framework/src/eloquent/relations/has_many.rs:167
  - [ ] fn `suprnova::HasMany::db_where` · framework/src/eloquent/relations/has_many.rs:173
  - [ ] fn `suprnova::HasMany::order_by` · framework/src/eloquent/relations/has_many.rs:178
  - [ ] fn `suprnova::HasMany::latest` · framework/src/eloquent/relations/has_many.rs:189
  - [ ] fn `suprnova::HasMany::oldest` · framework/src/eloquent/relations/has_many.rs:195
  - [ ] fn `suprnova::HasMany::limit` · framework/src/eloquent/relations/has_many.rs:200
  - [ ] fn `suprnova::HasMany::take` · framework/src/eloquent/relations/has_many.rs:206
  - [ ] fn `suprnova::HasMany::first` · framework/src/eloquent/relations/has_many.rs:215
  - [ ] fn `suprnova::HasMany::get` · framework/src/eloquent/relations/has_many.rs:227
  - [ ] fn `suprnova::HasMany::count` · framework/src/eloquent/relations/has_many.rs:235
  - [ ] fn `suprnova::HasMany::with_trashed` · framework/src/eloquent/relations/has_many.rs:264
  - [ ] fn `suprnova::HasMany::only_trashed` · framework/src/eloquent/relations/has_many.rs:270

### `suprnova::eloquent::relations::has_one`

- [ ] struct `suprnova::HasOne` · framework/src/eloquent/relations/has_one.rs:42 (also `suprnova::eloquent::HasOne`, `suprnova::eloquent::relations::HasOne`, `suprnova::eloquent::relations::has_one::HasOne`, `suprnova::relations::HasOne`, `suprnova::relations::has_one::HasOne`)
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::HasOne::foreign_key` · framework/src/eloquent/relations/has_one.rs:138
  - [ ] fn `suprnova::HasOne::local_key` · framework/src/eloquent/relations/has_one.rs:149
  - [ ] fn `suprnova::HasOne::filter` · framework/src/eloquent/relations/has_one.rs:157
  - [ ] fn `suprnova::HasOne::db_where` · framework/src/eloquent/relations/has_one.rs:163
  - [ ] fn `suprnova::HasOne::first` · framework/src/eloquent/relations/has_one.rs:170
  - [ ] fn `suprnova::HasOne::get` · framework/src/eloquent/relations/has_one.rs:182
  - [ ] fn `suprnova::HasOne::with_trashed` · framework/src/eloquent/relations/has_one.rs:212
  - [ ] fn `suprnova::HasOne::only_trashed` · framework/src/eloquent/relations/has_one.rs:219

### `suprnova::eloquent::relations::morph_registry`

- [ ] fn `suprnova::find_morph_type` · framework/src/eloquent/relations/morph_registry.rs:76 (also `suprnova::eloquent::find_morph_type`, `suprnova::eloquent::relations::find_morph_type`, `suprnova::eloquent::relations::morph_registry::find_morph_type`, `suprnova::relations::find_morph_type`, `suprnova::relations::morph_registry::find_morph_type`)
- [ ] fn `suprnova::find_morph_type_by_id` · framework/src/eloquent/relations/morph_registry.rs:83 (also `suprnova::eloquent::find_morph_type_by_id`, `suprnova::eloquent::relations::find_morph_type_by_id`, `suprnova::eloquent::relations::morph_registry::find_morph_type_by_id`, `suprnova::relations::find_morph_type_by_id`, `suprnova::relations::morph_registry::find_morph_type_by_id`)
- [ ] fn `suprnova::morph_types` · framework/src/eloquent/relations/morph_registry.rs:48 (also `suprnova::eloquent::morph_types`, `suprnova::eloquent::relations::morph_registry::morph_types`, `suprnova::eloquent::relations::morph_types`, `suprnova::relations::morph_registry::morph_types`, `suprnova::relations::morph_types`)
- [ ] struct `suprnova::MorphTypeEntry` · framework/src/eloquent/relations/morph_registry.rs:29 (also `suprnova::eloquent::MorphTypeEntry`, `suprnova::eloquent::relations::MorphTypeEntry`, `suprnova::eloquent::relations::morph_registry::MorphTypeEntry`, `suprnova::relations::MorphTypeEntry`, `suprnova::relations::morph_registry::MorphTypeEntry`)
  - Public fields: `morph_type`, `type_name`, `table`, `type_id`

### `suprnova::eloquent::relations::morph_to_many`

- [ ] struct `suprnova::MorphedByMany` · framework/src/eloquent/relations/morph_to_many.rs:902 (also `suprnova::eloquent::MorphedByMany`, `suprnova::eloquent::relations::MorphedByMany`, `suprnova::eloquent::relations::morph_to_many::MorphedByMany`, `suprnova::relations::MorphedByMany`, `suprnova::relations::morph_to_many::MorphedByMany`)
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::MorphedByMany::related_pk` · framework/src/eloquent/relations/morph_to_many.rs:1029
  - [ ] fn `suprnova::MorphedByMany::local_key` · framework/src/eloquent/relations/morph_to_many.rs:1035
  - [ ] fn `suprnova::MorphedByMany::where_pivot` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_op` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_op` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_in` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_in` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_not_in` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_not_in` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_null` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_null` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_not_null` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_not_null` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_between` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_between` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_not_between` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_not_between` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_group` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_group` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::get` · framework/src/eloquent/relations/morph_to_many.rs:1063
  - [ ] fn `suprnova::MorphedByMany::first` · framework/src/eloquent/relations/morph_to_many.rs:1140
  - [ ] fn `suprnova::MorphedByMany::count` · framework/src/eloquent/relations/morph_to_many.rs:1145
  - [ ] fn `suprnova::MorphedByMany::with_trashed` · framework/src/eloquent/relations/morph_to_many.rs:1226
  - [ ] fn `suprnova::MorphedByMany::only_trashed` · framework/src/eloquent/relations/morph_to_many.rs:1232
- [ ] struct `suprnova::MorphToMany` · framework/src/eloquent/relations/morph_to_many.rs:111 (also `suprnova::eloquent::MorphToMany`, `suprnova::eloquent::relations::MorphToMany`, `suprnova::eloquent::relations::morph_to_many::MorphToMany`, `suprnova::relations::MorphToMany`, `suprnova::relations::morph_to_many::MorphToMany`)
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::MorphToMany::with_pivot` · framework/src/eloquent/relations/morph_to_many.rs:260
  - [ ] fn `suprnova::MorphToMany::with_timestamps` · framework/src/eloquent/relations/morph_to_many.rs:273
  - [ ] fn `suprnova::MorphToMany::local_key` · framework/src/eloquent/relations/morph_to_many.rs:280
  - [ ] fn `suprnova::MorphToMany::related_pk` · framework/src/eloquent/relations/morph_to_many.rs:288
  - [ ] fn `suprnova::MorphToMany::where_pivot` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_op` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_op` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_in` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_in` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_not_in` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_not_in` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_null` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_null` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_not_null` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_not_null` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_between` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_between` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_not_between` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_not_between` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_group` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_group` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::attach` · framework/src/eloquent/relations/morph_to_many.rs:311
  - [ ] fn `suprnova::MorphToMany::attach_with` · framework/src/eloquent/relations/morph_to_many.rs:328
  - [ ] fn `suprnova::MorphToMany::detach` · framework/src/eloquent/relations/morph_to_many.rs:394
  - [ ] fn `suprnova::MorphToMany::sync` · framework/src/eloquent/relations/morph_to_many.rs:444
  - [ ] fn `suprnova::MorphToMany::get` · framework/src/eloquent/relations/morph_to_many.rs:622
  - [ ] fn `suprnova::MorphToMany::first` · framework/src/eloquent/relations/morph_to_many.rs:740
  - [ ] fn `suprnova::MorphToMany::count` · framework/src/eloquent/relations/morph_to_many.rs:746
  - [ ] fn `suprnova::MorphToMany::with_trashed` · framework/src/eloquent/relations/morph_to_many.rs:828
  - [ ] fn `suprnova::MorphToMany::only_trashed` · framework/src/eloquent/relations/morph_to_many.rs:834

### `suprnova::eloquent::relations::morph`

- [ ] struct `suprnova::MorphMany` · framework/src/eloquent/relations/morph.rs:52 (also `suprnova::eloquent::MorphMany`, `suprnova::eloquent::relations::MorphMany`, `suprnova::eloquent::relations::morph::MorphMany`, `suprnova::relations::MorphMany`, `suprnova::relations::morph::MorphMany`)
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::MorphMany::filter` · framework/src/eloquent/relations/morph.rs:158
  - [ ] fn `suprnova::MorphMany::db_where` · framework/src/eloquent/relations/morph.rs:164
  - [ ] fn `suprnova::MorphMany::order_by` · framework/src/eloquent/relations/morph.rs:169
  - [ ] fn `suprnova::MorphMany::latest` · framework/src/eloquent/relations/morph.rs:176
  - [ ] fn `suprnova::MorphMany::oldest` · framework/src/eloquent/relations/morph.rs:181
  - [ ] fn `suprnova::MorphMany::limit` · framework/src/eloquent/relations/morph.rs:186
  - [ ] fn `suprnova::MorphMany::take` · framework/src/eloquent/relations/morph.rs:192
  - [ ] fn `suprnova::MorphMany::first` · framework/src/eloquent/relations/morph.rs:197
  - [ ] fn `suprnova::MorphMany::get` · framework/src/eloquent/relations/morph.rs:206
  - [ ] fn `suprnova::MorphMany::count` · framework/src/eloquent/relations/morph.rs:213
  - [ ] fn `suprnova::MorphMany::with_trashed` · framework/src/eloquent/relations/morph.rs:241
  - [ ] fn `suprnova::MorphMany::only_trashed` · framework/src/eloquent/relations/morph.rs:247
- [ ] struct `suprnova::MorphOne` · framework/src/eloquent/relations/morph.rs:306 (also `suprnova::eloquent::MorphOne`, `suprnova::eloquent::relations::MorphOne`, `suprnova::eloquent::relations::morph::MorphOne`, `suprnova::relations::MorphOne`, `suprnova::relations::morph::MorphOne`)
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::MorphOne::filter` · framework/src/eloquent/relations/morph.rs:359
  - [ ] fn `suprnova::MorphOne::db_where` · framework/src/eloquent/relations/morph.rs:365
  - [ ] fn `suprnova::MorphOne::order_by` · framework/src/eloquent/relations/morph.rs:370
  - [ ] fn `suprnova::MorphOne::first` · framework/src/eloquent/relations/morph.rs:376
  - [ ] fn `suprnova::MorphOne::with_trashed` · framework/src/eloquent/relations/morph.rs:403
  - [ ] fn `suprnova::MorphOne::only_trashed` · framework/src/eloquent/relations/morph.rs:409
- [ ] struct `suprnova::MorphTo` · framework/src/eloquent/relations/morph.rs:485 (also `suprnova::eloquent::MorphTo`, `suprnova::eloquent::relations::MorphTo`, `suprnova::eloquent::relations::morph::MorphTo`, `suprnova::relations::MorphTo`, `suprnova::relations::morph::MorphTo`)
  - Public fields: `morph_id`, `morph_type`
  - Implements: `suprnova::Relation`

### `suprnova::eloquent::relations::through`

- [ ] struct `suprnova::HasManyThrough` · framework/src/eloquent/relations/through.rs:84 (also `suprnova::eloquent::HasManyThrough`, `suprnova::eloquent::relations::HasManyThrough`, `suprnova::eloquent::relations::through::HasManyThrough`, `suprnova::relations::HasManyThrough`, `suprnova::relations::through::HasManyThrough`)
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::HasManyThrough::first_key` · framework/src/eloquent/relations/through.rs:173
  - [ ] fn `suprnova::HasManyThrough::second_key` · framework/src/eloquent/relations/through.rs:179
  - [ ] fn `suprnova::HasManyThrough::local_key` · framework/src/eloquent/relations/through.rs:188
  - [ ] fn `suprnova::HasManyThrough::second_local_key` · framework/src/eloquent/relations/through.rs:196
  - [ ] fn `suprnova::HasManyThrough::get` · framework/src/eloquent/relations/through.rs:232
  - [ ] fn `suprnova::HasManyThrough::first` · framework/src/eloquent/relations/through.rs:259
  - [ ] fn `suprnova::HasManyThrough::count` · framework/src/eloquent/relations/through.rs:269
- [ ] struct `suprnova::HasOneThrough` · framework/src/eloquent/relations/through.rs:399 (also `suprnova::eloquent::HasOneThrough`, `suprnova::eloquent::relations::HasOneThrough`, `suprnova::eloquent::relations::through::HasOneThrough`, `suprnova::relations::HasOneThrough`, `suprnova::relations::through::HasOneThrough`)
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::HasOneThrough::first_key` · framework/src/eloquent/relations/through.rs:454
  - [ ] fn `suprnova::HasOneThrough::second_key` · framework/src/eloquent/relations/through.rs:460
  - [ ] fn `suprnova::HasOneThrough::local_key` · framework/src/eloquent/relations/through.rs:466
  - [ ] fn `suprnova::HasOneThrough::second_local_key` · framework/src/eloquent/relations/through.rs:472
  - [ ] fn `suprnova::HasOneThrough::first` · framework/src/eloquent/relations/through.rs:480
  - [ ] fn `suprnova::HasOneThrough::get` · framework/src/eloquent/relations/through.rs:486

### `suprnova::eloquent::relations`

- [ ] fn `suprnova::eloquent::aggregate_cache_key` · framework/src/eloquent/relations/mod.rs:167 (also `suprnova::eloquent::relations::aggregate_cache_key`, `suprnova::relations::aggregate_cache_key`)
- [ ] fn `suprnova::find_relation` · framework/src/eloquent/relations/mod.rs:339 (also `suprnova::eloquent::find_relation`, `suprnova::eloquent::relations::find_relation`, `suprnova::relations::find_relation`)
- [ ] fn `suprnova::relations` · framework/src/eloquent/relations/mod.rs:327 (also `suprnova::eloquent::relations`, `suprnova::eloquent::relations::relations`, `suprnova::relations::relations`)
- [ ] fn `suprnova::relations_of` · framework/src/eloquent/relations/mod.rs:332 (also `suprnova::eloquent::relations::relations_of`, `suprnova::eloquent::relations_of`, `suprnova::relations::relations_of`)
- [ ] fn `suprnova::eloquent::touch_column` · framework/src/eloquent/relations/mod.rs:189 (also `suprnova::eloquent::relations::touch_column`, `suprnova::relations::touch_column`)
- [ ] struct `suprnova::RelationEntry` · framework/src/eloquent/relations/mod.rs:261 (also `suprnova::eloquent::RelationEntry`, `suprnova::eloquent::relations::RelationEntry`, `suprnova::relations::RelationEntry`)
  - Public fields: `parent_type`, `target_type`, `name`, `kind`, `parent_type_name`, `target_type_name`, `target_table`, `foreign_key`, `parent_key`, `pivot_table`, `pivot_parent_key`, `pivot_related_key`, `morph_type_column`, `morph_type_value`, `target_primary_key`, `related_soft_deletes_column`, `related_updated_at_column`
- [ ] enum `suprnova::AggregateKind` · framework/src/eloquent/relations/mod.rs:126 (also `suprnova::eloquent::AggregateKind`, `suprnova::eloquent::relations::AggregateKind`, `suprnova::relations::AggregateKind`)
  - Variants: `Sum`, `Avg`, `Min`, `Max`
  - [ ] fn `suprnova::AggregateKind::as_key_str` · framework/src/eloquent/relations/mod.rs:142
- [ ] enum `suprnova::RelationKind` · framework/src/eloquent/relations/mod.rs:96 (also `suprnova::eloquent::RelationKind`, `suprnova::eloquent::relations::RelationKind`, `suprnova::relations::RelationKind`)
  - Variants: `HasOne`, `BelongsTo`, `HasMany`, `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphTo`, `MorphOne`, `MorphMany`, `MorphToMany`, `MorphedByMany`
- [ ] trait `suprnova::EagerLoadDispatch` · framework/src/eloquent/relations/mod.rs:459 (also `suprnova::eloquent::EagerLoadDispatch`, `suprnova::eloquent::relations::EagerLoadDispatch`, `suprnova::relations::EagerLoadDispatch`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] fn `suprnova::EagerLoadDispatch::eager_load` · framework/src/eloquent/relations/mod.rs:461 (required)
  - [ ] fn `suprnova::EagerLoadDispatch::count_relation` · framework/src/eloquent/relations/mod.rs:469 (required)
  - [ ] fn `suprnova::EagerLoadDispatch::aggregate_relation` · framework/src/eloquent/relations/mod.rs:476 (required)
  - [ ] fn `suprnova::EagerLoadDispatch::recurse_eager_load` · framework/src/eloquent/relations/mod.rs:497 (required)
  - [ ] fn `suprnova::EagerLoadDispatch::recurse_eager_load_batched` · framework/src/eloquent/relations/mod.rs:516 (required)
  - [ ] fn `suprnova::EagerLoadDispatch::set_pivot_arc` · framework/src/eloquent/relations/mod.rs:534 (required)
  - [ ] fn `suprnova::EagerLoadDispatch::has_eager` · framework/src/eloquent/relations/mod.rs:544 (required)
- [ ] trait `suprnova::Relation` · framework/src/eloquent/relations/mod.rs:207 (also `suprnova::eloquent::Relation`, `suprnova::eloquent::relations::Relation`, `suprnova::relations::Relation`)
  - Implemented here by: `BelongsTo`, `BelongsToMany`, `HasMany`, `HasManyThrough`, `HasOne`, `HasOneThrough`, `MorphMany`, `MorphOne`, `MorphTo`, `MorphToMany`, `MorphedByMany`
  - [ ] type `suprnova::Relation::Parent` · framework/src/eloquent/relations/mod.rs:209
  - [ ] type `suprnova::Relation::Target` · framework/src/eloquent/relations/mod.rs:211
  - [ ] const `suprnova::Relation::KIND` · framework/src/eloquent/relations/mod.rs:213
  - [ ] fn `suprnova::Relation::parent_key` · framework/src/eloquent/relations/mod.rs:217 (required)
  - [ ] fn `suprnova::Relation::foreign_key` · framework/src/eloquent/relations/mod.rs:223 (required)
