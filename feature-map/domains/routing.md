# Feature map: `manual/routing.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 178 checked.

## Rust API: suprnova

### `suprnova::database::route_binding`

- [ ] struct `suprnova::RouteParam` · framework/src/database/route_binding.rs:115 (also `suprnova::database::RouteParam`, `suprnova::database::route_binding::RouteParam`)
  - Public tuple fields: 1
  - Implements: `suprnova::AutoRouteBinding`
  - [ ] fn `suprnova::RouteParam::into_inner` · framework/src/database/route_binding.rs:119
- [ ] trait `suprnova::AutoRouteBinding` · framework/src/database/route_binding.rs:206 (also `suprnova::database::AutoRouteBinding`, `suprnova::database::route_binding::AutoRouteBinding`)
  - Implemented here by: `RouteParam`
  - [ ] fn `suprnova::AutoRouteBinding::from_route_param` · framework/src/database/route_binding.rs:217 (required)
- [ ] trait `suprnova::RouteBinding` · framework/src/database/route_binding.rs:168 (also `suprnova::database::RouteBinding`, `suprnova::database::route_binding::RouteBinding`)
  - [ ] fn `suprnova::RouteBinding::param_name` · framework/src/database/route_binding.rs:173 (required)
  - [ ] fn `suprnova::RouteBinding::from_route_param` · framework/src/database/route_binding.rs:185 (required)

### `suprnova::routing::group` (private module; items are public through re-exports)

- [ ] struct `suprnova::GroupBuilder` · framework/src/routing/group.rs:37 (also `suprnova::routing::GroupBuilder`)
  - [ ] fn `suprnova::GroupBuilder::middleware` · framework/src/routing/group.rs:89
  - [ ] fn `suprnova::GroupBuilder::block_session` · framework/src/routing/group.rs:96
  - [ ] fn `suprnova::GroupBuilder::try_finalize` · framework/src/routing/group.rs:136
- [ ] struct `suprnova::GroupRouter` · framework/src/routing/group.rs:205 (also `suprnova::routing::GroupRouter`)
  - [ ] fn `suprnova::GroupRouter::get` · framework/src/routing/group.rs:215
  - [ ] fn `suprnova::GroupRouter::post` · framework/src/routing/group.rs:230
  - [ ] fn `suprnova::GroupRouter::put` · framework/src/routing/group.rs:245
  - [ ] fn `suprnova::GroupRouter::delete` · framework/src/routing/group.rs:260
  - [ ] fn `suprnova::GroupRouter::patch` · framework/src/routing/group.rs:275
  - [ ] fn `suprnova::GroupRouter::head` · framework/src/routing/group.rs:295
  - [ ] fn `suprnova::GroupRouter::options` · framework/src/routing/group.rs:314
  - [ ] fn `suprnova::GroupRouter::any` · framework/src/routing/group.rs:336
  - [ ] fn `suprnova::GroupRouter::methods` · framework/src/routing/group.rs:369
  - [ ] fn `suprnova::GroupRouter::try_methods` · framework/src/routing/group.rs:382

### `suprnova::routing::macros` (private module; items are public through re-exports)

- [ ] fn `suprnova::validate_route_path` · framework/src/routing/macros.rs:45 (also `suprnova::routing::validate_route_path`)
- [ ] struct `suprnova::routing::AnyRouteDefBuilder` · framework/src/routing/macros.rs:586
  - Implements: `suprnova::IntoGroupItem`
  - [ ] fn `suprnova::routing::AnyRouteDefBuilder::name` · framework/src/routing/macros.rs:611
  - [ ] fn `suprnova::routing::AnyRouteDefBuilder::middleware` · framework/src/routing/macros.rs:618
  - [ ] fn `suprnova::routing::AnyRouteDefBuilder::register` · framework/src/routing/macros.rs:626
  - [ ] fn `suprnova::routing::AnyRouteDefBuilder::into_group_any_route` · framework/src/routing/macros.rs:1293
