//! Laravel's `accepted`, `digits`, `prohibited`, `missing`, `exclude_if`
//! and `exclude_unless`, alone and through every row shape of
//! `validate!`.

use serde_json::{Value, json};
use suprnova::rules::{
    Accepted, Digits, ExcludeIf, ExcludeUnless, Min, Missing, Prohibited, Required, RequiredIf,
};
use suprnova::{ContextualRule, FormContext, Rule, ValidationErrors, ValueRule, validate};

fn ctx(pairs: &[(&str, &str)]) -> FormContext {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn keys(result: Result<(), ValidationErrors>) -> Vec<String> {
    let errors = match result {
        Ok(()) => return Vec::new(),
        Err(errors) => errors,
    };
    let mut keys: Vec<String> = errors.errors.keys().cloned().collect();
    keys.sort();
    keys
}

#[test]
fn accepted_takes_laravels_four_strings_and_nothing_else() {
    for value in ["yes", "on", "1", "true"] {
        assert!(Rule::passes(&Accepted, value).is_ok(), "{value}");
    }
    for value in ["", "no", "off", "0", "false", "YES", "True", " yes"] {
        let err = Rule::passes(&Accepted, value).unwrap_err();
        assert_eq!(err.key, "validation-accepted", "{value}");
    }
}

#[test]
fn accepted_takes_json_true_and_one() {
    for value in [json!(true), json!(1), json!("on")] {
        assert!(ValueRule::passes(&Accepted, &value).is_ok(), "{value}");
    }
    for value in [
        json!(false),
        json!(0),
        json!(1.0),
        json!(2),
        json!(null),
        json!([]),
    ] {
        assert!(ValueRule::passes(&Accepted, &value).is_err(), "{value}");
    }
}

#[test]
fn digits_counts_ascii_digits_exactly() {
    let pin = Digits(4);
    assert!(pin.passes("0042").is_ok(), "leading zeros are digits");
    for value in ["042", "00420", "4.20", "-042", "+042", "٤٢٤٢", "", "abcd"] {
        let err = pin.passes(value).unwrap_err();
        assert_eq!(err.key, "validation-digits", "{value}");
    }
}

#[test]
fn prohibited_passes_only_empty_values() {
    assert!(Rule::passes(&Prohibited, "").is_ok());
    assert!(Rule::passes(&Prohibited, "   ").is_ok());
    let err = Rule::passes(&Prohibited, "x").unwrap_err();
    assert_eq!(err.key, "validation-prohibited");

    for value in [json!(null), json!(""), json!(" "), json!([]), json!({})] {
        assert!(ValueRule::passes(&Prohibited, &value).is_ok(), "{value}");
    }
    for value in [
        json!(false),
        json!(0),
        json!("x"),
        json!([null]),
        json!({"a": 1}),
    ] {
        assert!(ValueRule::passes(&Prohibited, &value).is_err(), "{value}");
    }
}

#[test]
fn missing_fails_every_value_that_arrived() {
    let err = Rule::passes(&Missing, "").unwrap_err();
    assert_eq!(err.key, "validation-missing");
    assert!(ValueRule::passes(&Missing, &json!(null)).is_err());
}

#[test]
fn exclude_if_and_exclude_unless_answer_from_the_sibling() {
    let cash = ctx(&[("payment", "cash")]);
    let card = ctx(&[("payment", "card")]);
    let none = FormContext::new();

    let exclude_if = ExcludeIf {
        other: "payment",
        value: "cash",
    };
    assert!(exclude_if.excludes(&cash));
    assert!(!exclude_if.excludes(&card));
    assert!(!exclude_if.excludes(&none));

    let exclude_unless = ExcludeUnless {
        other: "payment",
        value: "card",
    };
    assert!(exclude_unless.excludes(&cash));
    assert!(!exclude_unless.excludes(&card));
    assert!(
        exclude_unless.excludes(&none),
        "an absent sibling is not `card`"
    );

    assert!(exclude_if.passes("anything", &cash).is_ok());
    assert!(
        !RequiredIf {
            other: "payment",
            value: "card"
        }
        .excludes(&card),
        "other contextual rules never exclude"
    );
}

/// Every row shape `validate!` has, with the rules that implement more
/// than one field shape. This is a compile test as much as a runtime one:
/// a `String` field next to a rule that also takes JSON is where the
/// dispatch would turn ambiguous.
struct Signup {
    terms: String,
    newsletter: bool,
    consent: Value,
    pin: String,
    honeypot: String,
    honeypot_json: Value,
    referral: Option<String>,
    promo: Option<String>,
    promo_json: Option<Value>,
    agreed: Option<String>,
}

impl Signup {
    fn valid() -> Self {
        Self {
            terms: "yes".into(),
            newsletter: true,
            consent: json!(true),
            pin: "0042".into(),
            honeypot: String::new(),
            honeypot_json: json!(null),
            referral: None,
            promo: None,
            promo_json: None,
            agreed: Some("on".into()),
        }
    }

    fn check(&self) -> Result<(), ValidationErrors> {
        validate! { self =>
            terms => Accepted;
            newsletter => Accepted;
            consent => Accepted;
            pin => Digits(4);
            honeypot => Prohibited;
            honeypot_json => Prohibited;
            referral ?: Missing;
            promo ?: Missing;
            promo_json ?: Missing;
            agreed ?=> Accepted;
        }
    }
}

#[test]
fn the_rules_work_on_every_field_shape_validate_takes() {
    assert_eq!(keys(Signup::valid().check()), Vec::<String>::new());

    let signup = Signup {
        terms: "no".into(),
        newsletter: false,
        consent: json!(0),
        pin: "42".into(),
        honeypot: "bot".into(),
        honeypot_json: json!(["bot"]),
        referral: Some(String::new()),
        promo: Some("SAVE".into()),
        promo_json: Some(json!(null)),
        agreed: None,
    };
    assert_eq!(
        keys(signup.check()),
        [
            "agreed",
            "consent",
            "honeypot",
            "honeypot_json",
            "newsletter",
            "pin",
            "promo",
            "promo_json",
            "referral",
            "terms"
        ]
    );
}

struct Checkout {
    payment: String,
    card_number: String,
    pin: String,
    voucher: Option<String>,
    notes: Option<String>,
}

impl Checkout {
    fn check(&self) -> Result<(), ValidationErrors> {
        let form = ctx(&[("payment", self.payment.as_str())]);
        validate! { self =>
            card_number => ExcludeIf { other: "payment", value: "cash" } => with form, Required, Digits(16);
            pin => Digits(4), ExcludeIf { other: "payment", value: "cash" } => with form, Required;
            voucher ?=> ExcludeUnless { other: "payment", value: "voucher" } => with form, Required;
            notes ?: ExcludeIf { other: "payment", value: "cash" } => with form, Min(10);
        }
    }

    fn paying(payment: &str) -> Self {
        Self {
            payment: payment.into(),
            card_number: String::new(),
            pin: "1234".into(),
            voucher: None,
            notes: None,
        }
    }
}

#[test]
fn an_exclusion_skips_the_rules_after_it() {
    let mut cash = Checkout::paying("cash");
    cash.notes = Some("short".into());
    assert_eq!(keys(cash.check()), Vec::<String>::new());

    let mut card = Checkout::paying("card");
    card.card_number = "1234".into();
    card.notes = Some("short".into());
    assert_eq!(keys(card.check()), ["card_number", "notes"]);
}

/// Laravel runs a field's rules in order and stops at an exclusion, so an
/// error from a rule before it stands.
#[test]
fn an_error_before_the_exclusion_stands() {
    let mut cash = Checkout::paying("cash");
    cash.pin = "12".into();
    assert_eq!(keys(cash.check()), ["pin"]);
}

#[test]
fn exclude_unless_skips_the_row_unless_the_sibling_matches() {
    let card = Checkout {
        card_number: "4242424242424242".into(),
        ..Checkout::paying("card")
    };
    assert_eq!(
        keys(card.check()),
        Vec::<String>::new(),
        "excluded: the voucher is not required"
    );

    let voucher = Checkout {
        card_number: "4242424242424242".into(),
        ..Checkout::paying("voucher")
    };
    assert_eq!(keys(voucher.check()), ["voucher"]);
}

/// A rule that owns its data, built from a local it moves. `validate!`
/// evaluates each rule expression once, so a row may move it.
struct OneOf(Vec<String>);

impl ContextualRule for OneOf {
    fn passes(&self, value: &str, _ctx: &FormContext) -> Result<(), suprnova::ValidationMessage> {
        if self.0.iter().any(|allowed| allowed == value) {
            Ok(())
        } else {
            Err("is not one of the allowed codes".into())
        }
    }
}

struct Coupon {
    code: String,
}

impl Coupon {
    fn check(&self, allowed: Vec<String>) -> Result<(), ValidationErrors> {
        let form = FormContext::new();
        validate! { self =>
            code => OneOf(allowed) => with form;
        }
    }
}

#[test]
fn a_row_may_move_a_value_into_its_rule() {
    let allowed = || vec!["SAVE10".to_string(), "WELCOME".to_string()];
    assert!(
        Coupon {
            code: "SAVE10".into()
        }
        .check(allowed())
        .is_ok()
    );
    assert_eq!(
        keys(
            Coupon {
                code: "FREE".into()
            }
            .check(allowed())
        ),
        ["code"]
    );
}

struct Consent {
    marketing: Option<bool>,
}

impl Consent {
    fn check(&self) -> Result<(), ValidationErrors> {
        validate! { self =>
            marketing => Accepted;
        }
    }
}

#[test]
fn an_absent_optional_toggle_is_not_accepted() {
    assert!(
        Consent {
            marketing: Some(true)
        }
        .check()
        .is_ok()
    );
    assert_eq!(
        keys(
            Consent {
                marketing: Some(false)
            }
            .check()
        ),
        ["marketing"]
    );
    assert_eq!(keys(Consent { marketing: None }.check()), ["marketing"]);
}
