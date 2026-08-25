//! Rendering a cron expression in a timezone other than the one it was
//! written for.
//!
//! `schedule:list` shows operators when their tasks run. Once a task can
//! pin its own zone ([`super::TaskBuilder::timezone`]), the raw expression
//! stops being the whole truth: `0 3 * * *` in `Asia/Tokyo` is not 03:00 to
//! an operator reading the listing in UTC. Printing the raw field text and
//! a zone label next to it forces every reader to do the arithmetic in
//! their head, and the arithmetic is not simple - a whole-hour offset can
//! roll the day, a 45-minute offset (Asia/Kathmandu) rolls the minute
//! field too, and a day roll interacts with month lengths and the Vixie
//! day-of-month/day-of-week OR rule.
//!
//! This is a port of Laravel's `CronExpressionTimezoneConverter`
//! (framework PR #60877). It rewrites the five fields into the display
//! zone by expanding each field to explicit values, shifting them, and
//! collapsing the result back into the most compact cron syntax. When a
//! faithful rewrite is impossible it **refuses** and hands back the
//! original expression rather than printing something subtly wrong. The
//! refusals are deliberate and each has a reason:
//!
//! - the offset differs between the next run and the one after it (a DST
//!   transition sits between them, so no single expression is correct);
//! - the offset is zero (nothing to do);
//! - the expression does not have five fields;
//! - a day roll would have to move both day-of-week and day-of-month, which
//!   cron ORs rather than ANDs, so they cannot shift together;
//! - a day roll touches February, whose length varies by year;
//! - a field cannot be expanded at all (invalid part, `/0` step,
//!   out-of-range bound, inverted range).
//!
//! Everything here is a pure function of its arguments so the table tests
//! at the bottom can pin the algorithm against hand-derived expectations.

use super::expression::CronExpression;
use chrono::{DateTime, Offset, Utc};
use chrono_tz::Tz;

/// Number of days in each month of a **non-leap** year.
///
/// Laravel's port hard-codes 2023 for the same reason: a day roll has to
/// pick some month length, and cron expressions carry no year. Choosing the
/// non-leap lengths makes February 29 unrepresentable, which is exactly why
/// the February cases below refuse instead of guessing.
const DAYS_IN_MONTH: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

fn days_in_month(month: i32) -> i32 {
    DAYS_IN_MONTH[(month - 1) as usize]
}

/// What [`expressions_for_display`] decided about one expression.
///
/// The two cases are distinguished structurally rather than by comparing
/// the output text against the input: a genuine rewrite can legitimately
/// reproduce the text it started from (minute field `0,30` under a
/// 30-minute offset shifts to `30,0`, which collapses back to `0,30`), and
/// a caller that inferred "refused" from string equality would then label
/// that line with the wrong zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DisplayExpressions {
    /// The fields were rewritten into the display zone. One entry per
    /// output line: a schedule that straddles midnight in the display zone
    /// needs one cron line per side (Laravel's `flatMap`).
    Rewritten(Vec<String>),
    /// The expression is exactly as the user wrote it, because a faithful
    /// rewrite was refused - or was unnecessary, the two zones sharing an
    /// offset at the sampled instants. Either way the fields are still in
    /// the *event's* zone, which is what a caller must label them with.
    AsWritten(String),
}

/// The expressions to print for `expr` when the listing is being read in
/// `display_tz` and the task itself runs in `event_tz`.
///
/// Returns [`DisplayExpressions::AsWritten`] whenever a faithful conversion
/// is not possible - see the module docs for the full list of refusals.
///
/// `next` and `next2` are the next two instants the task fires, used only
/// to sample the two zones' offsets. `next2` being `None` skips the
/// DST-straddle check, matching Laravel's `if ($nextAt && ...)`. `next`
/// being `None` means the expression never fires at all, and a UTC offset
/// only exists at an instant, so there is nothing to sample and nothing to
/// convert - the expression is returned as written. (Laravel falls back to
/// "now" there; it has one in scope and this pure function does not. The
/// only expressions affected are unsatisfiable ones like `0 0 30 2 *`, for
/// which printing the text the user wrote is the more useful answer
/// anyway.)
pub(crate) fn expressions_for_display(
    expr: &CronExpression,
    event_tz: Tz,
    display_tz: Tz,
    next: Option<DateTime<Utc>>,
    next2: Option<DateTime<Utc>>,
) -> DisplayExpressions {
    match rewritten(expr, event_tz, display_tz, next, next2) {
        Some(expressions) => DisplayExpressions::Rewritten(expressions),
        None => DisplayExpressions::AsWritten(expr.expression().to_string()),
    }
}

