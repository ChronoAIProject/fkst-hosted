use std::collections::BTreeMap;

use k8s_openapi::chrono::{DateTime, SecondsFormat, Utc};

use crate::error::AppError;

const MARKER_PREFIX: &str = "<!-- fkst-cron-run:v1";

/// The completed status supported by the walking skeleton.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunStatus {
    Ok,
}

/// A durable scheduled-run record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRecord {
    pub slot: DateTime<Utc>,
    pub manual: bool,
    pub status: RunStatus,
    pub started: DateTime<Utc>,
    pub ended: Option<DateTime<Utc>>,
    pub issue: Option<u64>,
}

/// Render the predecessor-compatible `fkst-cron-run:v1` hidden marker.
pub fn render_marker(record: &RunRecord) -> String {
    let mut fields = vec![
        format!("slot=\"{}\"", timestamp(record.slot)),
        format!("manual=\"{}\"", record.manual),
        "status=\"ok\"".to_string(),
        format!("started=\"{}\"", timestamp(record.started)),
    ];
    if let Some(ended) = record.ended {
        fields.push(format!("ended=\"{}\"", timestamp(ended)));
    }
    if let Some(issue) = record.issue {
        fields.push(format!("issue=\"{issue}\""));
    }
    format!("{MARKER_PREFIX} {} -->", fields.join(" "))
}

/// Parse one marker while tolerating field order and unrecognized extra fields.
pub fn parse_marker(marker: &str) -> Result<RunRecord, AppError> {
    let marker = marker.trim();
    let attributes = marker
        .strip_prefix(MARKER_PREFIX)
        .and_then(|rest| rest.strip_suffix("-->"))
        .ok_or_else(|| invalid_marker("expected an `fkst-cron-run:v1` HTML marker"))?;
    let fields = parse_attributes(attributes.trim())?;

    Ok(RunRecord {
        slot: parse_timestamp(required(&fields, "slot")?, "slot")?,
        manual: parse_bool(required(&fields, "manual")?, "manual")?,
        status: parse_status(required(&fields, "status")?)?,
        started: parse_timestamp(required(&fields, "started")?, "started")?,
        ended: fields
            .get("ended")
            .map(|value| parse_timestamp(value, "ended"))
            .transpose()?,
        issue: fields
            .get("issue")
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| invalid_marker("field `issue` must be an unsigned issue number"))
            })
            .transpose()?,
    })
}

fn parse_attributes(input: &str) -> Result<BTreeMap<String, String>, AppError> {
    let mut fields = BTreeMap::new();
    let mut rest = input;
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let equals = rest
            .find('=')
            .ok_or_else(|| invalid_marker("expected a marker field assignment"))?;
        let key = rest[..equals].trim();
        if key.is_empty() || key.chars().any(char::is_whitespace) {
            return Err(invalid_marker("marker field name is invalid"));
        }
        rest = &rest[equals + 1..];
        let quoted = rest
            .strip_prefix('"')
            .ok_or_else(|| invalid_marker("marker field values must be quoted"))?;
        let quote = quoted
            .find('"')
            .ok_or_else(|| invalid_marker("marker field has an unterminated value"))?;
        let value = &quoted[..quote];
        rest = &quoted[quote + 1..];
        if fields.insert(key.to_string(), value.to_string()).is_some() {
            return Err(invalid_marker(&format!("duplicate marker field `{key}`")));
        }
    }
    Ok(fields)
}

fn required<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, AppError> {
    fields
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| invalid_marker(&format!("missing required marker field `{key}`")))
}

fn parse_timestamp(value: &str, field: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| invalid_marker(&format!("field `{field}` must be an RFC 3339 timestamp")))
}

fn parse_bool(value: &str, field: &str) -> Result<bool, AppError> {
    value
        .parse::<bool>()
        .map_err(|_| invalid_marker(&format!("field `{field}` must be `true` or `false`")))
}

fn parse_status(value: &str) -> Result<RunStatus, AppError> {
    match value {
        "ok" => Ok(RunStatus::Ok),
        _ => Err(invalid_marker("field `status` must be `ok`")),
    }
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn invalid_marker(detail: &str) -> AppError {
    AppError::Unprocessable(format!("invalid fkst-cron-run:v1 marker: {detail}"))
}

#[cfg(test)]
mod tests {
    use k8s_openapi::chrono::{TimeZone, Utc};

    use super::*;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 27, hour, 0, 0)
            .single()
            .expect("valid timestamp")
    }

    #[test]
    fn optional_fields_may_be_absent() {
        let record = RunRecord {
            slot: at(3),
            manual: false,
            status: RunStatus::Ok,
            started: at(3),
            ended: None,
            issue: None,
        };
        assert_eq!(
            parse_marker(&render_marker(&record)).expect("marker"),
            record
        );
    }

    #[test]
    fn parsing_tolerates_order_and_unknown_fields() {
        let marker = "<!-- fkst-cron-run:v1 unknown=\"future\" status=\"ok\" issue=\"42\" started=\"2026-07-27T03:00:00Z\" manual=\"false\" slot=\"2026-07-27T03:00:00Z\" -->";
        let record = parse_marker(marker).expect("marker");
        assert_eq!(record.issue, Some(42));
        assert_eq!(record.slot, at(3));
    }
}
