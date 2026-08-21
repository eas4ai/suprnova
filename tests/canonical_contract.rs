//! Canonical JSON and identity boundary contract tests.

use suprnova_live::canonical::{
    CanonicalErrorKind, CanonicalValue, parse_canonical_value, to_canonical_bytes,
};
use suprnova_live::identity::{
    BrowserNonce, ComponentName, Revision, ScopeFingerprint, UnixMillis,
};
use suprnova_live::limits::InputLimits;

fn limits() -> InputLimits {
    InputLimits::new(256, 3, 8, 32).expect("test limits are valid")
}

#[test]
fn canonicalizes_object_order_numbers_and_negative_zero() {
    let value = parse_canonical_value(br#"{"z":-0,"a":4.50,"nested":{"b":1,"a":2}}"#, &limits())
        .expect("supported JSON should parse");

    let encoded = to_canonical_bytes(&value, &limits()).expect("supported value canonicalizes");

    assert_eq!(encoded, br#"{"a":4.5,"nested":{"a":2,"b":1},"z":0}"#);
}

#[test]
fn canonicalization_is_stable_across_a_round_trip() {
    let parsed = parse_canonical_value(br#" { "b" : [true, null, "ok"], "a": 1e2 } "#, &limits())
        .expect("supported JSON should parse");
    let once = to_canonical_bytes(&parsed, &limits()).expect("first canonicalization succeeds");
    let reparsed = parse_canonical_value(&once, &limits()).expect("canonical bytes parse");
    let twice = to_canonical_bytes(&reparsed, &limits()).expect("second canonicalization succeeds");

    assert_eq!(once, twice);
    assert_eq!(parsed, reparsed);
}

#[test]
fn follows_the_rfc_8785_utf16_property_order_vector() {
    let rfc_limits = InputLimits::new(1_024, 3, 16, 64).expect("test limits are valid");
    let value = parse_canonical_value(
        r#"{"€":"Euro Sign","\r":"Carriage Return","דּ":"Hebrew Letter Dalet With Dagesh","1":"One","😀":"Emoji: Grinning Face","":"Control","ö":"Latin Small Letter O With Diaeresis"}"#
            .as_bytes(),
        &rfc_limits,
    )
    .expect("the RFC 8785 sorting vector should parse");

    let encoded = to_canonical_bytes(&value, &rfc_limits).expect("the RFC vector canonicalizes");

    assert_eq!(
        encoded,
        "{\"\\r\":\"Carriage Return\",\"1\":\"One\",\"\u{80}\":\"Control\",\"ö\":\"Latin Small Letter O With Diaeresis\",\"€\":\"Euro Sign\",\"😀\":\"Emoji: Grinning Face\",\"דּ\":\"Hebrew Letter Dalet With Dagesh\"}"
            .as_bytes()
    );
}

#[test]
fn rejects_duplicate_object_keys() {
    let error = parse_canonical_value(br#"{"state":1,"state":2}"#, &limits())
        .expect_err("duplicate keys must not use last-write-wins semantics");

    assert_eq!(error.kind(), CanonicalErrorKind::DuplicateKey);
}

#[test]
fn rejects_input_before_parsing_when_byte_limit_is_exceeded() {
    let tiny = InputLimits::new(6, 3, 8, 32).expect("test limits are valid");
    let error =
        parse_canonical_value(br#"{"a":1}"#, &tiny).expect_err("seven bytes exceed the limit");

    assert_eq!(error.kind(), CanonicalErrorKind::TooLarge);
}

#[test]
fn rejects_nesting_collection_and_string_limits() {
    let depth_error = parse_canonical_value(br#"[[[[0]]]]"#, &limits())
        .expect_err("four containers exceed depth three");
    assert_eq!(depth_error.kind(), CanonicalErrorKind::TooDeep);

    let entry_limits = InputLimits::new(256, 3, 2, 32).expect("test limits are valid");
    parse_canonical_value(br#"[1,2]"#, &entry_limits)
        .expect("exactly the configured entry count is accepted");
    let entry_error = parse_canonical_value(br#"[1,2,3]"#, &entry_limits)
        .expect_err("three entries exceed the configured total");
    assert_eq!(entry_error.kind(), CanonicalErrorKind::TooManyEntries);

    let string_limits = InputLimits::new(256, 3, 8, 3).expect("test limits are valid");
    let string_error =
        parse_canonical_value(br#""four""#, &string_limits).expect_err("string bytes are bounded");
    assert_eq!(string_error.kind(), CanonicalErrorKind::StringTooLong);
}

#[test]
fn rejects_non_interoperable_numbers_malformed_utf8_and_trailing_data() {
    for input in [
        br#"9007199254740992"#.as_slice(),
        br#"-9007199254740992"#.as_slice(),
    ] {
        let error = parse_canonical_value(input, &limits())
            .expect_err("integers outside the JavaScript safe range are strings on this wire");
        assert_eq!(error.kind(), CanonicalErrorKind::InvalidNumber);
    }

    let utf8_error =
        parse_canonical_value(&[b'"', 0xff, b'"'], &limits()).expect_err("canonical JSON is UTF-8");
    assert_eq!(utf8_error.kind(), CanonicalErrorKind::InvalidUtf8);

    let trailing_error = parse_canonical_value(br#"{} {}"#, &limits())
        .expect_err("one input contains exactly one JSON value");
    assert_eq!(trailing_error.kind(), CanonicalErrorKind::InvalidJson);
}

#[test]
fn identifier_types_enforce_grammar_and_binary_strength() {
    assert_eq!(
        ComponentName::parse("catalog.search")
            .expect("component name is valid")
            .as_str(),
        "catalog.search"
    );
    assert!(ComponentName::parse("catalog search").is_err());
    assert!(ComponentName::parse("").is_err());

    let nonce = BrowserNonce::parse("AAECAwQFBgcICQoLDA0ODw")
        .expect("base64url nonce contains exactly 128 bits");
    assert_eq!(nonce.as_bytes().len(), 16);
    assert!(BrowserNonce::parse("dG9vLXNob3J0").is_err());
    assert!(BrowserNonce::parse("AAECAwQFBgcICQoLDA0ODw==").is_err());

    let fingerprint = ScopeFingerprint::parse("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8")
        .expect("scope fingerprint contains exactly 256 bits");
    assert_eq!(fingerprint.as_bytes().len(), 32);

    assert_eq!(Revision::parse("0").expect("zero is canonical").get(), 0);
    assert_eq!(
        Revision::parse("42")
            .expect("decimal revision is valid")
            .get(),
        42
    );
    assert!(Revision::parse("042").is_err());
    assert!(Revision::parse("-1").is_err());
    assert!(UnixMillis::parse("18446744073709551616").is_err());
}

#[test]
fn canonical_value_never_accepts_non_finite_programmatic_numbers() {
    assert!(CanonicalValue::number(f64::NAN).is_err());
    assert!(CanonicalValue::number(f64::INFINITY).is_err());
    assert!(CanonicalValue::number(f64::NEG_INFINITY).is_err());
}

#[test]
fn programmatic_values_are_bounded_before_serialization() {
    let deep = CanonicalValue::Array(vec![CanonicalValue::Array(vec![CanonicalValue::Array(
        vec![CanonicalValue::Array(vec![CanonicalValue::Null])],
    )])]);
    let depth_error = to_canonical_bytes(&deep, &limits())
        .expect_err("trusted construction still obeys signed-output depth limits");
    assert_eq!(depth_error.kind(), CanonicalErrorKind::TooDeep);

    let entry_limits = InputLimits::new(256, 3, 2, 32).expect("test limits are valid");
    let entries = CanonicalValue::Array(vec![
        CanonicalValue::Null,
        CanonicalValue::Null,
        CanonicalValue::Null,
    ]);
    let entry_error = to_canonical_bytes(&entries, &entry_limits)
        .expect_err("trusted construction still obeys signed-output entry limits");
    assert_eq!(entry_error.kind(), CanonicalErrorKind::TooManyEntries);

    let string_limits = InputLimits::new(256, 3, 8, 3).expect("test limits are valid");
    let string_error =
        to_canonical_bytes(&CanonicalValue::String("four".to_owned()), &string_limits)
            .expect_err("trusted construction still obeys signed-output string limits");
    assert_eq!(string_error.kind(), CanonicalErrorKind::StringTooLong);
}