- [ ] struct `suprnova::FallbackDefBuilder` · framework/src/routing/macros.rs:814 (also `suprnova::routing::FallbackDefBuilder`)
  - [ ] fn `suprnova::FallbackDefBuilder::new` · framework/src/routing/macros.rs:825
  - [ ] fn `suprnova::FallbackDefBuilder::middleware` · framework/src/routing/macros.rs:833
  - [ ] fn `suprnova::FallbackDefBuilder::register` · framework/src/routing/macros.rs:839
- [ ] struct `suprnova::routing::GroupAnyRoute` · framework/src/routing/macros.rs:930
- [ ] struct `suprnova::GroupDef` · framework/src/routing/macros.rs:983 (also `suprnova::routing::GroupDef`)
  - Implements: `suprnova::IntoGroupItem`
  - [ ] fn `suprnova::GroupDef::add` · framework/src/routing/macros.rs:1012
  - [ ] fn `suprnova::GroupDef::route` · framework/src/routing/macros.rs:1020
  - [ ] fn `suprnova::GroupDef::middleware` · framework/src/routing/macros.rs:1051
  - [ ] fn `suprnova::GroupDef::block_session` · framework/src/routing/macros.rs:1060
  - [ ] fn `suprnova::GroupDef::register` · framework/src/routing/macros.rs:1083
- [ ] struct `suprnova::GroupRoute` · framework/src/routing/macros.rs:916 (also `suprnova::routing::GroupRoute`)
- [ ] struct `suprnova::RouteDefBuilder` · framework/src/routing/macros.rs:198 (also `suprnova::routing::RouteDefBuilder`)
  - Implements: `suprnova::IntoGroupItem`
  - [ ] fn `suprnova::RouteDefBuilder::new` · framework/src/routing/macros.rs:213
  - [ ] fn `suprnova::RouteDefBuilder::name` · framework/src/routing/macros.rs:225
  - [ ] fn `suprnova::RouteDefBuilder::middleware` · framework/src/routing/macros.rs:231
  - [ ] fn `suprnova::RouteDefBuilder::block_session` · framework/src/routing/macros.rs:239
  - [ ] fn `suprnova::RouteDefBuilder::register` · framework/src/routing/macros.rs:245
  - [ ] fn `suprnova::RouteDefBuilder::into_group_route` · framework/src/routing/macros.rs:1251
- [ ] struct `suprnova::WsRouteDef` · framework/src/routing/macros.rs:725 (also `suprnova::routing::WsRouteDef`)
  - [ ] fn `suprnova::WsRouteDef::new` · framework/src/routing/macros.rs:736
  - [ ] fn `suprnova::WsRouteDef::middleware` · framework/src/routing/macros.rs:755
  - [ ] fn `suprnova::WsRouteDef::config` · framework/src/routing/macros.rs:789
  - [ ] fn `suprnova::WsRouteDef::register` · framework/src/routing/macros.rs:796
- [ ] enum `suprnova::GroupItem` · framework/src/routing/macros.rs:940 (also `suprnova::routing::GroupItem`)
  - Variants: `Route`, `AnyRoute`, `NestedGroup`
- [ ] enum `suprnova::routing::HttpMethod` · framework/src/routing/macros.rs:161
  - Variants: `Get`, `Post`, `Put`, `Patch`, `Delete`, `Head`, `Options`
- [ ] trait `suprnova::IntoGroupItem` · framework/src/routing/macros.rs:950 (also `suprnova::routing::IntoGroupItem`)
  - Implemented here by: `GroupDef`, `RouteDefBuilder`, `routing::AnyRouteDefBuilder`
  - [ ] fn `suprnova::IntoGroupItem::into_group_item` · framework/src/routing/macros.rs:952 (required)

### `suprnova::routing::router` (private module; items are public through re-exports)

