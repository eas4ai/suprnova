# Feature map: `manual/responses.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 127 checked.

## Rust API: suprnova

### `suprnova::http::body`

- [ ] fn `suprnova::http::collect_body` · framework/src/http/body.rs:147 (also `suprnova::http::body::collect_body`)
- [ ] fn `suprnova::collect_body_with_cap` · framework/src/http/body.rs:82 (also `suprnova::http::body::collect_body_with_cap`)
- [ ] fn `suprnova::global_max_request_body_bytes` · framework/src/http/body.rs:54 (also `suprnova::http::body::global_max_request_body_bytes`)
- [ ] fn `suprnova::http::parse_form` · framework/src/http/body.rs:164 (also `suprnova::http::body::parse_form`)
- [ ] fn `suprnova::http::parse_json` · framework/src/http/body.rs:155 (also `suprnova::http::body::parse_json`)
- [ ] fn `suprnova::set_global_max_request_body_bytes` · framework/src/http/body.rs:46 (also `suprnova::http::body::set_global_max_request_body_bytes`)
- [ ] const `suprnova::DEFAULT_MAX_REQUEST_BODY_BYTES` · framework/src/http/body.rs:30 (also `suprnova::http::body::DEFAULT_MAX_REQUEST_BODY_BYTES`)

### `suprnova::http::cookie`

- [ ] fn `suprnova::http::parse_cookies` · framework/src/http/cookie.rs:570 (also `suprnova::http::cookie::parse_cookies`)
- [ ] struct `suprnova::Cookie` · framework/src/http/cookie.rs:239 (also `suprnova::http::Cookie`, `suprnova::http::cookie::Cookie`)
  - [ ] fn `suprnova::Cookie::new` · framework/src/http/cookie.rs:253
  - [ ] fn `suprnova::Cookie::name` · framework/src/http/cookie.rs:262
  - [ ] fn `suprnova::Cookie::value` · framework/src/http/cookie.rs:267
  - [ ] fn `suprnova::Cookie::http_only` · framework/src/http/cookie.rs:274
  - [ ] fn `suprnova::Cookie::secure` · framework/src/http/cookie.rs:282
  - [ ] fn `suprnova::Cookie::same_site` · framework/src/http/cookie.rs:290
  - [ ] fn `suprnova::Cookie::max_age` · framework/src/http/cookie.rs:298
  - [ ] fn `suprnova::Cookie::path` · framework/src/http/cookie.rs:304
  - [ ] fn `suprnova::Cookie::domain` · framework/src/http/cookie.rs:310
  - [ ] fn `suprnova::Cookie::prefixed` · framework/src/http/cookie.rs:320
  - [ ] fn `suprnova::Cookie::partitioned` · framework/src/http/cookie.rs:329
  - [ ] fn `suprnova::Cookie::to_header_value` · framework/src/http/cookie.rs:335
  - [ ] fn `suprnova::Cookie::forget` · framework/src/http/cookie.rs:421
  - [ ] fn `suprnova::Cookie::forget_with` · framework/src/http/cookie.rs:438
  - [ ] fn `suprnova::Cookie::forever` · framework/src/http/cookie.rs:453
  - [ ] fn `suprnova::Cookie::encrypted` · framework/src/http/cookie.rs:473
  - [ ] fn `suprnova::Cookie::read_encrypted_for` · framework/src/http/cookie.rs:492
  - [ ] fn `suprnova::Cookie::read_encrypted` · framework/src/http/cookie.rs:504 (deprecated)
  - [ ] fn `suprnova::Cookie::queue` · framework/src/http/cookie.rs:534
  - [ ] fn `suprnova::Cookie::queued` · framework/src/http/cookie.rs:541
  - [ ] fn `suprnova::Cookie::unqueue` · framework/src/http/cookie.rs:547
  - [ ] fn `suprnova::Cookie::expire` · framework/src/http/cookie.rs:556
- [ ] struct `suprnova::CookieOptions` · framework/src/http/cookie.rs:192 (also `suprnova::http::CookieOptions`, `suprnova::http::cookie::CookieOptions`)
  - Public fields: `http_only`, `secure`, `same_site`, `path`, `domain`, `max_age`, `partitioned`
- [ ] enum `suprnova::CookiePrefix` · framework/src/http/cookie.rs:83 (also `suprnova::http::CookiePrefix`, `suprnova::http::cookie::CookiePrefix`)
  - Variants: `None`, `Secure`, `Host`
  - [ ] fn `suprnova::CookiePrefix::parse` · framework/src/http/cookie.rs:101
  - [ ] fn `suprnova::CookiePrefix::as_str` · framework/src/http/cookie.rs:111
  - [ ] fn `suprnova::CookiePrefix::apply` · framework/src/http/cookie.rs:132
  - [ ] fn `suprnova::CookiePrefix::strip` · framework/src/http/cookie.rs:151
  - [ ] fn `suprnova::CookiePrefix::validate` · framework/src/http/cookie.rs:164