/// The convertible path of [`expressions_for_display`]; `None` is "refuse,
/// print the expression as written".
fn rewritten(
    expr: &CronExpression,
    event_tz: Tz,
    display_tz: Tz,
    next: Option<DateTime<Utc>>,
    next2: Option<DateTime<Utc>>,
) -> Option<Vec<String>> {
    let at = next?;
    let (total_offset_minutes, hour_offset, minute_offset) =
        offset_components(event_tz, display_tz, at, next2)?;
    if total_offset_minutes == 0 {
        return None;
    }
    convert_expression(expr.expression(), hour_offset, minute_offset)
}

/// The zone's UTC offset in seconds at `at`.
fn offset_seconds(tz: Tz, at: DateTime<Utc>) -> i32 {
    at.with_timezone(&tz).offset().fix().local_minus_utc()
}

/// `(total minutes, whole hours, leftover minutes)` between the two zones,
/// or `None` when a DST transition sits between the next run and the one
/// after it.
///
/// The whole-hour / leftover split is a truncating division, so a negative
/// offset splits into a negative hour part and a negative minute part that
/// still sum back to the total - the arithmetic downstream relies on that.
/// Zones like Asia/Kathmandu (+05:45) are the reason the minute field has
/// to shift at all.
fn offset_components(
    event_tz: Tz,
    display_tz: Tz,
    at: DateTime<Utc>,
    next_at: Option<DateTime<Utc>>,
) -> Option<(i32, i32, i32)> {
    let total = (offset_seconds(display_tz, at) - offset_seconds(event_tz, at)) / 60;
    if let Some(next_at) = next_at {
        let total_next =
            (offset_seconds(display_tz, next_at) - offset_seconds(event_tz, next_at)) / 60;
        if total != total_next {
            return None;
        }
    }
    Some((total, total / 60, total % 60))
}

/// Split `raw` into its five fields and rewrite them by the given offsets.
///
/// `None` when the expression is not five fields (defensive: every
/// [`CronExpression`] that parsed has five, but the converter is written
/// against the text, exactly like the upstream port) or when any step of
/// the rewrite refuses.
fn convert_expression(raw: &str, hour_offset: i32, minute_offset: i32) -> Option<Vec<String>> {
    let segments: Vec<&str> = raw.split_whitespace().collect();
    let segments: [&str; 5] = segments.try_into().ok()?;
    convert(&segments, hour_offset, minute_offset)
}

/// Rewrite five already-split fields.
///
/// Minutes are shifted first because a minute carry feeds the hour offset;
/// hours are shifted next because an hour carry feeds the day fields. Each
/// carry direction becomes its own output expression unless it can be
/// merged away.
fn convert(segments: &[&str; 5], hour_offset: i32, minute_offset: i32) -> Option<Vec<String>> {
    // When every day field is `*` the schedule repeats identically on all
    // days, so a day rollover changes nothing and the carry groups can be
    // folded back into one expression instead of two.
    let days_are_wildcard = segments[2] == "*" && segments[3] == "*" && segments[4] == "*";
    // Minutes may only merge when the hour field additionally covers every
    // hour: otherwise an hour that rolled is a different hour, and folding
    // the minute groups would claim minutes the task never runs at.
    let hours_are_every_hour =
        expand(segments[1], 0, 23).is_some_and(|hours| hours == (0..=23).collect::<Vec<i32>>());

    let minute_groups = shifted_groups(
        segments[0],
        minute_offset,
        0,
        59,
        days_are_wildcard && hours_are_every_hour,
    )?;

    let mut expressions = Vec::new();

    for (minute_carry, minute_values) in minute_groups {
        let hour_groups = shifted_groups(
            segments[1],
            hour_offset + minute_carry,
            0,
            23,
            days_are_wildcard,
        )?;

        for (hour_carry, hour_values) in hour_groups {
            let mut parts: [String; 5] = segments.map(str::to_string);
            parts[0].clone_from(&minute_values);
            parts[1] = hour_values;

            expressions.extend(expressions_for_hour_carry(segments, parts, hour_carry)?);
        }
    }

    Some(expressions)
}

