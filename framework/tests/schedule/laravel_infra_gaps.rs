//! The scheduling members of Laravel's surface that Suprnova lacked or
//! read differently (PAR-143): `at` and `daily_at` read a bare hour as
//! that hour on the hour, as Laravel's `dailyAt` does.

use suprnova::CronExpression;

#[test]
fn at_reads_a_bare_hour_as_that_hour_on_the_hour() {
    let expression = CronExpression::monthly_on(15).at("9");

    assert_eq!(
        expression.expression(),
        "0 9 15 * *",
        "Laravel's dailyAt reads \"9\" as 09:00, so monthlyOn(15, '9') runs at 09:00 on the 15th"
    );
}
