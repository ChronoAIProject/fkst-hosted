use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use fkst_qa_contracts::{compare_iso8601_timestamps, validate_scalar};

use crate::journal::{Journal, OwnedHandle};
use crate::RunError;

pub const RUN_ID_LABEL: &str = "fkst.local-qa/run-id";
pub const PROFILE_ID_LABEL: &str = "fkst.local-qa/profile-id";
pub const ENVIRONMENT_ID_LABEL: &str = "fkst.local-qa/environment-id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentRequest {
    pub intent_id: String,
    pub run_id: String,
    pub profile_id: String,
    pub environment_id: String,
    pub generation: i64,
    pub deadline_utc: String,
    pub provider_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRequest {
    pub stable_provider_key: String,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderResource {
    pub stable_provider_key: String,
    pub labels: BTreeMap<String, String>,
    pub provider_identity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderResourceState {
    Active,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStatusReceipt {
    pub resource: ProviderResource,
    pub state: ProviderResourceState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStopReceipt {
    pub resource: ProviderResource,
    pub stopped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadinessEndpointClass {
    LoopbackHttp,
    LoopbackTcp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessRequest {
    pub service_id: String,
    pub endpoint_class: ReadinessEndpointClass,
    pub deadline_utc: String,
    pub max_attempts: u32,
    pub max_duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderReadinessReceipt {
    pub resource: ProviderResource,
    pub service_id: String,
    pub endpoint_class: ReadinessEndpointClass,
    pub endpoint: SocketAddr,
    pub observed_at_utc: String,
    pub attempts: u32,
    pub elapsed_ms: u64,
    pub ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentStatus {
    Active,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentStopReceipt {
    pub intent_id: String,
    pub provider_identity: String,
    pub already_stopped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessReceipt {
    pub intent_id: String,
    pub provider_identity: String,
    pub service_id: String,
    pub endpoint_class: ReadinessEndpointClass,
    pub endpoint: SocketAddr,
    pub deadline_utc: String,
    pub attempts: u32,
    pub elapsed_ms: u64,
}

pub trait EnvironmentProvider {
    fn discover(&mut self, stable_provider_key: &str)
        -> Result<Option<ProviderResource>, RunError>;
    fn create(&mut self, request: CreateRequest) -> Result<ProviderResource, RunError>;

    fn status(&mut self, _resource: &ProviderResource) -> Result<ProviderStatusReceipt, RunError> {
        Err(RunError::Lifecycle(
            "environment provider status is unavailable",
        ))
    }

    fn stop(&mut self, _resource: &ProviderResource) -> Result<ProviderStopReceipt, RunError> {
        Err(RunError::Lifecycle(
            "environment provider stop is unavailable",
        ))
    }

    fn readiness(
        &mut self,
        _resource: &ProviderResource,
        _request: &ReadinessRequest,
    ) -> Result<ProviderReadinessReceipt, RunError> {
        Err(RunError::Lifecycle(
            "environment provider readiness is unavailable",
        ))
    }
}

/// A trusted Host observation in the contract's canonical UTC grammar. Implementors
/// must sample at each call (FixedClock deliberately freezes time for tests).
/// Lifecycle checks observe deadlines before effects and after callbacks return;
/// they cannot interrupt a hung synchronous provider, bound elapsed durations, or
/// compensate for wall-clock rollback. No deadline is extended by these checks.
pub trait Clock {
    fn now_utc(&self) -> Result<String, RunError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc(&self) -> Result<String, RunError> {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RunError::InvalidJournal("system clock is before Unix epoch"))?;
        Ok(format_utc_time(elapsed))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedClock {
    now_utc: String,
}

impl FixedClock {
    pub fn new(now_utc: impl Into<String>) -> Result<Self, RunError> {
        let now_utc = now_utc.into();
        validate_scalar("ISO8601", &now_utc)
            .map_err(|_| RunError::InvalidJournal("now_utc must be ISO8601"))?;
        Ok(Self { now_utc })
    }
}

impl Clock for FixedClock {
    fn now_utc(&self) -> Result<String, RunError> {
        Ok(self.now_utc.clone())
    }
}

pub fn stable_provider_key(intent_id: &str) -> String {
    format!("fkst-local-qa/environment/v1/{intent_id}")
}

pub fn ownership_labels(request: &EnvironmentRequest) -> BTreeMap<String, String> {
    BTreeMap::from([
        (RUN_ID_LABEL.to_owned(), request.run_id.clone()),
        (PROFILE_ID_LABEL.to_owned(), request.profile_id.clone()),
        (
            ENVIRONMENT_ID_LABEL.to_owned(),
            request.environment_id.clone(),
        ),
    ])
}

pub fn reconcile_environment<P: EnvironmentProvider>(
    journal: &mut Journal,
    provider: &mut P,
    request: &EnvironmentRequest,
    clock: &impl Clock,
) -> Result<OwnedHandle, RunError> {
    validate_request(request)?;
    if let Some(handle) = journal.owned_handle(&request.intent_id)? {
        validate_handle_request(request, &handle)?;
        return Ok(handle);
    }
    ensure_before_deadline(clock, &request.deadline_utc)?;
    let intent = journal.prepare_intent(
        &request.intent_id,
        &request.run_id,
        &request.profile_id,
        &request.environment_id,
        request.generation,
        &request.deadline_utc,
    )?;
    if intent.status != "prepared" {
        return Err(RunError::InvalidJournal(
            "resource intent is not available for binding",
        ));
    }
    ensure_before_deadline(clock, &intent.deadline_utc)?;

    let expected_key = stable_provider_key(&request.intent_id);
    let expected_labels = ownership_labels(request);
    let resource = match provider.discover(&expected_key)? {
        Some(resource) => resource,
        None => {
            ensure_before_deadline(clock, &intent.deadline_utc)?;
            provider.create(CreateRequest {
                stable_provider_key: expected_key.clone(),
                labels: expected_labels.clone(),
            })?
        }
    };
    validate_provider_resource(request, &expected_key, &expected_labels, &resource)?;

    let handle = journal.record_handle(&OwnedHandle {
        intent_id: request.intent_id.clone(),
        run_id: request.run_id.clone(),
        profile_id: request.profile_id.clone(),
        environment_id: request.environment_id.clone(),
        generation: request.generation,
        deadline_utc: request.deadline_utc.clone(),
        stable_provider_key: expected_key,
        provider_identity: resource.provider_identity,
        state: "active".to_owned(),
    })?;
    // Preserve the obtained identity even if the callback returned late or the
    // Host clock now fails. The committed handle remains available for safety work.
    ensure_before_deadline(clock, &intent.deadline_utc)?;
    Ok(handle)
}

pub fn environment_status<P: EnvironmentProvider>(
    provider: &mut P,
    handle: &OwnedHandle,
) -> Result<EnvironmentStatus, RunError> {
    let expected = resource_from_handle(handle)?;
    let receipt = provider.status(&expected)?;
    validate_receipt_resource(&expected, &receipt.resource)?;
    match receipt.state {
        ProviderResourceState::Active => Ok(EnvironmentStatus::Active),
        ProviderResourceState::Stopped => Ok(EnvironmentStatus::Stopped),
        ProviderResourceState::Unknown => Err(RunError::Lifecycle(
            "environment provider ownership is unknown",
        )),
    }
}

pub fn stop_environment<P: EnvironmentProvider>(
    provider: &mut P,
    handle: &OwnedHandle,
) -> Result<EnvironmentStopReceipt, RunError> {
    let expected = resource_from_handle(handle)?;
    let status = provider.status(&expected)?;
    validate_receipt_resource(&expected, &status.resource)?;
    match status.state {
        ProviderResourceState::Stopped => {
            return Ok(EnvironmentStopReceipt {
                intent_id: handle.intent_id.clone(),
                provider_identity: handle.provider_identity.clone(),
                already_stopped: true,
            });
        }
        ProviderResourceState::Unknown => {
            return Err(RunError::Lifecycle(
                "environment provider ownership is unknown",
            ));
        }
        ProviderResourceState::Active => {}
    }
    let receipt = provider.stop(&expected)?;
    validate_receipt_resource(&expected, &receipt.resource)?;
    if !receipt.stopped {
        return Err(RunError::Lifecycle(
            "environment stop receipt did not prove termination",
        ));
    }
    Ok(EnvironmentStopReceipt {
        intent_id: handle.intent_id.clone(),
        provider_identity: handle.provider_identity.clone(),
        already_stopped: false,
    })
}

pub fn check_environment_readiness<P: EnvironmentProvider>(
    provider: &mut P,
    handle: &OwnedHandle,
    request: &ReadinessRequest,
    clock: &impl Clock,
) -> Result<ReadinessReceipt, RunError> {
    validate_readiness_request(request)?;
    ensure_before_deadline(clock, &request.deadline_utc)?;
    if compare_iso8601_timestamps(&request.deadline_utc, &handle.deadline_utc)
        .map_err(|_| RunError::Lifecycle("environment deadline must be ISO8601"))?
        .is_gt()
    {
        return Err(RunError::Lifecycle(
            "readiness deadline exceeds the environment deadline",
        ));
    }
    let expected = resource_from_handle(handle)?;
    let receipt = provider.readiness(&expected, request)?;
    ensure_before_deadline(clock, &request.deadline_utc)?;
    validate_receipt_resource(&expected, &receipt.resource)?;
    let observed_before_deadline =
        compare_iso8601_timestamps(&receipt.observed_at_utc, &request.deadline_utc)
            .map_err(|_| RunError::Lifecycle("readiness observation time must be ISO8601"))?
            .is_lt();
    if receipt.service_id != request.service_id
        || receipt.endpoint_class != request.endpoint_class
        || receipt.attempts == 0
        || receipt.attempts > request.max_attempts
        || receipt.elapsed_ms > request.max_duration_ms
        || !observed_before_deadline
    {
        return Err(RunError::Lifecycle(
            "readiness receipt does not match service identity or budget",
        ));
    }
    if !receipt.endpoint.ip().is_loopback() || receipt.endpoint.port() == 0 {
        return Err(RunError::Lifecycle(
            "readiness endpoint must be an allocated loopback port",
        ));
    }
    if !receipt.ready {
        return Err(RunError::Lifecycle(
            "environment did not become ready within the bounded probe",
        ));
    }
    Ok(ReadinessReceipt {
        intent_id: handle.intent_id.clone(),
        provider_identity: handle.provider_identity.clone(),
        service_id: receipt.service_id,
        endpoint_class: receipt.endpoint_class,
        endpoint: receipt.endpoint,
        deadline_utc: request.deadline_utc.clone(),
        attempts: receipt.attempts,
        elapsed_ms: receipt.elapsed_ms,
    })
}

fn validate_request(request: &EnvironmentRequest) -> Result<(), RunError> {
    if request.provider_identity.is_empty() {
        return Err(RunError::InvalidJournal(
            "provider identity must not be empty",
        ));
    }
    Ok(())
}

fn validate_readiness_request(request: &ReadinessRequest) -> Result<(), RunError> {
    if request.service_id.is_empty() || request.max_attempts == 0 || request.max_duration_ms == 0 {
        return Err(RunError::Lifecycle("invalid readiness request"));
    }
    validate_scalar("ISO8601", &request.deadline_utc)
        .map_err(|_| RunError::Lifecycle("readiness deadline must be ISO8601"))?;
    Ok(())
}

fn resource_from_handle(handle: &OwnedHandle) -> Result<ProviderResource, RunError> {
    if handle.state != "active"
        || handle.intent_id.is_empty()
        || handle.provider_identity.is_empty()
        || handle.stable_provider_key != stable_provider_key(&handle.intent_id)
    {
        return Err(RunError::Lifecycle("invalid environment owned handle"));
    }
    Ok(ProviderResource {
        stable_provider_key: handle.stable_provider_key.clone(),
        labels: BTreeMap::from([
            (RUN_ID_LABEL.to_owned(), handle.run_id.clone()),
            (PROFILE_ID_LABEL.to_owned(), handle.profile_id.clone()),
            (
                ENVIRONMENT_ID_LABEL.to_owned(),
                handle.environment_id.clone(),
            ),
        ]),
        provider_identity: handle.provider_identity.clone(),
    })
}

fn validate_receipt_resource(
    expected: &ProviderResource,
    actual: &ProviderResource,
) -> Result<(), RunError> {
    if expected != actual {
        return Err(RunError::Lifecycle(
            "provider receipt does not match the owned environment identity",
        ));
    }
    Ok(())
}

fn validate_provider_resource(
    request: &EnvironmentRequest,
    expected_key: &str,
    expected_labels: &BTreeMap<String, String>,
    resource: &ProviderResource,
) -> Result<(), RunError> {
    if resource.stable_provider_key != expected_key || resource.labels != *expected_labels {
        return Err(RunError::InvalidJournal(
            "provider resource ownership does not match intent",
        ));
    }
    if resource.provider_identity != request.provider_identity {
        return Err(RunError::InvalidJournal(
            "provider identity does not match intent",
        ));
    }
    Ok(())
}

fn ensure_before_deadline(clock: &impl Clock, deadline_utc: &str) -> Result<(), RunError> {
    let now_utc = clock.now_utc()?;
    if !compare_iso8601_timestamps(&now_utc, deadline_utc)
        .map_err(|_| RunError::InvalidJournal("clock and deadline must be ISO8601"))?
        .is_lt()
    {
        return Err(RunError::InvalidJournal("resource intent deadline expired"));
    }
    Ok(())
}

fn validate_handle_request(
    request: &EnvironmentRequest,
    handle: &OwnedHandle,
) -> Result<(), RunError> {
    if handle.intent_id != request.intent_id
        || handle.run_id != request.run_id
        || handle.profile_id != request.profile_id
        || handle.environment_id != request.environment_id
        || handle.generation != request.generation
        || handle.deadline_utc != request.deadline_utc
        || handle.stable_provider_key != stable_provider_key(&request.intent_id)
        || handle.provider_identity != request.provider_identity
        || handle.state != "active"
    {
        return Err(RunError::InvalidJournal(
            "durable handle does not match intent",
        ));
    }
    Ok(())
}

fn format_utc_time(elapsed: std::time::Duration) -> String {
    let mut timestamp = format_utc_seconds(elapsed.as_secs());
    if elapsed.subsec_nanos() != 0 {
        timestamp.pop();
        timestamp.push('.');
        timestamp.push_str(format!("{:09}", elapsed.subsec_nanos()).trim_end_matches('0'));
        timestamp.push('Z');
    }
    timestamp
}

fn format_utc_seconds(seconds: u64) -> String {
    let days = seconds / 86_400;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        day_seconds / 3_600,
        (day_seconds % 3_600) / 60,
        day_seconds % 60
    )
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let days = days_since_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    (year + if month <= 2 { 1 } else { 0 }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_preserves_canonical_fractional_precision() {
        for (nanos, expected) in [
            (0, "1970-01-01T00:00:00Z"),
            (1, "1970-01-01T00:00:00.000000001Z"),
            (50_000_000, "1970-01-01T00:00:00.05Z"),
            (500_000_000, "1970-01-01T00:00:00.5Z"),
            (999_999_999, "1970-01-01T00:00:00.999999999Z"),
        ] {
            let timestamp = format_utc_time(std::time::Duration::new(0, nanos));
            assert_eq!(timestamp, expected);
            validate_scalar("ISO8601", &timestamp).unwrap();
        }
        assert_eq!(
            format_utc_time(std::time::Duration::new(1, 0)),
            "1970-01-01T00:00:01Z"
        );
    }
}
