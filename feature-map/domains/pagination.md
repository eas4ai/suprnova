# Feature map: `manual/pagination.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 51 checked.

## Rust API: suprnova

### `suprnova::pagination::cursor`

- [ ] struct `suprnova::CursorPaginator` · framework/src/pagination/cursor.rs:87 (also `suprnova::pagination::CursorPaginator`, `suprnova::pagination::cursor::CursorPaginator`)
  - Public fields: `data`, `per_page`, `next_cursor`, `prev_cursor`, `path`, `cursor_name`
  - Implements: `suprnova::IntoInertiaScroll`, `suprnova::Paginated`, `suprnova::ProvidesScrollMetadata`
  - [ ] fn `suprnova::CursorPaginator::new` · framework/src/pagination/cursor.rs:119
  - [ ] fn `suprnova::CursorPaginator::with_path` · framework/src/pagination/cursor.rs:137
  - [ ] fn `suprnova::CursorPaginator::with_cursor_name` · framework/src/pagination/cursor.rs:145
  - [ ] fn `suprnova::CursorPaginator::on_first_page` · framework/src/pagination/cursor.rs:153
  - [ ] fn `suprnova::CursorPaginator::on_last_page` · framework/src/pagination/cursor.rs:160
  - [ ] fn `suprnova::CursorPaginator::has_more_pages` · framework/src/pagination/cursor.rs:171
  - [ ] fn `suprnova::CursorPaginator::has_pages` · framework/src/pagination/cursor.rs:179
  - [ ] fn `suprnova::CursorPaginator::is_empty` · framework/src/pagination/cursor.rs:185
  - [ ] fn `suprnova::CursorPaginator::is_not_empty` · framework/src/pagination/cursor.rs:191
  - [ ] fn `suprnova::CursorPaginator::count` · framework/src/pagination/cursor.rs:197
  - [ ] fn `suprnova::CursorPaginator::encode_value` · framework/src/pagination/cursor.rs:236
  - [ ] fn `suprnova::CursorPaginator::decode_value` · framework/src/pagination/cursor.rs:288
  - [ ] fn `suprnova::CursorPaginator::encode_cursor` · framework/src/pagination/cursor.rs:323
  - [ ] fn `suprnova::CursorPaginator::try_encode_cursor` · framework/src/pagination/cursor.rs:335
  - [ ] fn `suprnova::CursorPaginator::decode_cursor` · framework/src/pagination/cursor.rs:352
- [ ] enum `suprnova::CursorDirection` · framework/src/pagination/cursor.rs:15 (also `suprnova::pagination::CursorDirection`, `suprnova::pagination::cursor::CursorDirection`)
  - Variants: `Next`, `Prev`

### `suprnova::pagination::inertia`

- [ ] trait `suprnova::IntoInertiaScroll` · framework/src/pagination/inertia.rs:13 (also `suprnova::pagination::IntoInertiaScroll`, `suprnova::pagination::inertia::IntoInertiaScroll`)
  - Implemented here by: `CursorPaginator`, `LengthAwarePaginator`, `Paginator`
  - [ ] fn `suprnova::IntoInertiaScroll::into_inertia_scroll` · framework/src/pagination/inertia.rs:16 (required)

### `suprnova::pagination::length_aware`

