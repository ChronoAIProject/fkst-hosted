use std::collections::BTreeMap;

use crate::error::AppError;
use crate::goals::section_parse::split_sections;

use super::CronExpr;

/// The schedule portion of a parsed cron job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleSpec {
    pub cron: CronExpr,
    pub timezone: String,
}

/// A parsed scheduled job and its executable definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CronJobSpec {
    pub schedule: ScheduleSpec,
    pub job: JobDef,
}

/// The executable scheduled-job definitions supported by this increment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobDef {
    Raise {
        label: String,
        title: String,
        body: String,
    },
}

/// Parse the supported scheduled `raise` issue body.
pub fn parse_cron_job(body: &str) -> Result<CronJobSpec, AppError> {
    let sections: BTreeMap<_, _> = split_sections(body)?.into_iter().collect();
    let schedule = required_section(&sections, "### Schedule")?;
    let schedule_values = parse_schedule(schedule)?;

    let cron = CronExpr::parse(required_schedule_value(&schedule_values, "cron")?)?;
    let timezone = required_schedule_value(&schedule_values, "timezone")?;
    if timezone != "UTC" {
        return Err(AppError::Unprocessable(format!(
            "invalid schedule timezone: only `UTC` is supported, got {timezone:?}"
        )));
    }

    let job_type = required_single_line(&sections, "### Job Type")?;
    if job_type != "raise" {
        return Err(AppError::Unprocessable(format!(
            "unsupported `### Job Type` value {job_type:?}: only `raise` is supported"
        )));
    }

    let label = required_single_line(&sections, "### Raise Label")?.to_string();
    let title = required_single_line(&sections, "### Raise Title")?.to_string();
    let raise_body = required_content(&sections, "### Raise Body")?.to_string();

    Ok(CronJobSpec {
        schedule: ScheduleSpec {
            cron,
            timezone: timezone.to_string(),
        },
        job: JobDef::Raise {
            label,
            title,
            body: raise_body,
        },
    })
}

fn parse_schedule(block: &str) -> Result<BTreeMap<String, String>, AppError> {
    let mut values = BTreeMap::new();
    for line in block.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let (key, value) = line.split_once(':').ok_or_else(|| {
            AppError::Unprocessable(format!(
                "invalid schedule field {line:?}: expected `key: value`"
            ))
        })?;
        let key = key.trim();
        let value = value.trim();
        if !matches!(key, "cron" | "timezone") {
            return Err(AppError::Unprocessable(format!(
                "unknown schedule field {key:?}"
            )));
        }
        if value.is_empty() {
            return Err(AppError::Unprocessable(format!(
                "schedule field `{key}` must not be empty"
            )));
        }
        if values.insert(key.to_string(), value.to_string()).is_some() {
            return Err(AppError::Unprocessable(format!(
                "duplicate schedule field `{key}`"
            )));
        }
    }
    Ok(values)
}

fn required_schedule_value<'a>(
    values: &'a BTreeMap<String, String>,
    key: &str,
) -> Result<&'a str, AppError> {
    values
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| AppError::Unprocessable(format!("missing required schedule field `{key}`")))
}

fn required_section<'a>(
    sections: &'a BTreeMap<String, String>,
    heading: &str,
) -> Result<&'a str, AppError> {
    sections
        .get(heading)
        .map(String::as_str)
        .ok_or_else(|| AppError::Unprocessable(format!("missing required section `{heading}`")))
}

fn required_single_line<'a>(
    sections: &'a BTreeMap<String, String>,
    heading: &str,
) -> Result<&'a str, AppError> {
    let content = required_content(sections, heading)?;
    if content.lines().count() != 1 {
        return Err(AppError::Unprocessable(format!(
            "the `{heading}` section must contain exactly one non-empty line"
        )));
    }
    Ok(content)
}

fn required_content<'a>(
    sections: &'a BTreeMap<String, String>,
    heading: &str,
) -> Result<&'a str, AppError> {
    let content = required_section(sections, heading)?.trim();
    if content.is_empty() {
        return Err(AppError::Unprocessable(format!(
            "the `{heading}` section must not be empty"
        )));
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = "### Schedule\ncron: 0 3 * * *\ntimezone: UTC\n\n### Job Type\nraise\n\n### Raise Label\nfkst-dev\n\n### Raise Title\nDaily maintenance\n\n### Raise Body\nRun maintenance.\n";

    fn error_message(body: &str) -> String {
        match parse_cron_job(body) {
            Err(AppError::Unprocessable(message)) => message,
            other => panic!("expected unprocessable parse error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_schedule_fields() {
        let body = BODY.replace("timezone: UTC", "timezone: UTC\njitter: 5m");
        assert!(error_message(&body).contains("jitter"));
    }

    #[test]
    fn rejects_non_utc_timezone() {
        let body = BODY.replace("timezone: UTC", "timezone: America/New_York");
        assert!(error_message(&body).contains("UTC"));
    }

    #[test]
    fn rejects_unimplemented_job_types() {
        let body = BODY.replace("### Job Type\nraise", "### Job Type\nworkflow");
        assert!(error_message(&body).contains("raise"));
    }

    #[test]
    fn requires_each_accepted_raise_field() {
        for heading in ["### Raise Label", "### Raise Title", "### Raise Body"] {
            let body = BODY.replace(heading, "### Missing");
            assert!(error_message(&body).contains(heading));
        }
    }
}
