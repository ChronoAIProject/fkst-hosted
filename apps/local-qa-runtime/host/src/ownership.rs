use std::collections::BTreeMap;

use fkst_qa_contracts::validate_scalar;

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

pub trait EnvironmentProvider {
    fn discover(&mut self, stable_provider_key: &str) -> Result<Option<ProviderResource>, RunError>;
    fn create(&mut self, request: CreateRequest) -> Result<ProviderResource, RunError>;
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
    now_utc: &str,
) -> Result<OwnedHandle, RunError> {
    validate_scalar("ISO8601", now_utc)
        .map_err(|_| RunError::InvalidJournal("now_utc must be ISO8601"))?;
    let intent = journal.prepare_intent(
        &request.intent_id,
        &request.run_id,
        &request.profile_id,
        &request.environment_id,
        request.generation,
        &request.deadline_utc,
    )?;
    if let Some(handle) = journal.owned_handle(&request.intent_id)? {
        validate_handle(&intent, &handle)?;
        return Ok(handle);
    }
    if now_utc >= intent.deadline_utc.as_str() {
        return Err(RunError::InvalidJournal("resource intent deadline expired"));
    }

    let expected_key = stable_provider_key(&request.intent_id);
    let expected_labels = ownership_labels(request);
    let resource = match provider.discover(&expected_key)? {
        Some(resource) => resource,
        None => provider.create(CreateRequest {
            stable_provider_key: expected_key.clone(),
            labels: expected_labels.clone(),
        })?,
    };
    if resource.stable_provider_key != expected_key || resource.labels != expected_labels {
        return Err(RunError::InvalidJournal(
            "provider resource ownership does not match intent",
        ));
    }
    if resource.provider_identity != request.provider_identity
        || resource.provider_identity.is_empty()
    {
        return Err(RunError::InvalidJournal("provider identity does not match intent"));
    }

    journal.record_handle(&OwnedHandle {
        intent_id: request.intent_id.clone(),
        run_id: request.run_id.clone(),
        profile_id: request.profile_id.clone(),
        environment_id: request.environment_id.clone(),
        generation: request.generation,
        stable_provider_key: expected_key,
        provider_identity: resource.provider_identity,
        state: "active".to_owned(),
    })
}

fn validate_handle(
    intent: &crate::journal::ResourceIntent,
    handle: &OwnedHandle,
) -> Result<(), RunError> {
    if handle.intent_id != intent.intent_id
        || handle.run_id != intent.run_id
        || handle.profile_id != intent.profile_id
        || handle.environment_id != intent.environment_id
        || handle.generation != intent.generation
        || handle.stable_provider_key != intent.stable_provider_key
        || handle.state != "active"
    {
        return Err(RunError::InvalidJournal("durable handle does not match intent"));
    }
    Ok(())
}