/// Apply a day rollover (`hour_carry` of -1 or +1) to the day fields.
///
/// The two refusals here are the load-bearing ones. Cron ORs a restricted
/// day-of-week with a restricted day-of-month, so shifting both would
/// change which days match rather than relabel them. And a day-of-month
/// roll needs month lengths, which February does not have a single answer
/// for.
fn expressions_for_hour_carry(
    segments: &[&str; 5],
    mut parts: [String; 5],
    hour_carry: i32,
) -> Option<Vec<String>> {
    if hour_carry == 0 {
        return Some(vec![parts.join(" ")]);
    }

    if segments[4] != "*" {
        // Cron treats restricted day-of-month and day-of-week fields as an
        // "or", so they cannot shift together.
        if segments[2] != "*" || segments[3] != "*" {
            return None;
        }

        let days = expand(segments[4], 0, 7)?;
        parts[4] = collapse(&shift_wrapped(&days, hour_carry, 7, 0), 0, 6);

        return Some(vec![parts.join(" ")]);
    }

    if segments[2] == "*" && segments[3] == "*" {
        return Some(vec![parts.join(" ")]);
    }

    let day_groups = shift_days_of_month(segments[2], segments[3], hour_carry)?;

    Some(
        day_groups
            .into_iter()
            .map(|(days, months)| {
                let mut parts = parts.clone();
                parts[2] = days;
                parts[3] = months;
                parts.join(" ")
            })
            .collect(),
    )
}

/// Shift the day-of-month field by `carry`, respecting month lengths.
///
/// Days that roll out of one month land in the next, so the result is
/// grouped: every distinct day set gets its own `(day-of-month, month)`
/// pair, and each pair becomes one output expression. Refuses whenever the
/// roll would have to reason about February's length.
fn shift_days_of_month(
    dom_field: &str,
    month_field: &str,
    carry: i32,
) -> Option<Vec<(String, String)>> {
    let months: Vec<i32> = if month_field == "*" {
        (1..=12).collect()
    } else {
        expand(month_field, 1, 12)?
    };
    let days: Vec<i32> = if dom_field == "*" {
        (1..=31).collect()
    } else {
        expand(dom_field, 1, 31)?
    };

    // Insertion-ordered so the emitted expression order is deterministic
    // and matches the upstream port's array ordering.
    let mut shifted: Vec<(i32, Vec<i32>)> = Vec::new();

    for &month in &months {
        for &day in &days {
            if month == 2 && day == 29 {
                return None;
            }

            if day > days_in_month(month) {
                continue;
            }

            let (mut target_month, mut target_day) = (month, day + carry);

            while target_day < 1 {
                target_month = if target_month == 1 {
                    12
                } else {
                    target_month - 1
                };

                if target_month == 2 {
                    return None;
                }

                target_day += days_in_month(target_month);
            }

            while target_day > days_in_month(target_month) {
                if target_month == 2 {
                    return None;
                }

                target_day -= days_in_month(target_month);
                target_month = if target_month == 12 {
                    1
                } else {
                    target_month + 1
                };
            }

            match shifted.iter_mut().find(|(m, _)| *m == target_month) {
                Some((_, days)) => {
                    if !days.contains(&target_day) {
                        days.push(target_day);
                    }
                }
                None => shifted.push((target_month, vec![target_day])),
            }
        }
    }

    if shifted.is_empty() {
        return None;
    }

    // Months that ended up with the same day set share one expression.
    let mut groups: Vec<(Vec<i32>, Vec<i32>)> = Vec::new();
    for (month, mut days) in shifted {
        days.sort_unstable();
        match groups.iter_mut().find(|(key, _)| *key == days) {
            Some((_, months)) => months.push(month),
            None => groups.push((days, vec![month])),
        }
    }

    Some(
        groups
            .into_iter()
            .map(|(days, mut months)| {
                months.sort_unstable();
                (collapse(&days, 1, 31), collapse(&months, 1, 12))
            })
            .collect(),
    )
}