- [ ] enum `suprnova::SameSite` · framework/src/http/cookie.rs:53 (also `suprnova::http::SameSite`, `suprnova::http::cookie::SameSite`)
  - Variants: `Strict`, `Lax`, `None`

### `suprnova::http::response` (private module; items are public through re-exports)

- [ ] struct `suprnova::HttpResponse` · framework/src/http/response.rs:41 (also `suprnova::http::HttpResponse`, `suprnova::prelude::HttpResponse`)
  - [ ] fn `suprnova::HttpResponse::new` · framework/src/http/response.rs:70
  - [ ] fn `suprnova::HttpResponse::text` · framework/src/http/response.rs:79
  - [ ] fn `suprnova::HttpResponse::json` · framework/src/http/response.rs:88
  - [ ] fn `suprnova::HttpResponse::bytes` · framework/src/http/response.rs:106
  - [ ] fn `suprnova::HttpResponse::from_engine_response` · framework/src/http/response.rs:134
  - [ ] fn `suprnova::HttpResponse::html` · framework/src/http/response.rs:164
  - [ ] fn `suprnova::HttpResponse::sse` · framework/src/http/response.rs:189
  - [ ] fn `suprnova::HttpResponse::event_stream` · framework/src/http/response.rs:211
  - [ ] fn `suprnova::HttpResponse::stream_bytes` · framework/src/http/response.rs:251
  - [ ] fn `suprnova::HttpResponse::stream_json` · framework/src/http/response.rs:273
  - [ ] fn `suprnova::HttpResponse::status` · framework/src/http/response.rs:310
  - [ ] fn `suprnova::HttpResponse::status_code` · framework/src/http/response.rs:316
  - [ ] fn `suprnova::HttpResponse::bytes_body` · framework/src/http/response.rs:324
  - [ ] fn `suprnova::HttpResponse::body` · framework/src/http/response.rs:334
  - [ ] fn `suprnova::HttpResponse::is_streaming` · framework/src/http/response.rs:349
  - [ ] fn `suprnova::HttpResponse::header` · framework/src/http/response.rs:354
  - [ ] fn `suprnova::HttpResponse::with_headers` · framework/src/http/response.rs:366
  - [ ] fn `suprnova::HttpResponse::without_header` · framework/src/http/response.rs:380
  - [ ] fn `suprnova::HttpResponse::header_value` · framework/src/http/response.rs:388
  - [ ] fn `suprnova::HttpResponse::header_values` · framework/src/http/response.rs:411
  - [ ] fn `suprnova::HttpResponse::headers` · framework/src/http/response.rs:433
  - [ ] fn `suprnova::HttpResponse::replace_header` · framework/src/http/response.rs:439
  - [ ] fn `suprnova::HttpResponse::cookie` · framework/src/http/response.rs:457
  - [ ] fn `suprnova::HttpResponse::with_cookies` · framework/src/http/response.rs:465
  - [ ] fn `suprnova::HttpResponse::without_cookie` · framework/src/http/response.rs:478
  - [ ] fn `suprnova::HttpResponse::without_cookies` · framework/src/http/response.rs:489
  - [ ] fn `suprnova::HttpResponse::ok` · framework/src/http/response.rs:501
  - [ ] fn `suprnova::HttpResponse::into_hyper` · framework/src/http/response.rs:528
- [ ] struct `suprnova::Redirect` · framework/src/http/response.rs:684 (also `suprnova::http::Redirect`, `suprnova::prelude::Redirect`)
  - [ ] fn `suprnova::Redirect::to` · framework/src/http/response.rs:719
  - [ ] fn `suprnova::Redirect::route` · framework/src/http/response.rs:734
  - [ ] fn `suprnova::Redirect::back` · framework/src/http/response.rs:763
  - [ ] fn `suprnova::Redirect::away` · framework/src/http/response.rs:777
  - [ ] fn `suprnova::Redirect::refresh` · framework/src/http/response.rs:791
  - [ ] fn `suprnova::Redirect::refresh_for` · framework/src/http/response.rs:804
  - [ ] fn `suprnova::Redirect::guest` · framework/src/http/response.rs:818
  - [ ] fn `suprnova::Redirect::intended` · framework/src/http/response.rs:833
  - [ ] fn `suprnova::Redirect::set_intended_url` · framework/src/http/response.rs:844
  - [ ] fn `suprnova::Redirect::signed_route` · framework/src/http/response.rs:861
  - [ ] fn `suprnova::Redirect::temporary_signed_route` · framework/src/http/response.rs:868
  - [ ] fn `suprnova::Redirect::query` · framework/src/http/response.rs:879
  - [ ] fn `suprnova::Redirect::permanent` · framework/src/http/response.rs:885
  - [ ] fn `suprnova::Redirect::status` · framework/src/http/response.rs:893
  - [ ] fn `suprnova::Redirect::with` · framework/src/http/response.rs:903
  - [ ] fn `suprnova::Redirect::with_input` · framework/src/http/response.rs:912
  - [ ] fn `suprnova::Redirect::with_errors` · framework/src/http/response.rs:942
  - [ ] fn `suprnova::Redirect::with_errors_bag` · framework/src/http/response.rs:953
  - [ ] fn `suprnova::Redirect::cookie` · framework/src/http/response.rs:980
  - [ ] fn `suprnova::Redirect::with_cookies` · framework/src/http/response.rs:987
  - [ ] fn `suprnova::Redirect::without_cookie` · framework/src/http/response.rs:1002
  - [ ] fn `suprnova::Redirect::without_cookies` · framework/src/http/response.rs:1009
  - [ ] fn `suprnova::Redirect::header` · framework/src/http/response.rs:1022
  - [ ] fn `suprnova::Redirect::with_headers` · framework/src/http/response.rs:1029
  - [ ] fn `suprnova::Redirect::with_fragment` · framework/src/http/response.rs:1044
  - [ ] fn `suprnova::Redirect::without_fragment` · framework/src/http/response.rs:1054
  - [ ] fn `suprnova::Redirect::preserve_fragment` · framework/src/http/response.rs:1071
