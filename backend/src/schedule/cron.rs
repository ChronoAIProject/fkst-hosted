use k8s_openapi::chrono::{DateTime, Days, TimeZone, Utc};

use crate::error::AppError;

/// A daily five-field cron expression in the supported `M H * * *` subset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CronExpr {
    minute: u32,
    hour: u32,
}

impl CronExpr {
    /// Parse the walking-skeleton subset `M H * * *`.
    pub fn parse(expression: &str) -> Result<Self, AppError> {
        let fields: Vec<_> = expression.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(invalid_cron(
                "cron",
                "must contain exactly five whitespace-separated fields",
            ));
        }

        let minute = parse_number(fields[0], "minute", 59)?;
        let hour = parse_number(fields[1], "hour", 23)?;
        if fields[2] != "*" {
            return Err(invalid_cron("day-of-month", "only `*` is supported"));
        }
        if fields[3] != "*" {
            return Err(invalid_cron("month", "only `*` is supported"));
        }
        if fields[4] != "*" {
            return Err(invalid_cron("day-of-week", "only `*` is supported"));
        }

        Ok(Self { minute, hour })
    }

    /// Return the first matching UTC instant strictly after `after`.
    pub fn next_after(&self, after: DateTime<Utc>) -> Result<DateTime<Utc>, AppError> {
        let date = after.date_naive();
        let time = date
            .and_hms_opt(self.hour, self.minute, 0)
            .expect("validated cron hour and minute form a valid UTC time");
        let candidate = Utc.from_utc_datetime(&time);
        if candidate > after {
            return Ok(candidate);
        }

        let next_date = date.checked_add_days(Days::new(1)).ok_or_else(|| {
            AppError::Validation("schedule slot exceeds the supported UTC date range".to_string())
        })?;
        let next_time = next_date
            .and_hms_opt(self.hour, self.minute, 0)
            .expect("validated cron hour and minute form a valid UTC time");
        Ok(Utc.from_utc_datetime(&next_time))
    }
}

fn parse_number(value: &str, field: &str, max: u32) -> Result<u32, AppError> {
    if value.is_empty() || !value.chars().all(|character| character.is_ascii_digit()) {
        return Err(invalid_cron(
            field,
            &format!("must be an integer in the range 0..={max}"),
        ));
    }
    let parsed = value
        .parse::<u32>()
        .map_err(|_| invalid_cron(field, &format!("must be an integer in the range 0..={max}")))?;
    if parsed > max {
        return Err(invalid_cron(
            field,
            &format!("must be in the range 0..={max}"),
        ));
    }
    Ok(parsed)
}

fn invalid_cron(field: &str, detail: &str) -> AppError {
    AppError::Unprocessable(format!("invalid cron {field} field: {detail}"))
}

#[cfg(test)]
mod tests {
    use k8s_openapi::chrono::{TimeZone, Utc};

    use super::*;

    fn error_message(expression: &str) -> String {
        match CronExpr::parse(expression) {
            Err(AppError::Unprocessable(message)) => message,
            other => panic!("expected unprocessable cron error, got {other:?}"),
        }
    }

    #[test]
    fn parse_accepts_minute_and_hour_boundaries() {
        assert!(CronExpr::parse("0 0 * * *").is_ok());
        assert!(CronExpr::parse("59 23 * * *").is_ok());
    }

    #[test]
    fn parse_names_invalid_fields() {
        assert!(error_message("60 3 * * *").contains("minute"));
        assert!(error_message("0 24 * * *").contains("hour"));
        assert!(error_message("0 3 1 * *").contains("day-of-month"));
        assert!(error_message("0 3 * 1 *").contains("month"));
        assert!(error_message("0 3 * * 1").contains("day-of-week"));
        assert!(error_message("0 3 * *").contains("cron"));
    }

    #[test]
    fn next_after_is_strict_and_rolls_to_the_next_day() {
        let cron = CronExpr::parse("0 3 * * *").expect("valid cron");
        let before = Utc
            .with_ymd_and_hms(2026, 7, 27, 2, 0, 0)
            .single()
            .expect("valid timestamp");
        let at_slot = Utc
            .with_ymd_and_hms(2026, 7, 27, 3, 0, 0)
            .single()
            .expect("valid timestamp");
        let next_day = Utc
            .with_ymd_and_hms(2026, 7, 28, 3, 0, 0)
            .single()
            .expect("valid timestamp");

        assert_eq!(cron.next_after(before).expect("slot"), at_slot);
        assert_eq!(cron.next_after(at_slot).expect("slot"), next_day);
    }
}
