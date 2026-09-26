# Feature map: `manual/queries.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 175 checked.

## Rust API: suprnova

### Re-exported from other crates

- [ ] trait `suprnova::Iden` re-exports `sea_query::types::iden::core::Iden`
- [ ] proc derive `suprnova::Iden` re-exports `sea_query_derive::Iden`
- [ ] module `suprnova::sea_query` re-exports `sea_query`

### `suprnova::eloquent::builder`

- [ ] struct `suprnova::Builder` · framework/src/eloquent/builder.rs:400 (also `suprnova::eloquent::Builder`, `suprnova::eloquent::builder::Builder`)
  - [ ] fn `suprnova::Builder::new` · framework/src/eloquent/builder.rs:901
  - [ ] fn `suprnova::Builder::on` · framework/src/eloquent/builder.rs:951
  - [ ] fn `suprnova::Builder::on_write_connection` · framework/src/eloquent/builder.rs:976
  - [ ] fn `suprnova::Builder::with_tx` · framework/src/eloquent/builder.rs:998
  - [ ] fn `suprnova::Builder::with` · framework/src/eloquent/builder.rs:1020
  - [ ] fn `suprnova::Builder::with_count` · framework/src/eloquent/builder.rs:1046
  - [ ] fn `suprnova::Builder::with_sum` · framework/src/eloquent/builder.rs:1071
  - [ ] fn `suprnova::Builder::with_avg` · framework/src/eloquent/builder.rs:1080
  - [ ] fn `suprnova::Builder::with_min` · framework/src/eloquent/builder.rs:1089
  - [ ] fn `suprnova::Builder::with_max` · framework/src/eloquent/builder.rs:1097
  - [ ] fn `suprnova::Builder::with_where` · framework/src/eloquent/builder.rs:1124
  - [ ] fn `suprnova::Builder::filter` · framework/src/eloquent/builder.rs:1184
  - [ ] fn `suprnova::Builder::db_where` · framework/src/eloquent/builder.rs:1192
  - [ ] fn `suprnova::Builder::filter_op` · framework/src/eloquent/builder.rs:1204
  - [ ] fn `suprnova::Builder::db_where_op` · framework/src/eloquent/builder.rs:1215
  - [ ] fn `suprnova::Builder::or_filter` · framework/src/eloquent/builder.rs:1223
  - [ ] fn `suprnova::Builder::or_where` · framework/src/eloquent/builder.rs:1245
  - [ ] fn `suprnova::Builder::filter_not` · framework/src/eloquent/builder.rs:1251
  - [ ] fn `suprnova::Builder::where_not` · framework/src/eloquent/builder.rs:1261
  - [ ] fn `suprnova::Builder::filter_in` · framework/src/eloquent/builder.rs:1270
  - [ ] fn `suprnova::Builder::where_in` · framework/src/eloquent/builder.rs:1282
  - [ ] fn `suprnova::Builder::filter_not_in` · framework/src/eloquent/builder.rs:1293
  - [ ] fn `suprnova::Builder::where_not_in` · framework/src/eloquent/builder.rs:1305
  - [ ] fn `suprnova::Builder::filter_between` · framework/src/eloquent/builder.rs:1318
  - [ ] fn `suprnova::Builder::where_between` · framework/src/eloquent/builder.rs:1334
  - [ ] fn `suprnova::Builder::filter_not_between` · framework/src/eloquent/builder.rs:1344
  - [ ] fn `suprnova::Builder::where_not_between` · framework/src/eloquent/builder.rs:1360
  - [ ] fn `suprnova::Builder::filter_null` · framework/src/eloquent/builder.rs:1372
  - [ ] fn `suprnova::Builder::where_null` · framework/src/eloquent/builder.rs:1379
  - [ ] fn `suprnova::Builder::filter_not_null` · framework/src/eloquent/builder.rs:1385
  - [ ] fn `suprnova::Builder::where_not_null` · framework/src/eloquent/builder.rs:1392
  - [ ] fn `suprnova::Builder::filter_like` · framework/src/eloquent/builder.rs:1401
  - [ ] fn `suprnova::Builder::where_like` · framework/src/eloquent/builder.rs:1409
  - [ ] fn `suprnova::Builder::filter_not_like` · framework/src/eloquent/builder.rs:1415
  - [ ] fn `suprnova::Builder::where_not_like` · framework/src/eloquent/builder.rs:1423
  - [ ] fn `suprnova::Builder::filter_binary` · framework/src/eloquent/builder.rs:1439
  - [ ] fn `suprnova::Builder::where_binary` · framework/src/eloquent/builder.rs:1447
  - [ ] fn `suprnova::Builder::or_filter_binary` · framework/src/eloquent/builder.rs:1455
  - [ ] fn `suprnova::Builder::or_where_binary` · framework/src/eloquent/builder.rs:1462
  - [ ] fn `suprnova::Builder::filter_not_binary` · framework/src/eloquent/builder.rs:1469
  - [ ] fn `suprnova::Builder::where_not_binary` · framework/src/eloquent/builder.rs:1477
  - [ ] fn `suprnova::Builder::or_filter_not_binary` · framework/src/eloquent/builder.rs:1483
  - [ ] fn `suprnova::Builder::or_where_not_binary` · framework/src/eloquent/builder.rs:1490
  - [ ] fn `suprnova::Builder::filter_date` · framework/src/eloquent/builder.rs:1499
  - [ ] fn `suprnova::Builder::where_date` · framework/src/eloquent/builder.rs:1510
  - [ ] fn `suprnova::Builder::filter_day` · framework/src/eloquent/builder.rs:1516
  - [ ] fn `suprnova::Builder::where_day` · framework/src/eloquent/builder.rs:1527
  - [ ] fn `suprnova::Builder::filter_month` · framework/src/eloquent/builder.rs:1533
  - [ ] fn `suprnova::Builder::where_month` · framework/src/eloquent/builder.rs:1544
  - [ ] fn `suprnova::Builder::filter_year` · framework/src/eloquent/builder.rs:1550
  - [ ] fn `suprnova::Builder::where_year` · framework/src/eloquent/builder.rs:1561
  - [ ] fn `suprnova::Builder::filter_time` · framework/src/eloquent/builder.rs:1567
  - [ ] fn `suprnova::Builder::where_time` · framework/src/eloquent/builder.rs:1578
  - [ ] fn `suprnova::Builder::filter_json_contains` · framework/src/eloquent/builder.rs:1588
  - [ ] fn `suprnova::Builder::where_json_contains` · framework/src/eloquent/builder.rs:1596
  - [ ] fn `suprnova::Builder::filter_json_length` · framework/src/eloquent/builder.rs:1603
  - [ ] fn `suprnova::Builder::where_json_length` · framework/src/eloquent/builder.rs:1611
  - [ ] fn `suprnova::Builder::filter_column` · framework/src/eloquent/builder.rs:1619
  - [ ] fn `suprnova::Builder::where_column` · framework/src/eloquent/builder.rs:1627
  - [ ] fn `suprnova::Builder::filter_raw` · framework/src/eloquent/builder.rs:1646
  - [ ] fn `suprnova::Builder::where_raw` · framework/src/eloquent/builder.rs:1653
  - [ ] fn `suprnova::Builder::order_by` · framework/src/eloquent/builder.rs:1660
  - [ ] fn `suprnova::Builder::order_by_desc` · framework/src/eloquent/builder.rs:1666
  - [ ] fn `suprnova::Builder::order_by_asc` · framework/src/eloquent/builder.rs:1671
  - [ ] fn `suprnova::Builder::order_by_raw` · framework/src/eloquent/builder.rs:1685
  - [ ] fn `suprnova::Builder::in_random_order` · framework/src/eloquent/builder.rs:1693
  - [ ] fn `suprnova::Builder::in_order_of` · framework/src/eloquent/builder.rs:1722
  - [ ] fn `suprnova::Builder::group_by` · framework/src/eloquent/builder.rs:1737
  - [ ] fn `suprnova::Builder::having` · framework/src/eloquent/builder.rs:1743
  - [ ] fn `suprnova::Builder::having_op` · framework/src/eloquent/builder.rs:1751
  - [ ] fn `suprnova::Builder::limit` · framework/src/eloquent/builder.rs:1761
  - [ ] fn `suprnova::Builder::offset` · framework/src/eloquent/builder.rs:1767
  - [ ] fn `suprnova::Builder::take` · framework/src/eloquent/builder.rs:1773
  - [ ] fn `suprnova::Builder::skip` · framework/src/eloquent/builder.rs:1778
  - [ ] fn `suprnova::Builder::distinct` · framework/src/eloquent/builder.rs:1783
  - [ ] fn `suprnova::Builder::select` · framework/src/eloquent/builder.rs:1790
  - [ ] fn `suprnova::Builder::add_select` · framework/src/eloquent/builder.rs:1801
  - [ ] fn `suprnova::Builder::select_raw` · framework/src/eloquent/builder.rs:1817
  - [ ] fn `suprnova::Builder::union` · framework/src/eloquent/builder.rs:1824
  - [ ] fn `suprnova::Builder::union_all` · framework/src/eloquent/builder.rs:1830
  - [ ] fn `suprnova::Builder::lock_for_update` · framework/src/eloquent/builder.rs:1863
  - [ ] fn `suprnova::Builder::shared_lock` · framework/src/eloquent/builder.rs:1881
  - [ ] fn `suprnova::Builder::with_casts` · framework/src/eloquent/builder.rs:1898
  - [ ] fn `suprnova::Builder::has` · framework/src/eloquent/builder.rs:3010
  - [ ] fn `suprnova::Builder::has_count` · framework/src/eloquent/builder.rs:3024
  - [ ] fn `suprnova::Builder::or_has` · framework/src/eloquent/builder.rs:3040
  - [ ] fn `suprnova::Builder::doesnt_have` · framework/src/eloquent/builder.rs:3050
  - [ ] fn `suprnova::Builder::or_doesnt_have` · framework/src/eloquent/builder.rs:3058
  - [ ] fn `suprnova::Builder::where_has` · framework/src/eloquent/builder.rs:3076
  - [ ] fn `suprnova::Builder::or_where_has` · framework/src/eloquent/builder.rs:3097
  - [ ] fn `suprnova::Builder::where_doesnt_have` · framework/src/eloquent/builder.rs:3119
  - [ ] fn `suprnova::Builder::or_where_doesnt_have` · framework/src/eloquent/builder.rs:3140
  - [ ] fn `suprnova::Builder::where_relation` · framework/src/eloquent/builder.rs:3165
  - [ ] fn `suprnova::Builder::where_relation_op` · framework/src/eloquent/builder.rs:3187
  - [ ] fn `suprnova::Builder::or_where_relation` · framework/src/eloquent/builder.rs:3209
  - [ ] fn `suprnova::Builder::where_belongs_to` · framework/src/eloquent/builder.rs:3238
  - [ ] fn `suprnova::Builder::where_key` · framework/src/eloquent/builder.rs:3255
  - [ ] fn `suprnova::Builder::filter_key` · framework/src/eloquent/builder.rs:3261
  - [ ] fn `suprnova::Builder::where_key_not` · framework/src/eloquent/builder.rs:3267
  - [ ] fn `suprnova::Builder::filter_key_not` · framework/src/eloquent/builder.rs:3273
  - [ ] fn `suprnova::Builder::or_where_key` · framework/src/eloquent/builder.rs:3281
  - [ ] fn `suprnova::Builder::or_filter_key` · framework/src/eloquent/builder.rs:3287
  - [ ] fn `suprnova::Builder::or_where_key_not` · framework/src/eloquent/builder.rs:3296
  - [ ] fn `suprnova::Builder::or_filter_key_not` · framework/src/eloquent/builder.rs:3303
  - [ ] fn `suprnova::Builder::latest` · framework/src/eloquent/builder.rs:3309
  - [ ] fn `suprnova::Builder::latest_by` · framework/src/eloquent/builder.rs:3314
  - [ ] fn `suprnova::Builder::oldest` · framework/src/eloquent/builder.rs:3320
  - [ ] fn `suprnova::Builder::oldest_by` · framework/src/eloquent/builder.rs:3325
  - [ ] fn `suprnova::Builder::without` · framework/src/eloquent/builder.rs:3333
  - [ ] fn `suprnova::Builder::with_only` · framework/src/eloquent/builder.rs:3354
  - [ ] fn `suprnova::Builder::qualify_column` · framework/src/eloquent/builder.rs:3371
  - [ ] fn `suprnova::Builder::qualify_columns` · framework/src/eloquent/builder.rs:3377
  - [ ] fn `suprnova::Builder::to_sql` · framework/src/eloquent/builder.rs:3391
  - [ ] fn `suprnova::Builder::to_sql_with_bindings` · framework/src/eloquent/builder.rs:3407
  - [ ] fn `suprnova::Builder::to_sql_for` · framework/src/eloquent/builder.rs:3419
  - [ ] fn `suprnova::Builder::to_sql_with_bindings_for` · framework/src/eloquent/builder.rs:3434
  - [ ] fn `suprnova::Builder::try_to_sql_with_bindings_for` · framework/src/eloquent/builder.rs:3450
  - [ ] fn `suprnova::Builder::dump` · framework/src/eloquent/builder.rs:3478
  - [ ] fn `suprnova::Builder::dd` · framework/src/eloquent/builder.rs:3522
  - [ ] fn `suprnova::Builder::to_delete_sql_with_bindings_for` · framework/src/eloquent/builder.rs:3553
  - [ ] fn `suprnova::Builder::get` · framework/src/eloquent/builder.rs:3682
  - [ ] fn `suprnova::Builder::first` · framework/src/eloquent/builder.rs:3809
  - [ ] fn `suprnova::Builder::first_or_fail` · framework/src/eloquent/builder.rs:3819
  - [ ] fn `suprnova::Builder::sole` · framework/src/eloquent/builder.rs:3832
  - [ ] fn `suprnova::Builder::sole_value` · framework/src/eloquent/builder.rs:3849
  - [ ] fn `suprnova::Builder::value_or_fail` · framework/src/eloquent/builder.rs:3880
  - [ ] fn `suprnova::Builder::exists` · framework/src/eloquent/builder.rs:3894
  - [ ] fn `suprnova::Builder::doesnt_exist` · framework/src/eloquent/builder.rs:3899
  - [ ] fn `suprnova::Builder::count` · framework/src/eloquent/builder.rs:3904
  - [ ] fn `suprnova::Builder::paginate` · framework/src/eloquent/builder.rs:3938
  - [ ] fn `suprnova::Builder::paginate_using` · framework/src/eloquent/builder.rs:3957
  - [ ] fn `suprnova::Builder::simple_paginate` · framework/src/eloquent/builder.rs:4021
  - [ ] fn `suprnova::Builder::cursor_paginate` · framework/src/eloquent/builder.rs:4065
  - [ ] fn `suprnova::Builder::chunk` · framework/src/eloquent/builder.rs:4181
  - [ ] fn `suprnova::Builder::chunk_by_id` · framework/src/eloquent/builder.rs:4250
  - [ ] fn `suprnova::Builder::chunk_map` · framework/src/eloquent/builder.rs:4319
  - [ ] fn `suprnova::Builder::each` · framework/src/eloquent/builder.rs:4373
  - [ ] fn `suprnova::Builder::lazy` · framework/src/eloquent/builder.rs:4432
  - [ ] fn `suprnova::Builder::lazy_by_id` · framework/src/eloquent/builder.rs:4444
  - [ ] fn `suprnova::Builder::cursor` · framework/src/eloquent/builder.rs:4493
  - [ ] fn `suprnova::Builder::sum` · framework/src/eloquent/builder.rs:4508
  - [ ] fn `suprnova::Builder::avg` · framework/src/eloquent/builder.rs:4520
  - [ ] fn `suprnova::Builder::min` · framework/src/eloquent/builder.rs:4531
  - [ ] fn `suprnova::Builder::max` · framework/src/eloquent/builder.rs:4542
  - [ ] fn `suprnova::Builder::value` · framework/src/eloquent/builder.rs:4553
  - [ ] fn `suprnova::Builder::pluck` · framework/src/eloquent/builder.rs:4576
  - [ ] fn `suprnova::Builder::pluck_keyed` · framework/src/eloquent/builder.rs:4600
  - [ ] fn `suprnova::Builder::model_keys` · framework/src/eloquent/builder.rs:4645
  - [ ] fn `suprnova::Builder::update_all` · framework/src/eloquent/builder.rs:4739
  - [ ] fn `suprnova::Builder::delete_all` · framework/src/eloquent/builder.rs:4801
  - [ ] fn `suprnova::Builder::increment_each` · framework/src/eloquent/builder.rs:4831
  - [ ] fn `suprnova::Builder::decrement_each` · framework/src/eloquent/builder.rs:4892
  - [ ] fn `suprnova::Builder::upsert` · framework/src/eloquent/builder.rs:4915
  - [ ] fn `suprnova::Builder::with_trashed` · framework/src/eloquent/soft_deletes.rs:109
  - [ ] fn `suprnova::Builder::only_trashed` · framework/src/eloquent/soft_deletes.rs:120