- [ ] struct `suprnova::RedirectRouteBuilder` · framework/src/http/response.rs:1219 (also `suprnova::http::RedirectRouteBuilder`)
  - [ ] fn `suprnova::RedirectRouteBuilder::with` · framework/src/http/response.rs:1240
  - [ ] fn `suprnova::RedirectRouteBuilder::query` · framework/src/http/response.rs:1246
  - [ ] fn `suprnova::RedirectRouteBuilder::permanent` · framework/src/http/response.rs:1252
  - [ ] fn `suprnova::RedirectRouteBuilder::status` · framework/src/http/response.rs:1259
  - [ ] fn `suprnova::RedirectRouteBuilder::flash` · framework/src/http/response.rs:1268
  - [ ] fn `suprnova::RedirectRouteBuilder::with_input` · framework/src/http/response.rs:1275
  - [ ] fn `suprnova::RedirectRouteBuilder::with_errors` · framework/src/http/response.rs:1298
  - [ ] fn `suprnova::RedirectRouteBuilder::with_errors_bag` · framework/src/http/response.rs:1308
  - [ ] fn `suprnova::RedirectRouteBuilder::cookie` · framework/src/http/response.rs:1330
  - [ ] fn `suprnova::RedirectRouteBuilder::with_cookies` · framework/src/http/response.rs:1336
  - [ ] fn `suprnova::RedirectRouteBuilder::without_cookie` · framework/src/http/response.rs:1346
  - [ ] fn `suprnova::RedirectRouteBuilder::without_cookies` · framework/src/http/response.rs:1353
  - [ ] fn `suprnova::RedirectRouteBuilder::header` · framework/src/http/response.rs:1365
  - [ ] fn `suprnova::RedirectRouteBuilder::with_headers` · framework/src/http/response.rs:1371
  - [ ] fn `suprnova::RedirectRouteBuilder::with_fragment` · framework/src/http/response.rs:1384
  - [ ] fn `suprnova::RedirectRouteBuilder::without_fragment` · framework/src/http/response.rs:1393
  - [ ] fn `suprnova::RedirectRouteBuilder::preserve_fragment` · framework/src/http/response.rs:1401
- [ ] trait `suprnova::ResponseExt` · framework/src/http/response.rs:604 (also `suprnova::http::ResponseExt`)
  - Implemented here by: `Response`
  - [ ] fn `suprnova::ResponseExt::status` · framework/src/http/response.rs:606 (required)
  - [ ] fn `suprnova::ResponseExt::header` · framework/src/http/response.rs:608 (required)
  - [ ] fn `suprnova::ResponseExt::with_headers` · framework/src/http/response.rs:611 (required)
  - [ ] fn `suprnova::ResponseExt::without_header` · framework/src/http/response.rs:618 (required)
  - [ ] fn `suprnova::ResponseExt::cookie` · framework/src/http/response.rs:620 (required)
  - [ ] fn `suprnova::ResponseExt::with_cookies` · framework/src/http/response.rs:623 (required)
  - [ ] fn `suprnova::ResponseExt::without_cookie` · framework/src/http/response.rs:628 (required)
  - [ ] fn `suprnova::ResponseExt::without_cookies` · framework/src/http/response.rs:631 (required)
- [ ] type `suprnova::Response` · framework/src/http/response.rs:56 (also `suprnova::http::Response`, `suprnova::prelude::Response`)

### `suprnova`

- [ ] macro `suprnova::json_response` · framework/src/lib.rs:668
- [ ] macro `suprnova::text_response` · framework/src/lib.rs:679

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::redirect` · suprnova-macros/src/lib.rs:210 (re-exported as `suprnova::redirect`)
  - Form: function-like `redirect!(...)`