- [ ] fn `suprnova::clear_route_names_for_test` · framework/src/routing/router.rs:80 (also `suprnova::routing::clear_route_names_for_test`)
- [ ] fn `suprnova::routing::register_route_name` · framework/src/routing/router.rs:148
- [ ] fn `suprnova::route` · framework/src/routing/router.rs:412 (also `suprnova::routing::route`)
- [ ] fn `suprnova::routing::route_name_for_pattern` · framework/src/routing/router.rs:537
- [ ] fn `suprnova::routing::route_with_params` · framework/src/routing/router.rs:426
- [ ] fn `suprnova::routing::try_register_route_name` · framework/src/routing/router.rs:159
- [ ] fn `suprnova::routing::try_route` · framework/src/routing/router.rs:487
- [ ] fn `suprnova::routing::try_route_with_params` · framework/src/routing/router.rs:506
- [ ] struct `suprnova::routing::MultiMethodRouteBuilder` · framework/src/routing/router.rs:2296
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::methods` · framework/src/routing/router.rs:2306
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::path` · framework/src/routing/router.rs:2312
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::name` · framework/src/routing/router.rs:2324
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::try_name` · framework/src/routing/router.rs:2329
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::middleware` · framework/src/routing/router.rs:2338
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::middleware_boxed` · framework/src/routing/router.rs:2350
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::block_session` · framework/src/routing/router.rs:2361
- [ ] struct `suprnova::RouteBuilder` · framework/src/routing/router.rs:1975 (also `suprnova::routing::RouteBuilder`)
  - [ ] fn `suprnova::RouteBuilder::group` · framework/src/routing/group.rs:498
  - [ ] fn `suprnova::RouteBuilder::name` · framework/src/routing/router.rs:2011
  - [ ] fn `suprnova::RouteBuilder::try_name` · framework/src/routing/router.rs:2019
  - [ ] fn `suprnova::RouteBuilder::middleware` · framework/src/routing/router.rs:2046
  - [ ] fn `suprnova::RouteBuilder::middleware_boxed` · framework/src/routing/router.rs:2056
  - [ ] fn `suprnova::RouteBuilder::block_session` · framework/src/routing/router.rs:2069
  - [ ] fn `suprnova::RouteBuilder::get` · framework/src/routing/router.rs:2075
  - [ ] fn `suprnova::RouteBuilder::post` · framework/src/routing/router.rs:2084
  - [ ] fn `suprnova::RouteBuilder::put` · framework/src/routing/router.rs:2093
  - [ ] fn `suprnova::RouteBuilder::delete` · framework/src/routing/router.rs:2102
  - [ ] fn `suprnova::RouteBuilder::try_get` · framework/src/routing/router.rs:2111
  - [ ] fn `suprnova::RouteBuilder::try_post` · framework/src/routing/router.rs:2120
  - [ ] fn `suprnova::RouteBuilder::try_put` · framework/src/routing/router.rs:2129
  - [ ] fn `suprnova::RouteBuilder::try_delete` · framework/src/routing/router.rs:2139
  - [ ] fn `suprnova::RouteBuilder::patch` · framework/src/routing/router.rs:2148
  - [ ] fn `suprnova::RouteBuilder::try_patch` · framework/src/routing/router.rs:2157
  - [ ] fn `suprnova::RouteBuilder::head` · framework/src/routing/router.rs:2166
  - [ ] fn `suprnova::RouteBuilder::try_head` · framework/src/routing/router.rs:2175
  - [ ] fn `suprnova::RouteBuilder::options` · framework/src/routing/router.rs:2184
  - [ ] fn `suprnova::RouteBuilder::try_options` · framework/src/routing/router.rs:2194
  - [ ] fn `suprnova::RouteBuilder::any` · framework/src/routing/router.rs:2205
  - [ ] fn `suprnova::RouteBuilder::try_any` · framework/src/routing/router.rs:2214
  - [ ] fn `suprnova::RouteBuilder::methods` · framework/src/routing/router.rs:2228
  - [ ] fn `suprnova::RouteBuilder::try_methods` · framework/src/routing/router.rs:2243
- [ ] struct `suprnova::Router` · framework/src/routing/router.rs:564 (also `suprnova::routing::Router`)
  - [ ] fn `suprnova::Router::try_live_nested_segment` · framework/src/live/document.rs:575
  - [ ] fn `suprnova::Router::try_live_document` · framework/src/live/document.rs:606
  - [ ] fn `suprnova::Router::try_live_mount` · framework/src/live/document.rs:621
  - [ ] fn `suprnova::Router::try_live` · framework/src/live/routes.rs:83
  - [ ] fn `suprnova::Router::try_live_ui_assets` · framework/src/live/routes.rs:98
  - [ ] fn `suprnova::Router::try_live_ui_assets_from` · framework/src/live/routes.rs:107
  - [ ] fn `suprnova::Router::try_live_with` · framework/src/live/routes.rs:144
  - [ ] fn `suprnova::Router::try_live_upload_reacquisition` · framework/src/live/upload.rs:38
  - [ ] fn `suprnova::Router::group` · framework/src/routing/group.rs:472
  - [ ] fn `suprnova::Router::resource` · framework/src/routing/resource.rs:693
  - [ ] fn `suprnova::Router::api_resource` · framework/src/routing/resource.rs:705
  - [ ] fn `suprnova::Router::resources` · framework/src/routing/resource.rs:719
  - [ ] fn `suprnova::Router::api_resources` · framework/src/routing/resource.rs:741
  - [ ] fn `suprnova::Router::new` · framework/src/routing/router.rs:631
  - [ ] fn `suprnova::Router::get_route_middleware` · framework/src/routing/router.rs:661
  - [ ] fn `suprnova::Router::try_render_cache` · framework/src/routing/router.rs:804
  - [ ] fn `suprnova::Router::try_render_cache_group` · framework/src/routing/router.rs:822
  - [ ] fn `suprnova::Router::get_fallback` · framework/src/routing/router.rs:863
  - [ ] fn `suprnova::Router::get` · framework/src/routing/router.rs:1081
  - [ ] fn `suprnova::Router::try_get` · framework/src/routing/router.rs:1096
  - [ ] fn `suprnova::Router::post` · framework/src/routing/router.rs:1120
  - [ ] fn `suprnova::Router::try_post` · framework/src/routing/router.rs:1130
  - [ ] fn `suprnova::Router::put` · framework/src/routing/router.rs:1158
  - [ ] fn `suprnova::Router::try_put` · framework/src/routing/router.rs:1168
  - [ ] fn `suprnova::Router::delete` · framework/src/routing/router.rs:1192
  - [ ] fn `suprnova::Router::try_delete` · framework/src/routing/router.rs:1202
  - [ ] fn `suprnova::Router::patch` · framework/src/routing/router.rs:1230
  - [ ] fn `suprnova::Router::try_patch` · framework/src/routing/router.rs:1240
  - [ ] fn `suprnova::Router::head` · framework/src/routing/router.rs:1284
  - [ ] fn `suprnova::Router::try_head` · framework/src/routing/router.rs:1294
  - [ ] fn `suprnova::Router::options` · framework/src/routing/router.rs:1328
  - [ ] fn `suprnova::Router::try_options` · framework/src/routing/router.rs:1338
  - [ ] fn `suprnova::Router::methods` · framework/src/routing/router.rs:1381
  - [ ] fn `suprnova::Router::try_methods` · framework/src/routing/router.rs:1399
  - [ ] fn `suprnova::Router::any` · framework/src/routing/router.rs:1456
  - [ ] fn `suprnova::Router::try_any` · framework/src/routing/router.rs:1467
  - [ ] fn `suprnova::Router::ws` · framework/src/routing/router.rs:1490
  - [ ] fn `suprnova::Router::try_ws` · framework/src/routing/router.rs:1501
  - [ ] fn `suprnova::Router::ws_with_config` · framework/src/routing/router.rs:1536
  - [ ] fn `suprnova::Router::try_ws_with_config` · framework/src/routing/router.rs:1546
  - [ ] fn `suprnova::Router::ws_with_middleware` · framework/src/routing/router.rs:1566
  - [ ] fn `suprnova::Router::try_ws_with_middleware` · framework/src/routing/router.rs:1581
  - [ ] fn `suprnova::Router::ws_with_middleware_and_config` · framework/src/routing/router.rs:1596
  - [ ] fn `suprnova::Router::try_ws_with_middleware_and_config` · framework/src/routing/router.rs:1612
  - [ ] fn `suprnova::Router::match_ws` · framework/src/routing/router.rs:1741
  - [ ] fn `suprnova::Router::match_route` · framework/src/routing/router.rs:1766
  - [ ] fn `suprnova::Router::has_explicit_head` · framework/src/routing/router.rs:1810
  - [ ] fn `suprnova::Router::redirect` · framework/src/routing/router.rs:1831
  - [ ] fn `suprnova::Router::permanent_redirect` · framework/src/routing/router.rs:1853
  - [ ] fn `suprnova::Router::inertia` · framework/src/routing/router.rs:1890
  - [ ] fn `suprnova::Router::try_inertia` · framework/src/routing/router.rs:1908
  - [ ] fn `suprnova::Router::view` · framework/src/routing/router.rs:1963
- [ ] struct `suprnova::routing::WsMatch` · framework/src/routing/router.rs:2382
  - [ ] fn `suprnova::routing::WsMatch::handler` · framework/src/routing/router.rs:2393
  - [ ] fn `suprnova::routing::WsMatch::pattern` · framework/src/routing/router.rs:2399
  - [ ] fn `suprnova::routing::WsMatch::params` · framework/src/routing/router.rs:2408
  - [ ] fn `suprnova::routing::WsMatch::middleware` · framework/src/routing/router.rs:2415
  - [ ] fn `suprnova::routing::WsMatch::config` · framework/src/routing/router.rs:2426
- [ ] enum `suprnova::routing::RouteUrlError` · framework/src/routing/router.rs:445
  - Variants: `NameNotFound`, `MissingParams`
- [ ] type `suprnova::routing::BoxedHandler` · framework/src/routing/router.rs:552

### `suprnova::routing`

- [ ] fn `suprnova::redirect` · framework/src/routing/mod.rs:71 (also `suprnova::routing::redirect`)
- [ ] fn `suprnova::redirect_to` · framework/src/routing/mod.rs:79 (also `suprnova::routing::redirect_to`)

### `suprnova::static_files`

- [ ] struct `suprnova::StaticFiles` · framework/src/static_files.rs:33 (also `suprnova::static_files::StaticFiles`)
  - [ ] fn `suprnova::StaticFiles::public` · framework/src/static_files.rs:40
  - [ ] fn `suprnova::StaticFiles::from_dir` · framework/src/static_files.rs:45
  - [ ] fn `suprnova::StaticFiles::cache_control` · framework/src/static_files.rs:53
  - [ ] fn `suprnova::StaticFiles::handler` · framework/src/static_files.rs:59

### `suprnova`

- [ ] macro `suprnova::any` · framework/src/routing/macros.rs:560
- [ ] macro `suprnova::delete` · framework/src/routing/macros.rs:394
- [ ] macro `suprnova::fallback` · framework/src/routing/macros.rs:897
- [ ] macro `suprnova::get` · framework/src/routing/macros.rs:295
- [ ] macro `suprnova::group` · framework/src/routing/macros.rs:1365
- [ ] macro `suprnova::head` · framework/src/routing/macros.rs:470
- [ ] macro `suprnova::options` · framework/src/routing/macros.rs:509
- [ ] macro `suprnova::patch` · framework/src/routing/macros.rs:431
- [ ] macro `suprnova::post` · framework/src/routing/macros.rs:328
- [ ] macro `suprnova::put` · framework/src/routing/macros.rs:361
- [ ] macro `suprnova::route_binding` · framework/src/database/route_binding.rs:292
- [ ] macro `suprnova::routes` · framework/src/routing/macros.rs:1420
- [ ] macro `suprnova::ws` · framework/src/routing/macros.rs:672