/// Shift one field by `offset` and group the results by carry direction.
///
/// Returns `(carry, collapsed field text)` pairs in ascending carry order.
/// A zero offset short-circuits to the field's original text so an
/// untouched field is never reformatted. `merge_carries` folds the groups
/// back into a single carry-0 group, which is only sound when the caller
/// has established that a rollover cannot change which days match.
fn shifted_groups(
    field: &str,
    offset: i32,
    min: i32,
    max: i32,
    merge_carries: bool,
) -> Option<Vec<(i32, String)>> {
    if offset == 0 {
        return Some(vec![(0, field.to_string())]);
    }

    let values = expand(field, min, max)?;
    let mut groups = shift_and_group_values(&values, offset, max - min + 1, min);

    if merge_carries && groups.len() > 1 {
        let mut merged: Vec<i32> = groups.iter().flat_map(|(_, g)| g.iter().copied()).collect();
        merged.sort_unstable();
        groups = vec![(0, merged)];
    }

    Some(
        groups
            .into_iter()
            .map(|(carry, group)| (carry, collapse(&group, min, max)))
            .collect(),
    )
}

/// Expand a cron field into the sorted, deduplicated list of values it
/// matches, or `None` when the field is not something this converter can
/// reason about.
///
/// The accepted grammar is the upstream port's: `*`, `N`, `N-M`, any of
/// those with a `/step` suffix, joined by commas. `N/step` means "from N to
/// the field maximum, every step" - the same reading `cron` itself uses.
/// A zero step, an out-of-range bound, an inverted range, or anything the
/// grammar does not cover all yield `None`, which the caller turns into a
/// refusal rather than a guess.
fn expand(field: &str, min: i32, max: i32) -> Option<Vec<i32>> {
    if field == "*" {
        return Some((min..=max).collect());
    }

    let mut values: Vec<i32> = Vec::new();

    for part in field.split(',') {
        let (head, step) = match part.split_once('/') {
            Some((head, step)) => (head, Some(parse_number(step)?)),
            None => (part, None),
        };

        if step == Some(0) {
            return None;
        }

        let (start, end) = if head == "*" {
            (min, max)
        } else if let Some((from, to)) = head.split_once('-') {
            (parse_number(from)?, parse_number(to)?)
        } else {
            let value = parse_number(head)?;
            (value, if step.is_none() { value } else { max })
        };

        if start < min || end > max || start > end {
            return None;
        }

        let mut value = start;
        while value <= end {
            values.push(value);
            value += step.unwrap_or(1);
        }
    }

    values.sort_unstable();
    values.dedup();

    Some(values)
}

/// Parse one bare decimal number, rejecting signs, whitespace and empties.
///
/// `str::parse` alone would accept `+5` and ` 5`, neither of which the
/// upstream grammar (`\d+`) allows; a value too large for `i32` is refused
/// for the same reason an out-of-range bound is.
fn parse_number(text: &str) -> Option<i32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<i32>().ok()
}

/// Shift values by `offset` within a `modulus`-wide field, grouping them by
/// how many whole field-widths each one crossed.
///
/// A group's carry is what the next field up has to absorb: minutes that
/// cross 60 push an hour, hours that cross 24 push a day.
fn shift_and_group_values(
    values: &[i32],
    offset: i32,
    modulus: i32,
    min: i32,
) -> Vec<(i32, Vec<i32>)> {
    let mut groups: Vec<(i32, Vec<i32>)> = Vec::new();

    for &value in values {
        let carry = (value + offset - min).div_euclid(modulus);
        let shifted = value + offset - carry * modulus;

        match groups.iter_mut().find(|(c, _)| *c == carry) {
            Some((_, group)) => group.push(shifted),
            None => groups.push((carry, vec![shifted])),
        }
    }

    for (_, group) in &mut groups {
        group.sort_unstable();
    }

    groups
}

/// Shift values that wrap within their own field instead of carrying out
/// of it - the day-of-week field, where Monday minus one day is Sunday
/// rather than a carry into the week before.
///
/// Expanding day-of-week over `0..=7` accepts cron's `7 = Sunday` alias;
/// wrapping modulo 7 folds it onto `0`.
fn shift_wrapped(values: &[i32], offset: i32, modulus: i32, min: i32) -> Vec<i32> {
    let mut shifted: Vec<i32> = values
        .iter()
        .map(|value| (value + offset - min).rem_euclid(modulus) + min)
        .collect();
    shifted.sort_unstable();
    shifted.dedup();
    shifted
}

