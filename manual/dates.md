# Dates

`Date` reads a date or a time from text: an exact form such as
`2026-10-09 09:30`, or a relative one such as `tomorrow`, `+2 days` or
`next monday`. It answers a `chrono::DateTime`, so you compare, format and
store the result with chrono, and the relative forms read the clock the
framework reads, so a test that freezes the clock freezes them too.

```rust
use suprnova::Date;

let due = Date::parse("tomorrow 09:30")?;       // DateTime<Utc>
let expires = Date::parse("+2 weeks")?;
let launch = Date::parse("2026-11-01T12:00:00+02:00")?;
assert!(Date::parse("soonish").is_err());
# Ok::<(), suprnova::DateParseError>(())
```

## The forms it reads

`Date::parse(text)` reads these forms, ignoring case and extra spaces,
and answers a time in UTC:

| Form | Example | Answers |
|---|---|---|
| RFC 3339 | `2026-10-09T12:00:00+02:00` | That moment |
| A date | `2026-10-09` | That day at midnight |
| A date and a time, with a space or `T` | `2026-10-09 09:30`, `2026-10-09T09:30:15.250` | That time |
| Unix seconds | `@1791547200` | That moment |
| A word | `now`, `today`, `tomorrow`, `yesterday`, `midnight`, `noon` | See below |
| A count of units from now | `+2 days`, `-30 minutes`, `5 hours`, `3 weeks ago` | See below |
| A weekday | `monday`, `next friday`, `last sunday` | That day at midnight |
| The first or last day of a month | `first day of next month`, `last day of this month` | That day, at the current time |
| A word form and a time | `tomorrow 09:30`, `next monday 08:00`, `last day of this month 23:59` | That day at that time |

The units are `second`, `minute`, `hour`, `day`, `week`, `fortnight`,
`month` and `year`, singular or plural. `N <unit> ago` counts back.

The time of day follows PHP's relative formats, which Laravel's `Date`
reads through Carbon:

- `today`, `tomorrow`, `yesterday` and `midnight` are at midnight, and
  `noon` at 12:00.
- A weekday is at midnight. A weekday alone is today when today is that
  day; `next` is strictly after today and `last` strictly before.
- `now`, a count of units and the first or last day of a month keep the
  current time of day.
- A time after a word form, `HH:MM`, sets the time.

Days, weeks, fortnights, months and years move the calendar date and keep
the wall-clock time; seconds, minutes and hours move the moment. A month
that has no such day overflows into the next month, as PHP's does: one
month after January 31 is March 3 in a common year.

Text it does not read is an error, a `DateParseError` that names the
text:

```rust
use suprnova::Date;

let error = Date::parse("in a fortnight-ish").unwrap_err();
assert_eq!(error.text(), "in a fortnight-ish");
```

`DateParseError` converts into `FrameworkError`, so a handler can use `?`.

## Time zones

`Date::parse_in(text, tz)` reads the same forms in a time zone and
answers a `DateTime<Tz>`. A date, a time without an offset and every
relative form are taken in that zone, so `tomorrow` is the next midnight
in Paris:

```rust
use suprnova::{Date, Tz};

let paris: Tz = "Europe/Paris".parse().expect("a known zone");
let tomorrow = Date::parse_in("tomorrow", paris)?;
let meeting = Date::parse_in("2026-10-09 09:30", paris)?; // 07:30 UTC
# Ok::<(), suprnova::DateParseError>(())
```

A local time that a daylight-saving change skips moves forward by the
length of the skip, as PHP's does: `2026-03-29 02:30` in Paris is 03:30.
A local time that a change repeats is the earlier of the two.

`Date::raw_parse(text)` is the same as `Date::parse`. Laravel's `rawParse`
skips the hook Carbon runs on every parse, and Suprnova has no such hook.

## Testing with a frozen clock

Every relative form reads `suprnova::clock::now()`, so freeze the clock
with `TestClock` to test code that parses relative dates:

```rust
use chrono::{DateTime, Utc};
use suprnova::Date;
use suprnova::testing::TestClock;

#[test]
fn a_reminder_is_due_tomorrow_morning() {
    let friday_noon: DateTime<Utc> = "2026-10-09T12:00:00Z".parse().unwrap();
    let _clock = TestClock::travel_to(friday_noon);

    let due = Date::parse("tomorrow 09:30").unwrap();
    assert_eq!(due.to_rfc3339(), "2026-10-10T09:30:00+00:00");
}
```

See [Testing](testing.md) for `TestClock`.

### Why Suprnova diverges

- **A named set of forms.** Carbon reads PHP's whole relative-format
  grammar, including forms such as `third friday of next month` or
  `+1 week 2 days`. `Date::parse` reads the forms listed above and refuses
  the rest, because a form read differently from PHP would be worse than
  a form refused. Combine forms with chrono's arithmetic instead.
- **Only English weekday names, spelled in full.** PHP also reads
  abbreviations such as `mon`.
- **A time follows a word form only.** PHP reads `+2 days 09:30`; here a
  count of units takes no time after it.
- **It answers chrono types.** Laravel answers a mutable Carbon instance;
  here you get a `DateTime<Utc>` or `DateTime<Tz>` value.

## Next

- [Strings](strings.md) - the other text helpers
- [Localization](localization.md) - formatting a date for a reader's
  locale
- [Testing](testing.md) - `TestClock` and the other test helpers
