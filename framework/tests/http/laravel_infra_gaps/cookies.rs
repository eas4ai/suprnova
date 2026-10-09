//! Raw cookies (PAR-164): `Cookie::raw` writes its value to `Set-Cookie`
//! without percent-encoding, as Laravel's `CookieJar::make` does with
//! `raw: true`, and refuses a name that is not an RFC 6265 token or a
//! value byte outside cookie-octet, so the raw form cannot inject a header
//! or an attribute. `Cookie::new` keeps encoding.

use suprnova::{Cookie, CookiePrefix, http::parse_cookies};

#[test]
fn raw_cookie_writes_its_value_unencoded() {
    let cookie = Cookie::raw("token", "a:b/c").expect("a valid raw cookie");
    assert!(cookie.is_raw());
    let header = cookie.to_header_value();
    assert!(header.starts_with("token=a:b/c;"), "{header}");
}

#[test]
fn raw_cookie_keeps_the_secure_defaults() {
    let header = Cookie::raw("token", "abc")
        .expect("valid")
        .to_header_value();
    for attribute in ["Path=/", "HttpOnly", "Secure", "SameSite=Lax"] {
        assert!(header.contains(attribute), "{attribute} missing: {header}");
    }
}

#[test]
fn raw_cookie_accepts_every_cookie_octet() {
    // cookie-octet: 0x21, 0x23-0x2B, 0x2D-0x3A, 0x3C-0x5B, 0x5D-0x7E.
    let octets: String = (0x21u8..=0x7E)
        .filter(|b| !matches!(b, b'"' | b',' | b';' | b'\\'))
        .map(char::from)
        .collect();
    let cookie = Cookie::raw("t", octets.clone()).expect("every cookie-octet is allowed");
    assert!(
        cookie
            .to_header_value()
            .starts_with(&format!("t={octets};")),
        "{}",
        cookie.to_header_value()
    );
    // An empty value is allowed: `cookie-value = *cookie-octet`.
    assert!(Cookie::raw("t", "").is_ok());
}

#[test]
fn raw_cookie_refuses_an_attribute_injection() {
    let error = Cookie::raw("t", "x;Domain=evil").expect_err("`;` is not a cookie-octet");
    assert!(error.to_string().contains("cookie"), "{error}");
}

#[test]
fn raw_cookie_refuses_every_byte_outside_cookie_octet() {
    for value in [
        "a\rb",
        "a\nb",
        "a b",
        "a\"b",
        "a,b",
        "a;b",
        "a\\b",
        "a\0b",
        "a\x7Fb",
        "caf\u{e9}",
    ] {
        assert!(
            Cookie::raw("t", value).is_err(),
            "{value:?} holds a byte outside cookie-octet"
        );
    }
}

#[test]
fn raw_cookie_refuses_a_name_that_is_not_a_token() {
    for name in ["", "a b", "a=b", "a;b", "a\r\nb", "a(b", "a/b", "a\u{e9}"] {
        assert!(Cookie::raw(name, "v").is_err(), "{name:?} is not a token");
    }
    // Token characters other than letters and digits stay allowed.
    assert!(Cookie::raw("a!#$%&'*+-.^_`|~9", "v").is_ok());
}

#[test]
fn new_cookie_keeps_encoding_its_value() {
    let cookie = Cookie::new("token", "a:b/c");
    assert!(!cookie.is_raw());
    let header = cookie.to_header_value();
    assert!(header.starts_with("token=a%3Ab%2Fc;"), "{header}");
}

#[test]
fn raw_cookie_keeps_the_prefix_rules() {
    let header = Cookie::raw("session", "abc")
        .expect("valid")
        .secure(false)
        .path("/admin")
        .domain("example.com")
        .prefixed(CookiePrefix::Host)
        .to_header_value();
    assert!(header.starts_with("__Host-session=abc;"), "{header}");
    assert!(header.contains("Path=/;"), "{header}");
    assert!(header.contains("Secure"), "{header}");
    assert!(!header.contains("Domain="), "{header}");
}

#[test]
fn a_raw_value_holding_percent_reads_back_decoded() {
    // The read path cannot tell a raw cookie from an encoded one and
    // decodes every value, as PHP decodes `$_COOKIE` for Laravel.
    let header = Cookie::raw("t", "100%25").expect("valid").to_header_value();
    let pair = header.split(';').next().expect("a name=value pair");
    assert_eq!(pair, "t=100%25");
    assert_eq!(
        parse_cookies(pair).get("t").map(String::as_str),
        Some("100%")
    );
}