- [ ] enum `suprnova::Direction` · framework/src/eloquent/builder.rs:137 (also `suprnova::eloquent::Direction`, `suprnova::eloquent::builder::Direction`)
  - Variants: `Asc`, `Desc`
- [ ] trait `suprnova::IntoColumn` · framework/src/eloquent/builder.rs:92 (also `suprnova::eloquent::IntoColumn`, `suprnova::eloquent::builder::IntoColumn`)
  - Implemented here by: `String`, `features::entity::Column`, `payments::entities::customer::Column`, `payments::entities::payment_method::Column`, `payments::entities::subscription::Column`, `payments::entities::subscription_item::Column`, `payments::entities::transaction::Column`, `payments::entities::webhook_event::Column`, `rbac::entity::ModelPermissionColumn`, `rbac::entity::ModelRoleColumn`, `rbac::entity::PermissionColumn`, `rbac::entity::RoleColumn`, `rbac::entity::RolePermissionColumn`
  - [ ] fn `suprnova::IntoColumn::col_name` · framework/src/eloquent/builder.rs:96 (required)
- [ ] trait `suprnova::IntoVal` · framework/src/eloquent/builder.rs:121 (also `suprnova::eloquent::IntoVal`, `suprnova::eloquent::builder::IntoVal`)
  - [ ] fn `suprnova::IntoVal::into_val` · framework/src/eloquent/builder.rs:123 (required)