/// Collapse a sorted value list back into the most compact cron field text.
///
/// Preferring `*`, then `a-b`, then `*/n` keeps the converted expression
/// readable: an operator comparing the listing against the expression they
/// wrote should not have to read a 24-entry comma list where `*` says the
/// same thing.
fn collapse(values: &[i32], min: i32, max: i32) -> String {
    if values.iter().copied().eq(min..=max) {
        return "*".to_string();
    }

    if values.len() == 1 {
        return values[0].to_string();
    }

    let steps: Vec<i32> = values.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let mut unique_steps = steps.clone();
    unique_steps.sort_unstable();
    unique_steps.dedup();

    if values.len() < 3 || unique_steps.len() > 1 {
        return collapse_runs(values);
    }

    let (first, last, step) = (values[0], values[values.len() - 1], steps[0]);

    if step == 1 {
        return format!("{first}-{last}");
    }

    if first == min && last + step > max {
        format!("*/{step}")
    } else {
        format!("{first}-{last}/{step}")
    }
}

/// Collapse consecutive runs of three or more values into ranges, leaving
/// shorter runs as comma-separated values.
///
/// Two-value runs stay listed because `15,16` is no longer than `15-16` and
/// reads more plainly.
fn collapse_runs(values: &[i32]) -> String {
    let mut pieces: Vec<String> = Vec::new();
    let mut index = 0;

    while index < values.len() {
        let mut end = index;
        while end + 1 < values.len() && values[end + 1] == values[end] + 1 {
            end += 1;
        }

        let run = &values[index..=end];
        if run.len() >= 3 {
            pieces.push(format!("{}-{}", run[0], run[run.len() - 1]));
        } else {
            pieces.push(run.iter().map(i32::to_string).collect::<Vec<_>>().join(","));
        }

        index = end + 1;
    }

    pieces.join(",")
}

#[cfg(test)]
mod tests {
    //! Every expectation below was derived by hand against Laravel's
    //! `CronExpressionTimezoneConverter`, then sanity-checked against the
    //! wall-clock meaning of the expression in both zones. Where the
    //! algorithm and an intuition disagreed, the algorithm's own source
    //! line is cited in the test.
    use super::*;
    use chrono::TimeZone as _;

    fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .expect("test instant must be unambiguous")
    }

    fn tz(name: &str) -> Tz {
        name.parse().expect("test timezone must be known")
    }

    /// A DST-stable pair of sample instants for zones that either never
    /// change offset (UTC, Asia/Tokyo, Asia/Kathmandu) or are mid-season on
    /// these dates (June, for the southern and northern zones used here).
    fn stable_samples() -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
        (Some(utc(2026, 6, 1, 0, 0)), Some(utc(2026, 6, 2, 0, 0)))
    }

    fn display(expr: &str, event: &str, display: &str) -> DisplayExpressions {
        let (next, next2) = stable_samples();
        expressions_for_display(
            &CronExpression::parse(expr).expect("test expression must parse"),
            tz(event),
            tz(display),
            next,
            next2,
        )
    }

    /// Assert the converter rewrote the expression into the display zone,
    /// and hand back the lines it produced.
    ///
    /// Going through the variant rather than comparing strings is the point:
    /// a rewrite that happens to reproduce its input is still a rewrite, and
    /// the caller labels the line from the variant.
    fn rewritten_as(expr: &str, event: &str, display_zone: &str) -> Vec<String> {
        match display(expr, event, display_zone) {
            DisplayExpressions::Rewritten(expressions) => expressions,
            DisplayExpressions::AsWritten(raw) => {
                panic!(
                    "expected `{expr}` to convert into {display_zone}, but it was refused: {raw}"
                )
            }
        }
    }

    /// Assert the converter refused, and hand back the untouched expression.
    fn refused(expr: &str, event: &str, display_zone: &str) -> String {
        match display(expr, event, display_zone) {
            DisplayExpressions::AsWritten(raw) => raw,
            DisplayExpressions::Rewritten(expressions) => panic!(
                "expected `{expr}` to be refused for {display_zone}, but it converted to {expressions:?}"
            ),
        }
    }

    #[test]
    fn whole_hour_shift_without_a_day_roll() {
        // UTC+9: 03:00 UTC is 12:00 JST the same day.
        assert_eq!(
            rewritten_as("0 3 * * *", "UTC", "Asia/Tokyo"),
            ["0 12 * * *"]
        );
    }

    #[test]
    fn day_roll_collapses_when_every_day_field_is_wildcard() {
        // 20:00 UTC is 05:00 JST *tomorrow*, but "every day" tomorrow is
        // still "every day", so the carry needs no second expression.
        assert_eq!(
            rewritten_as("0 20 * * *", "UTC", "Asia/Tokyo"),
            ["0 5 * * *"]
        );
    }

    #[test]
    fn sub_hour_offset_shifts_the_minute_field() {
        // Asia/Kathmandu is +05:45: 03:00 UTC is 08:45 NPT.
        assert_eq!(
            rewritten_as("0 3 * * *", "UTC", "Asia/Kathmandu"),
            ["45 8 * * *"]
        );
    }

    #[test]
    fn a_weekly_task_that_does_not_roll_leaves_day_of_week_alone() {
        // Pacific/Auckland is +12 in June (NZST): 03:00 UTC Monday is
        // 15:00 NZST the same Monday. `expressionsForHourCarry` returns
        // early on a zero carry, so the day-of-week field is untouched.
        assert_eq!(
            rewritten_as("0 3 * * 1", "UTC", "Pacific/Auckland"),
            ["0 15 * * 1"]
        );
    }

    #[test]
    fn a_weekly_task_that_rolls_forward_wraps_day_of_week() {
        // 20:00 UTC Monday is 08:00 NZST Tuesday.
        assert_eq!(
            rewritten_as("0 20 * * 1", "UTC", "Pacific/Auckland"),
            ["0 8 * * 2"]
        );
    }

    #[test]
    fn a_weekly_task_that_rolls_backward_wraps_day_of_week() {
        // America/Los_Angeles is -7 in June (PDT): 03:00 UTC Sunday is
        // 20:00 PDT Saturday, so Sunday (0) wraps to Saturday (6).
        assert_eq!(
            rewritten_as("0 3 * * 0", "UTC", "America/Los_Angeles"),
            ["0 20 * * 6"]
        );
    }

    #[test]
    fn a_split_across_midnight_renders_one_expression_per_side() {
        // 14:00 UTC Monday is 23:00 JST Monday; 20:00 UTC Monday is 05:00
        // JST Tuesday. Two carries, two expressions.
        assert_eq!(
            rewritten_as("0 14,20 * * 1", "UTC", "Asia/Tokyo"),
            ["0 23 * * 1", "0 5 * * 2"]
        );
    }

    #[test]
    fn a_sub_hour_offset_can_split_both_the_minute_and_the_hour_field() {
        // `10,50 * * * 1` in UTC, read in Kathmandu (+05:45):
        //   :10 UTC hours 00..18 -> :55 NPT hours 05..23 Monday
        //   :10 UTC hours 19..23 -> :55 NPT hours 00..04 Tuesday
        //   :50 UTC hours 00..17 -> :35 NPT hours 06..23 Monday
        //   :50 UTC hours 18..23 -> :35 NPT hours 00..05 Tuesday
        assert_eq!(
            rewritten_as("10,50 * * * 1", "UTC", "Asia/Kathmandu"),
            [
                "55 5-23 * * 1",
                "55 0-4 * * 2",
                "35 6-23 * * 1",
                "35 0-5 * * 2",
            ]
        );
    }

    #[test]
    fn an_all_day_step_survives_a_sub_hour_offset() {
        // Every 30 minutes is every 30 minutes anywhere; only the phase
        // moves. This is the merge-carries path in both fields.
        assert_eq!(
            rewritten_as("*/30 * * * *", "UTC", "Asia/Kathmandu"),
            ["15,45 * * * *"]
        );
    }

    #[test]
    fn identical_zones_leave_the_expression_untouched() {
        assert_eq!(
            refused("15 4 * * *", "Asia/Tokyo", "Asia/Tokyo"),
            "15 4 * * *"
        );
    }

    #[test]
    fn february_29_without_a_day_roll_still_converts() {
        // The Feb 29 refusal lives in `shiftDaysOfMonth`, which a zero hour
        // carry never reaches - so this converts normally.
        assert_eq!(
            rewritten_as("0 3 29 2 *", "UTC", "Asia/Tokyo"),
            ["0 12 29 2 *"]
        );
    }

    #[test]
    fn february_29_with_a_day_roll_refuses() {
        // 20:00 + 9h rolls the day, which drags Feb 29 into a calculation
        // that has no year - refuse and print what the user wrote.
        assert_eq!(refused("0 20 29 2 *", "UTC", "Asia/Tokyo"), "0 20 29 2 *");
    }

    #[test]
    fn a_roll_out_of_february_refuses() {
        // Feb 28 + 1 day is March 1 only in a non-leap year.
        assert_eq!(refused("0 20 28 2 *", "UTC", "Asia/Tokyo"), "0 20 28 2 *");
    }

    #[test]
    fn a_roll_into_february_refuses() {
        // March 1 minus one day is February 28 or 29 depending on the year.
        assert_eq!(
            refused("0 3 1 3 *", "UTC", "America/Los_Angeles"),
            "0 3 1 3 *"
        );
    }

    /// A day carry that crosses the December/January boundary is the one
    /// month roll that also changes the year, and `shiftDaysOfMonth` has to
    /// wrap the month field rather than run off the end of it. February is
    /// not involved either way, so both directions convert rather than
    /// refuse.
    ///
    /// America/Phoenix is used for the backward direction because it is
    /// -07:00 all year (no DST), which keeps the sampled offsets stable
    /// without picking a winter sample pair.
    #[test]
    fn a_day_carry_across_the_year_boundary_converts_in_both_directions() {
        // Backward: January 1 at 03:00 UTC is December 31 at 20:00 MST, so
        // the day-of-month wraps 1 -> 31 and the month wraps 1 -> 12.
        assert_eq!(
            rewritten_as("0 3 1 1 *", "UTC", "America/Phoenix"),
            ["0 20 31 12 *"]
        );

        // Forward: December 31 at 20:00 UTC is January 1 at 05:00 JST, so
        // the day-of-month wraps 31 -> 1 and the month wraps 12 -> 1.
        assert_eq!(
            rewritten_as("0 20 31 12 *", "UTC", "Asia/Tokyo"),
            ["0 5 1 1 *"]
        );
    }

    #[test]
    fn a_restricted_day_of_week_and_day_of_month_together_refuse() {
        // Cron ORs the two restricted day fields, so shifting both would
        // change which days match rather than relabel them.
        assert_eq!(refused("0 20 1 * 1", "UTC", "Asia/Tokyo"), "0 20 1 * 1");
    }

    #[test]
    fn a_dst_transition_between_the_next_two_runs_refuses() {
        // Europe/Berlin springs forward at 01:00 UTC on 2026-03-29, so the
        // Berlin-to-UTC offset is -60 minutes at the first sample and -120
        // at the second. No single expression covers both.
        let expr = CronExpression::parse("0 3 * * *").expect("cron");
        assert_eq!(
            expressions_for_display(
                &expr,
                tz("Europe/Berlin"),
                tz("UTC"),
                Some(utc(2026, 3, 28, 2, 0)),
                Some(utc(2026, 3, 29, 2, 0)),
            ),
            DisplayExpressions::AsWritten("0 3 * * *".to_string())
        );
    }

    #[test]
    fn the_same_pair_outside_the_transition_does_convert() {
        // The mirror of the test above: same zones, both samples in CET.
        let expr = CronExpression::parse("0 3 * * *").expect("cron");
        assert_eq!(
            expressions_for_display(
                &expr,
                tz("Europe/Berlin"),
                tz("UTC"),
                Some(utc(2026, 3, 27, 2, 0)),
                Some(utc(2026, 3, 28, 2, 0)),
            ),
            DisplayExpressions::Rewritten(vec!["0 2 * * *".to_string()])
        );
    }

    #[test]
    fn an_expression_that_never_fires_has_no_instant_to_sample() {
        let expr = CronExpression::parse("0 0 30 2 *").expect("cron");
        assert_eq!(
            expressions_for_display(&expr, tz("UTC"), tz("Asia/Tokyo"), None, None),
            DisplayExpressions::AsWritten("0 0 30 2 *".to_string())
        );
    }

    #[test]
    fn a_missing_second_sample_skips_the_dst_check() {
        // Laravel's `if ($nextAt && ...)` guard: with only one sample there
        // is nothing to compare, so the conversion proceeds.
        let expr = CronExpression::parse("0 3 * * *").expect("cron");
        assert_eq!(
            expressions_for_display(
                &expr,
                tz("UTC"),
                tz("Asia/Tokyo"),
                Some(utc(2026, 6, 1, 3, 0)),
                None,
            ),
            DisplayExpressions::Rewritten(vec!["0 12 * * *".to_string()])
        );
    }

    #[test]
    fn a_non_five_field_expression_is_refused() {
        assert!(convert_expression("0 3 * * * 2026", 9, 0).is_none());
        assert!(convert_expression("0 3 * *", 9, 0).is_none());
    }

    #[test]
    fn an_unexpandable_field_is_refused() {
        // Step zero, inverted range, out-of-range bound, and a malformed
        // part each reach `expand` and refuse the whole conversion.
        assert!(convert_expression("*/0 3 * * *", 0, 45).is_none());
        assert!(convert_expression("0 5-3 * * *", 9, 0).is_none());
        assert!(convert_expression("0 30 * * *", 9, 0).is_none());
        assert!(convert_expression("0 3-, * * *", 9, 0).is_none());
    }

    #[test]
    fn expand_covers_the_accepted_grammar() {
        assert_eq!(expand("*", 0, 59), Some((0..=59).collect::<Vec<_>>()));
        assert_eq!(expand("5", 0, 59), Some(vec![5]));
        assert_eq!(expand("5/10", 0, 59), Some(vec![5, 15, 25, 35, 45, 55]));
        assert_eq!(expand("1-5", 0, 23), Some(vec![1, 2, 3, 4, 5]));
        assert_eq!(expand("1-10/3", 0, 23), Some(vec![1, 4, 7, 10]));
        assert_eq!(expand("3,1,3", 0, 23), Some(vec![1, 3]));
        assert_eq!(expand("*/15", 0, 59), Some(vec![0, 15, 30, 45]));
        // Cron's Sunday alias survives expansion; the wrap folds it later.
        assert_eq!(expand("7", 0, 7), Some(vec![7]));
    }

    #[test]
    fn expand_refuses_what_it_cannot_represent() {
        assert_eq!(expand("*/0", 0, 59), None, "a zero step matches nothing");
        assert_eq!(expand("70", 0, 59), None, "out of range");
        assert_eq!(expand("5-3", 0, 59), None, "inverted range");
        assert_eq!(expand("1-", 0, 59), None, "malformed range");
        assert_eq!(expand("abc", 0, 59), None, "not numeric");
        assert_eq!(expand("+5", 0, 59), None, "signs are not cron syntax");
        assert_eq!(expand("1/2/3", 0, 59), None, "one step only");
        assert_eq!(
            expand("99999999999999", 0, 59),
            None,
            "a value too large for the field is still out of range"
        );
    }

    #[test]
    fn collapse_prefers_the_most_compact_form() {
        assert_eq!(collapse(&(0..=59).collect::<Vec<_>>(), 0, 59), "*");
        assert_eq!(collapse(&[5], 0, 59), "5");
        assert_eq!(collapse(&[1, 2, 3, 4], 0, 59), "1-4");
        assert_eq!(collapse(&[0, 15, 30, 45], 0, 59), "*/15");
        assert_eq!(collapse(&[5, 20, 35, 50], 0, 59), "5-50/15");
        assert_eq!(collapse(&[15, 35], 0, 59), "15,35");
        assert_eq!(collapse(&[1, 2, 3, 7, 9, 10, 11], 0, 59), "1-3,7,9-11");
    }

    #[test]
    fn shift_wrapped_folds_the_sunday_alias() {
        assert_eq!(shift_wrapped(&[0], -1, 7, 0), vec![6]);
        assert_eq!(shift_wrapped(&[6], 1, 7, 0), vec![0]);
        assert_eq!(shift_wrapped(&[0, 7], 1, 7, 0), vec![1]);
    }

    #[test]
    fn shift_and_group_values_reports_the_carry_it_pushed() {
        assert_eq!(
            shift_and_group_values(&[14, 20], 9, 24, 0),
            vec![(0, vec![23]), (1, vec![5])]
        );
        assert_eq!(
            shift_and_group_values(&[3], -7, 24, 0),
            vec![(-1, vec![20])]
        );
    }
}
