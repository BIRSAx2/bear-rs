use anyhow::{Result, anyhow, bail};
use chrono::{Datelike, Duration, Local, LocalResult, NaiveDate, TimeZone};

const APPLE_EPOCH_OFFSET_SECONDS: i64 = 978_307_200;

pub fn parse_bear_date_filter(input: &str) -> Result<i64> {
    let now = Local::now();
    let today = now.date_naive();

    let date = match input {
        "today" => today,
        "yesterday" => today - Duration::days(1),
        "last-week" => today - Duration::days(7),
        "last-month" => shift_months(today, -1),
        "last-year" => shift_years(today, -1),
        other => NaiveDate::parse_from_str(other, "%Y-%m-%d")
            .map_err(|_| anyhow!("invalid date filter: {other}"))?,
    };

    let datetime = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("invalid date value"))?;
    let local = match Local.from_local_datetime(&datetime) {
        LocalResult::Single(value) => value,
        LocalResult::Ambiguous(earliest, _) => earliest,
        LocalResult::None => bail!("could not resolve local date for {input}"),
    };
    Ok(local.timestamp() - APPLE_EPOCH_OFFSET_SECONDS)
}

fn shift_months(date: NaiveDate, delta: i32) -> NaiveDate {
    let total_months = date.year() * 12 + date.month0() as i32 + delta;
    let year = total_months.div_euclid(12);
    let month0 = total_months.rem_euclid(12) as u32;
    let last_day = last_day_of_month(year, month0 + 1);
    let day = date.day().min(last_day);
    NaiveDate::from_ymd_opt(year, month0 + 1, day).expect("valid shifted month date")
}

fn shift_years(date: NaiveDate, delta: i32) -> NaiveDate {
    let year = date.year() + delta;
    let last_day = last_day_of_month(year, date.month());
    let day = date.day().min(last_day);
    NaiveDate::from_ymd_opt(year, date.month(), day).expect("valid shifted year date")
}

fn last_day_of_month(year: i32, month: u32) -> u32 {
    for day in (28..=31).rev() {
        if NaiveDate::from_ymd_opt(year, month, day).is_some() {
            return day;
        }
    }
    28
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, Duration, Local, TimeZone};

    use super::parse_bear_date_filter;

    const APPLE_EPOCH_OFFSET_SECONDS: i64 = 978_307_200;

    #[test]
    fn parses_absolute_date() {
        let parsed = parse_bear_date_filter("2026-04-01").expect("date should parse");
        let expected = Local
            .with_ymd_and_hms(2026, 4, 1, 0, 0, 0)
            .earliest()
            .expect("valid local date")
            .timestamp()
            - APPLE_EPOCH_OFFSET_SECONDS;
        assert_eq!(parsed, expected);
    }

    #[test]
    fn parses_relative_dates() {
        let now = Local::now();
        let today = now.date_naive();
        let expected_yesterday = Local
            .from_local_datetime(
                &(today - Duration::days(1))
                    .and_hms_opt(0, 0, 0)
                    .expect("valid midnight"),
            )
            .earliest()
            .expect("valid local datetime")
            .timestamp()
            - APPLE_EPOCH_OFFSET_SECONDS;

        let parsed = parse_bear_date_filter("yesterday").expect("yesterday should parse");
        assert_eq!(parsed, expected_yesterday);
    }

    #[test]
    fn parses_last_month_with_clamped_day() {
        let now = Local::now().date_naive();
        if now.month() == 3 && now.day() == 31 {
            let parsed = parse_bear_date_filter("last-month").expect("last-month should parse");
            let expected = Local
                .with_ymd_and_hms(now.year(), 2, 28, 0, 0, 0)
                .earliest()
                .expect("valid local date")
                .timestamp()
                - APPLE_EPOCH_OFFSET_SECONDS;
            assert_eq!(parsed, expected);
        }
    }
}
