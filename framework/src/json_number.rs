//! Exact comparison of JSON numbers.
//!
//! `serde_json` keeps an integer as an `i64` or a `u64` and any other
//! number as an `f64`. Compared through `f64`, integers above 2^53 lose
//! their distinctions: neighbouring integers share one `f64`, so a
//! collection sorted by such an id leaves it unsorted, and a "distinct"
//! check calls two different ids the same. These comparisons are exact for
//! every pair of numbers `serde_json` holds.

use std::cmp::Ordering;

use serde_json::Number;

/// The order of two JSON numbers by value. Two integers compare as
/// integers, an integer and a float compare exactly, and two floats compare
/// as floats.
pub(crate) fn compare(a: &Number, b: &Number) -> Ordering {
    match (integer(a), integer(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(x), None) => b
            .as_f64()
            .map_or(Ordering::Equal, |y| compare_integer_with_float(x, y)),
        (None, Some(y)) => a.as_f64().map_or(Ordering::Equal, |x| {
            compare_integer_with_float(y, x).reverse()
        }),
        (None, None) => a
            .as_f64()
            .partial_cmp(&b.as_f64())
            .unwrap_or(Ordering::Equal),
    }
}

/// The number as an integer, when it is one. `i128` holds every `i64` and
/// every `u64`.
fn integer(number: &Number) -> Option<i128> {
    number
        .as_i64()
        .map(i128::from)
        .or_else(|| number.as_u64().map(i128::from))
}

/// `integer` against `float`, exactly: the float's whole part as an
/// integer first, then its fraction.
fn compare_integer_with_float(integer: i128, float: f64) -> Ordering {
    // 2^127. Every integer here lies well inside it, so a float at or
    // beyond it orders by its sign alone.
    const BOUND: f64 = 170_141_183_460_469_231_731_687_303_715_884_105_728.0;
    if float.is_nan() {
        return Ordering::Equal;
    }
    if float >= BOUND {
        return Ordering::Less;
    }
    if float <= -BOUND {
        return Ordering::Greater;
    }
    let whole = float.trunc();
    // `whole` is integral and inside the `i128` range, so the cast is
    // exact.
    match integer.cmp(&(whole as i128)) {
        Ordering::Equal => 0.0_f64
            .partial_cmp(&(float - whole))
            .unwrap_or(Ordering::Equal),
        unequal => unequal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn number(value: serde_json::Value) -> Number {
        match value {
            serde_json::Value::Number(number) => number,
            other => panic!("not a number: {other}"),
        }
    }

    fn cmp(a: serde_json::Value, b: serde_json::Value) -> Ordering {
        compare(&number(a), &number(b))
    }

    #[test]
    fn integers_above_two_to_the_53_compare_exactly() {
        assert_eq!(
            cmp(
                json!(9_007_199_254_740_993_i64),
                json!(9_007_199_254_740_992_i64)
            ),
            Ordering::Greater
        );
        assert_eq!(cmp(json!(u64::MAX), json!(u64::MAX - 1)), Ordering::Greater);
        assert_eq!(cmp(json!(-1), json!(u64::MAX)), Ordering::Less);
    }

    #[test]
    fn an_integer_and_a_float_compare_by_exact_value() {
        assert_eq!(cmp(json!(2), json!(2.0)), Ordering::Equal);
        assert_eq!(cmp(json!(2), json!(2.5)), Ordering::Less);
        assert_eq!(cmp(json!(-2), json!(-2.5)), Ordering::Greater);
        assert_eq!(
            cmp(
                json!(9_007_199_254_740_993_i64),
                json!(9_007_199_254_740_992.0)
            ),
            Ordering::Greater
        );
        assert_eq!(cmp(json!(1.0e300), json!(u64::MAX)), Ordering::Greater);
        assert_eq!(cmp(json!(-1.0e300), json!(i64::MIN)), Ordering::Less);
    }

    #[test]
    fn floats_compare_as_floats() {
        assert_eq!(cmp(json!(0.5), json!(0.25)), Ordering::Greater);
        assert_eq!(cmp(json!(0.5), json!(0.5)), Ordering::Equal);
    }
}