- [ ] struct `suprnova::LengthAwarePaginator` · framework/src/pagination/length_aware.rs:45 (also `suprnova::pagination::LengthAwarePaginator`, `suprnova::pagination::length_aware::LengthAwarePaginator`)
  - Public fields: `data`, `current_page`, `last_page`, `per_page`, `total`, `from`, `to`, `path`, `page_name`
  - Implements: `suprnova::IntoInertiaScroll`, `suprnova::Paginated`, `suprnova::ProvidesScrollMetadata`
  - [ ] fn `suprnova::LengthAwarePaginator::new` · framework/src/pagination/length_aware.rs:90
  - [ ] fn `suprnova::LengthAwarePaginator::with_page_name` · framework/src/pagination/length_aware.rs:151
  - [ ] fn `suprnova::LengthAwarePaginator::with_path` · framework/src/pagination/length_aware.rs:158
  - [ ] fn `suprnova::LengthAwarePaginator::with_base_url` · framework/src/pagination/length_aware.rs:168
  - [ ] fn `suprnova::LengthAwarePaginator::url_for_page` · framework/src/pagination/length_aware.rs:182
  - [ ] fn `suprnova::LengthAwarePaginator::has_more_pages` · framework/src/pagination/length_aware.rs:188
  - [ ] fn `suprnova::LengthAwarePaginator::on_first_page` · framework/src/pagination/length_aware.rs:195
  - [ ] fn `suprnova::LengthAwarePaginator::on_last_page` · framework/src/pagination/length_aware.rs:207
  - [ ] fn `suprnova::LengthAwarePaginator::has_pages` · framework/src/pagination/length_aware.rs:215
  - [ ] fn `suprnova::LengthAwarePaginator::is_empty` · framework/src/pagination/length_aware.rs:221
  - [ ] fn `suprnova::LengthAwarePaginator::is_not_empty` · framework/src/pagination/length_aware.rs:227
  - [ ] fn `suprnova::LengthAwarePaginator::count` · framework/src/pagination/length_aware.rs:235

### `suprnova::pagination::simple`

- [ ] struct `suprnova::Paginator` · framework/src/pagination/simple.rs:42 (also `suprnova::pagination::Paginator`, `suprnova::pagination::simple::Paginator`)
  - Public fields: `data`, `current_page`, `per_page`, `has_more`, `path`
  - Implements: `suprnova::IntoInertiaScroll`, `suprnova::ProvidesScrollMetadata`
  - [ ] fn `suprnova::Paginator::new` · framework/src/pagination/simple.rs:61
  - [ ] fn `suprnova::Paginator::with_path` · framework/src/pagination/simple.rs:73
  - [ ] fn `suprnova::Paginator::on_first_page` · framework/src/pagination/simple.rs:80
  - [ ] fn `suprnova::Paginator::on_last_page` · framework/src/pagination/simple.rs:86
  - [ ] fn `suprnova::Paginator::has_more_pages` · framework/src/pagination/simple.rs:93
  - [ ] fn `suprnova::Paginator::has_pages` · framework/src/pagination/simple.rs:100
  - [ ] fn `suprnova::Paginator::is_empty` · framework/src/pagination/simple.rs:106
  - [ ] fn `suprnova::Paginator::is_not_empty` · framework/src/pagination/simple.rs:112
  - [ ] fn `suprnova::Paginator::count` · framework/src/pagination/simple.rs:118

### `suprnova::pagination`

- [ ] struct `suprnova::Pagination` · framework/src/pagination/mod.rs:25 (also `suprnova::pagination::Pagination`)
  - [ ] fn `suprnova::Pagination::length_aware` · framework/src/pagination/mod.rs:44
  - [ ] fn `suprnova::Pagination::length_aware_on` · framework/src/pagination/mod.rs:65
  - [ ] fn `suprnova::Pagination::cursor` · framework/src/pagination/mod.rs:141
  - [ ] fn `suprnova::Pagination::cursor_on` · framework/src/pagination/mod.rs:165
- [ ] trait `suprnova::Paginated` · framework/src/pagination/mod.rs:278 (also `suprnova::pagination::Paginated`)
  - Implemented here by: `CursorPaginator`, `LengthAwarePaginator`
  - [ ] fn `suprnova::Paginated::items` · framework/src/pagination/mod.rs:280 (required)
  - [ ] fn `suprnova::Paginated::meta_value` · framework/src/pagination/mod.rs:284 (required)
  - [ ] fn `suprnova::Paginated::links_iter` · framework/src/pagination/mod.rs:288 (required)
