# Feature map: `manual/eloquent-collections.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 56 checked.

## Rust API: suprnova

### `suprnova::eloquent::collection`

- [ ] struct `suprnova::Collection` · framework/src/eloquent/collection.rs:42 (also `suprnova::eloquent::Collection`, `suprnova::eloquent::collection::Collection`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::Collection::new` · framework/src/eloquent/collection.rs:46
  - [ ] fn `suprnova::Collection::into_vec` · framework/src/eloquent/collection.rs:51
  - [ ] fn `suprnova::Collection::as_slice` · framework/src/eloquent/collection.rs:58
  - [ ] fn `suprnova::Collection::from_vec` · framework/src/eloquent/collection.rs:72
  - [ ] fn `suprnova::Collection::len` · framework/src/eloquent/collection.rs:78
  - [ ] fn `suprnova::Collection::is_empty` · framework/src/eloquent/collection.rs:83
  - [ ] fn `suprnova::Collection::is_not_empty` · framework/src/eloquent/collection.rs:89
  - [ ] fn `suprnova::Collection::first` · framework/src/eloquent/collection.rs:94
  - [ ] fn `suprnova::Collection::last` · framework/src/eloquent/collection.rs:99
  - [ ] fn `suprnova::Collection::first_where` · framework/src/eloquent/collection.rs:105
  - [ ] fn `suprnova::Collection::last_where` · framework/src/eloquent/collection.rs:114
  - [ ] fn `suprnova::Collection::each` · framework/src/eloquent/collection.rs:129
  - [ ] fn `suprnova::Collection::map` · framework/src/eloquent/collection.rs:140
  - [ ] fn `suprnova::Collection::map_to_map` · framework/src/eloquent/collection.rs:149
  - [ ] fn `suprnova::Collection::filter` · framework/src/eloquent/collection.rs:158
  - [ ] fn `suprnova::Collection::reject` · framework/src/eloquent/collection.rs:167
  - [ ] fn `suprnova::Collection::reduce` · framework/src/eloquent/collection.rs:175
  - [ ] fn `suprnova::Collection::group_by_with` · framework/src/eloquent/collection.rs:185
  - [ ] fn `suprnova::Collection::key_by_with` · framework/src/eloquent/collection.rs:202
  - [ ] fn `suprnova::Collection::pluck_by` · framework/src/eloquent/collection.rs:220
  - [ ] fn `suprnova::Collection::sort_with` · framework/src/eloquent/collection.rs:229
  - [ ] fn `suprnova::Collection::unique` · framework/src/eloquent/collection.rs:240
  - [ ] fn `suprnova::Collection::unique_by` · framework/src/eloquent/collection.rs:255
  - [ ] fn `suprnova::Collection::contains_where` · framework/src/eloquent/collection.rs:267
  - [ ] fn `suprnova::Collection::chunk` · framework/src/eloquent/collection.rs:278
  - [ ] fn `suprnova::Collection::take` · framework/src/eloquent/collection.rs:293
  - [ ] fn `suprnova::Collection::skip` · framework/src/eloquent/collection.rs:299
  - [ ] fn `suprnova::Collection::slice` · framework/src/eloquent/collection.rs:305
  - [ ] fn `suprnova::Collection::reverse` · framework/src/eloquent/collection.rs:310
  - [ ] fn `suprnova::Collection::shuffle` · framework/src/eloquent/collection.rs:316
  - [ ] fn `suprnova::Collection::random` · framework/src/eloquent/collection.rs:324
  - [ ] fn `suprnova::Collection::random_n` · framework/src/eloquent/collection.rs:333
  - [ ] fn `suprnova::Collection::concat` · framework/src/eloquent/collection.rs:345
  - [ ] fn `suprnova::Collection::merge` · framework/src/eloquent/collection.rs:351
  - [ ] fn `suprnova::Collection::diff` · framework/src/eloquent/collection.rs:357
  - [ ] fn `suprnova::Collection::intersect` · framework/src/eloquent/collection.rs:371
  - [ ] fn `suprnova::Collection::load` · framework/src/eloquent/collection.rs:482
  - [ ] fn `suprnova::Collection::load_missing` · framework/src/eloquent/collection.rs:522
  - [ ] fn `suprnova::Collection::pluck` · framework/src/eloquent/collection.rs:606
  - [ ] fn `suprnova::Collection::model_keys` · framework/src/eloquent/collection.rs:635
  - [ ] fn `suprnova::Collection::pluck_keyed` · framework/src/eloquent/collection.rs:653
  - [ ] fn `suprnova::Collection::group_by` · framework/src/eloquent/collection.rs:681
  - [ ] fn `suprnova::Collection::key_by` · framework/src/eloquent/collection.rs:699
  - [ ] fn `suprnova::Collection::sort_by` · framework/src/eloquent/collection.rs:724
  - [ ] fn `suprnova::Collection::sort_by_desc` · framework/src/eloquent/collection.rs:735
  - [ ] fn `suprnova::Collection::where_eq` · framework/src/eloquent/collection.rs:746
  - [ ] fn `suprnova::Collection::where_in` · framework/src/eloquent/collection.rs:757
  - [ ] fn `suprnova::Collection::where_not_in` · framework/src/eloquent/collection.rs:773
  - [ ] fn `suprnova::Collection::sum` · framework/src/eloquent/collection.rs:790
  - [ ] fn `suprnova::Collection::avg` · framework/src/eloquent/collection.rs:808
  - [ ] fn `suprnova::Collection::min` · framework/src/eloquent/collection.rs:829
  - [ ] fn `suprnova::Collection::max` · framework/src/eloquent/collection.rs:852
  - [ ] fn `suprnova::Collection::to_array` · framework/src/eloquent/collection.rs:883
  - [ ] fn `suprnova::Collection::to_json` · framework/src/eloquent/collection.rs:900
  - [ ] fn `suprnova::Collection::try_to_json` · framework/src/eloquent/collection.rs:908