### `suprnova::eloquent::lazy`

- [ ] struct `suprnova::LazyCollection` · framework/src/eloquent/lazy.rs:57 (also `suprnova::eloquent::LazyCollection`, `suprnova::eloquent::lazy::LazyCollection`)
  - [ ] fn `suprnova::LazyCollection::boxed` · framework/src/eloquent/lazy.rs:67
  - [ ] fn `suprnova::LazyCollection::next` · framework/src/eloquent/lazy.rs:76

### `suprnova::eloquent::scopes`

- [ ] struct `suprnova::ScopeRegistry` · framework/src/eloquent/scopes.rs:196 (also `suprnova::eloquent::ScopeRegistry`, `suprnova::eloquent::scopes::ScopeRegistry`)
  - [ ] fn `suprnova::ScopeRegistry::register` · framework/src/eloquent/scopes.rs:210
  - [ ] fn `suprnova::ScopeRegistry::apply_to` · framework/src/eloquent/scopes.rs:294
- [ ] enum `suprnova::eloquent::scopes::ScopeDependency` · framework/src/eloquent/scopes.rs:125
  - Variants: `Constant`, `PerRequest`
- [ ] trait `suprnova::GlobalScope` · framework/src/eloquent/scopes.rs:88 (also `suprnova::eloquent::GlobalScope`, `suprnova::eloquent::scopes::GlobalScope`)
  - [ ] fn `suprnova::GlobalScope::apply` · framework/src/eloquent/scopes.rs:104 (required)
  - [ ] fn `suprnova::GlobalScope::dependency` · framework/src/eloquent/scopes.rs:112 (provided)

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::scopes` · suprnova-macros/src/lib.rs:1118 (re-exported as `suprnova::scopes`)
  - Form: attribute `#[scopes]`
