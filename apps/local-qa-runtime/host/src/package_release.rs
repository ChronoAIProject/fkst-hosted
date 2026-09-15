use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::journal::Journal;

pub const TESTING_PACKAGES_REPOSITORY: &str = "ChronoAIProject/fkst-packages-testing";

const SUBJECT_NAME: &str = "package-release/testing-package-release.v1.json";
const AUTHORIZATION_PATH: &str = "package-release/testing-package-release.v1.key.json";
const DSSE_PATH: &str = "package-release/testing-package-release.v1.dsse.json";
const TOOL_CATALOG_PATH: &str = "package-release/testing-package-tool-catalog.v1.json";
const PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";
const STATEMENT_TYPE: &str = "https://in-toto.io/Statement/v1";
const PREDICATE_TYPE: &str =
    "https://chronoaiproject.github.io/fkst-packages-testing/attestations/testing-package-release/v1";
const LEGACY_KEY_ID: &str = "fkst-packages-testing-release-v1-2026-09-04";
const AUTHORITY_ISSUER: &str = "https://releases.chronoaiproject.org/fkst-packages-testing";
const REVOCATION_AUTHORITY: &str =
    "https://releases.chronoaiproject.org/fkst-packages-testing/revocations/v1";
const SIGNATURE_PROFILE: &str = "dsse-ed25519.v1";
const RELEASE_SCHEMA: &str = "testing-package-release.v1";
const RELEASE_CANONICALIZATION: &str = "fkst-testing-package-release-canonical-json.v1";
const MANIFEST_SCHEMA: &str = "testing-package-manifest.v1";
const MANIFEST_CANONICALIZATION: &str = "fkst-testing-package-manifest-canonical-json.v1";
const BUNDLE_SCHEMA: &str = "testing-package-bundle.v1";
const SCHEMA_CATALOG_SCHEMA: &str = "testing-schema-catalog.v1";
const SCHEMA_CATALOG_CANONICALIZATION: &str = "fkst-testing-schema-catalog-canonical-json.v1";
const SCHEMA_RELEASE_SCHEMA: &str = "testing-package-schema-release.v1";
const SCHEMA_RELEASE_CANONICALIZATION: &str =
    "fkst-testing-package-schema-release-canonical-json.v1";
const PACKAGE_ID: &str = "testing-runner";
const PACKAGE_VERSION: &str = "1.0.0";
const SUPPORTED_PROFILE: &str = "browser-deterministic.v1";
const SUPPORTED_CAPABILITY: &str = "browser.read-title.v1";
const SUPPORTED_PLATFORM: &str = "linux-amd64";
const SUPPORTED_LUA: &str = "5.4.0";
const EXECUTOR_ID: &str = "testing-package-executor.browser-title.v1";
const EXECUTOR_MODULE: &str = "testing_package_executor.executor";
const EXECUTOR_FUNCTION: &str = "execute";
const ENTRYPOINT: &str = "testing-runner.run";
const CONTRACT_MAJOR: &str = "testing-runner.v1";
const REDUCER_SCHEMA: &str = "testing-assertion-reducer-identity.v1";
const REDUCER_ID: &str = "testing.assertion-reducer.browser-title-equals";
const REDUCER_VERSION: &str = "1.0.0";
const REDUCER_POLICY: &str = "browser-title-equals.v1";
const RESULT_CONTRACT_MAJOR: &str = "testing-case-result-set.v2";
const RESULT_AUTHORITY_RECEIPT_SCHEMA: &str = "testing-result-authority-receipt.v1";
const MAX_RELEASE_BYTES: usize = 256 * 1024;
const MAX_AUTHORIZATION_BYTES: usize = 16 * 1024;
const MAX_DSSE_BYTES: usize = 16 * 1024;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_BUNDLE_BYTES: usize = 1024 * 1024;
const MAX_SCHEMA_CATALOG_BYTES: usize = 256 * 1024;
const MAX_SCHEMA_RELEASE_BYTES: usize = 64 * 1024;
const MAX_TOOL_CATALOG_BYTES: usize = 64 * 1024;

const BUNDLE_PATHS: [&str; 10] = [
    "libraries/contract/canonical_json.lua",
    "libraries/contract/error_facts.lua",
    "libraries/contract/sha256.lua",
    "libraries/contract/strings.lua",
    "libraries/contract/testing_evidence_manifest.lua",
    "libraries/contract/testing_package_executor.lua",
    "libraries/contract/testing_result_authority.lua",
    "libraries/contract/testing_results.lua",
    "libraries/contract/time.lua",
    "libraries/testing_package_executor/executor.lua",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageReleaseError {
    MutableReleaseReference,
    InvalidReleaseReference(&'static str),
    InvalidAdmissionRequest(&'static str),
    FetchUnavailable(String),
    FetchLimitExceeded(String),
    VerificationFailed(&'static str),
    CacheCorrupt,
    AdmissionConflict,
    AntiRollback,
    Io(String),
    Journal(String),
}

impl fmt::Display for PackageReleaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MutableReleaseReference => {
                formatter.write_str("release ref must be an immutable commit")
            }
            Self::InvalidReleaseReference(detail) => {
                write!(formatter, "invalid release ref: {detail}")
            }
            Self::InvalidAdmissionRequest(detail) => {
                write!(formatter, "invalid admission request: {detail}")
            }
            Self::FetchUnavailable(path) => {
                write!(formatter, "release artifact is unavailable: {path}")
            }
            Self::FetchLimitExceeded(path) => {
                write!(formatter, "release artifact exceeds fetch limit: {path}")
            }
            Self::VerificationFailed(detail) => {
                write!(formatter, "release verification failed: {detail}")
            }
            Self::CacheCorrupt => formatter.write_str("content-addressed release cache is corrupt"),
            Self::AdmissionConflict => {
                formatter.write_str("package release admission key has a different digest")
            }
            Self::AntiRollback => {
                formatter.write_str("package release is below the selected anti-rollback sequence")
            }
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Journal(error) => write!(formatter, "journal error: {error}"),
        }
    }
}

impl From<std::io::Error> for PackageReleaseError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<crate::RunError> for PackageReleaseError {
    fn from(error: crate::RunError) -> Self {
        Self::Journal(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImmutablePackageReleaseRef {
    repository: String,
    commit_sha: String,
    path: String,
}

impl ImmutablePackageReleaseRef {
    pub fn new(
        repository: impl Into<String>,
        commit_sha: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<Self, PackageReleaseError> {
        let repository = repository.into();
        if repository != TESTING_PACKAGES_REPOSITORY {
            return Err(PackageReleaseError::InvalidReleaseReference(
                "repository is not the trusted Testing Packages repository",
            ));
        }
        let commit_sha = commit_sha.into();
        if !is_hex_len(&commit_sha, 40) {
            return Err(PackageReleaseError::MutableReleaseReference);
        }
        let path = path.into();
        if path != SUBJECT_NAME || !safe_path(&path) {
            return Err(PackageReleaseError::InvalidReleaseReference(
                "release path is not the signed package release descriptor",
            ));
        }
        Ok(Self {
            repository,
            commit_sha,
            path,
        })
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn commit_sha(&self) -> &str {
        &self.commit_sha
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageReleasePolicy {
    verification_time_utc: String,
    minimum_release_sequence: u64,
    supported_profile: String,
    platform: String,
    capability: String,
    revoked_keyids: Vec<String>,
}

impl PackageReleasePolicy {
    pub fn production(
        verification_time_utc: impl Into<String>,
    ) -> Result<Self, PackageReleaseError> {
        let verification_time_utc = verification_time_utc.into();
        if !valid_utc_timestamp(&verification_time_utc) {
            return Err(PackageReleaseError::InvalidAdmissionRequest(
                "verification time must be a canonical UTC timestamp",
            ));
        }
        Ok(Self {
            verification_time_utc,
            minimum_release_sequence: 1,
            supported_profile: SUPPORTED_PROFILE.to_owned(),
            platform: SUPPORTED_PLATFORM.to_owned(),
            capability: SUPPORTED_CAPABILITY.to_owned(),
            revoked_keyids: Vec::new(),
        })
    }

    pub fn with_minimum_release_sequence(mut self, minimum_release_sequence: u64) -> Self {
        self.minimum_release_sequence = minimum_release_sequence;
        self
    }

    pub fn with_revoked_keyids(mut self, revoked_keyids: Vec<String>) -> Self {
        self.revoked_keyids = revoked_keyids;
        self
    }

    fn receipt(&self, transition: PackageReleaseTransition) -> ReceiptPolicy {
        ReceiptPolicy {
            verification_time_utc: self.verification_time_utc.clone(),
            minimum_release_sequence: self.minimum_release_sequence,
            supported_profile: self.supported_profile.clone(),
            platform: self.platform.clone(),
            capability: self.capability.clone(),
            transition: transition.as_str().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageReleaseTransition {
    Initial,
    Update,
    ApprovedRollback,
}

impl PackageReleaseTransition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Update => "update",
            Self::ApprovedRollback => "approved-rollback",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageReleaseAdmissionRequest {
    idempotency_key: String,
    release_ref: ImmutablePackageReleaseRef,
    expected_release_sha256: String,
    trusted_authorization_sha256: String,
    policy: PackageReleasePolicy,
    transition: PackageReleaseTransition,
}

impl PackageReleaseAdmissionRequest {
    pub fn new(
        idempotency_key: impl Into<String>,
        release_ref: ImmutablePackageReleaseRef,
        expected_release_sha256: impl Into<String>,
        trusted_authorization_sha256: impl Into<String>,
        policy: PackageReleasePolicy,
    ) -> Result<Self, PackageReleaseError> {
        let idempotency_key = idempotency_key.into();
        if !valid_idempotency_key(&idempotency_key) {
            return Err(PackageReleaseError::InvalidAdmissionRequest(
                "idempotency key is invalid",
            ));
        }
        let expected_release_sha256 = expected_release_sha256.into();
        if !is_hex_len(&expected_release_sha256, 64) {
            return Err(PackageReleaseError::InvalidAdmissionRequest(
                "expected release digest must be lowercase SHA-256 hex",
            ));
        }
        let trusted_authorization_sha256 = trusted_authorization_sha256.into();
        if !is_hex_len(&trusted_authorization_sha256, 64) {
            return Err(PackageReleaseError::InvalidAdmissionRequest(
                "trusted authorization digest must be lowercase SHA-256 hex",
            ));
        }
        Ok(Self {
            idempotency_key,
            release_ref,
            expected_release_sha256,
            trusted_authorization_sha256,
            policy,
            transition: PackageReleaseTransition::Initial,
        })
    }

    pub fn with_update_transition(mut self) -> Self {
        self.transition = PackageReleaseTransition::Update;
        self
    }

    pub fn with_approved_rollback_transition(mut self) -> Self {
        self.transition = PackageReleaseTransition::ApprovedRollback;
        self
    }

    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    pub fn release_ref(&self) -> &ImmutablePackageReleaseRef {
        &self.release_ref
    }

    pub fn transition(&self) -> PackageReleaseTransition {
        self.transition
    }

    fn request_digest(&self) -> Result<String, PackageReleaseError> {
        #[derive(Serialize)]
        struct DigestMaterial<'a> {
            schema: &'static str,
            transition: &'static str,
            idempotency_key: &'a str,
            repository: &'a str,
            commit_sha: &'a str,
            release_path: &'a str,
            expected_release_sha256: &'a str,
            trusted_authorization_sha256: &'a str,
            verification_time_utc: &'a str,
            minimum_release_sequence: u64,
            supported_profile: &'a str,
            platform: &'a str,
            capability: &'a str,
        }

        let material = DigestMaterial {
            schema: "fkst-local-qa-package-release-admission-request.v1",
            transition: self.transition.as_str(),
            idempotency_key: &self.idempotency_key,
            repository: &self.release_ref.repository,
            commit_sha: &self.release_ref.commit_sha,
            release_path: &self.release_ref.path,
            expected_release_sha256: &self.expected_release_sha256,
            trusted_authorization_sha256: &self.trusted_authorization_sha256,
            verification_time_utc: &self.policy.verification_time_utc,
            minimum_release_sequence: self.policy.minimum_release_sequence,
            supported_profile: &self.policy.supported_profile,
            platform: &self.policy.platform,
            capability: &self.policy.capability,
        };
        let bytes = serde_json::to_vec(&material).map_err(|_| {
            PackageReleaseError::InvalidAdmissionRequest("request digest serialization failed")
        })?;
        Ok(sha256_hex(&bytes))
    }
}

pub trait PackageReleaseFetcher {
    fn fetch(
        &mut self,
        reference: &ImmutablePackageReleaseRef,
        path: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, PackageReleaseError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageReleaseAdmission {
    Created(PackageReleaseAdmissionReceipt),
    Replay(PackageReleaseAdmissionReceipt),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageReleaseAdmissionReceipt {
    pub schema: String,
    pub request_digest: String,
    pub release: ReceiptRelease,
    pub authority: ReceiptAuthority,
    pub manifest: ReceiptManifestArtifact,
    pub bundle: ReceiptArtifact,
    pub schema_catalog: ReceiptArtifact,
    pub schema_release: ReceiptArtifact,
    pub dependency_lock: ReceiptDependencyLock,
    pub package: ReceiptPackage,
    pub executor: ReceiptExecutor,
    pub reducer: ReceiptReducer,
    pub capability: String,
    pub policy: ReceiptPolicy,
    pub claim: ReceiptClaim,
    pub cache_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptRelease {
    pub repository: String,
    pub commit_sha: String,
    pub path: String,
    pub sha256: String,
    pub release_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptAuthority {
    pub keyid: String,
    pub authorization_sha256: String,
    pub dsse_sha256: String,
    pub payload_type: String,
    pub signature_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptArtifact {
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptManifestArtifact {
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub manifest_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptDependencyLock {
    pub repository_commit: String,
    pub fkst_packages_commit: String,
    pub fkst_substrate_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptPackage {
    pub package_id: String,
    pub package_version: String,
    pub package_content_sha256: String,
    pub supported_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptExecutor {
    pub executor_id: String,
    pub module: String,
    pub function: String,
    pub entrypoint: String,
    pub contract_major: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptReducer {
    pub reducer_id: String,
    pub reducer_version: String,
    pub reducer_sha256: String,
    pub policy_profile: String,
    pub supported_result_contract_majors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptPolicy {
    pub verification_time_utc: String,
    pub minimum_release_sequence: u64,
    pub supported_profile: String,
    pub platform: String,
    pub capability: String,
    pub transition: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptClaim {
    pub claim_id: String,
    pub claim_kind: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedPackageRelease {
    pub release_sha256: String,
    pub release_sequence: u64,
    pub executor: ReceiptExecutor,
    pub receipt: PackageReleaseAdmissionReceipt,
}

pub(crate) enum PackageReleaseAdmissionPreflight {
    Continue,
    Replay(Box<PackageReleaseAdmissionReceipt>),
    Conflict,
}

pub(crate) enum PackageReleaseJournalAdmission {
    Created(Box<PackageReleaseAdmissionReceipt>),
    Replay(Box<PackageReleaseAdmissionReceipt>),
    Conflict,
    AntiRollback,
    InvalidTransition,
}

pub(crate) enum PackageReleaseSelectionPreflight {
    Continue,
    AntiRollback,
    InvalidTransition,
}

struct VerifiedPackageRelease {
    receipt: PackageReleaseAdmissionReceipt,
    artifacts: VerifiedArtifacts,
}

struct VerifiedArtifacts {
    release_bytes: Vec<u8>,
    authorization_bytes: Vec<u8>,
    dsse_bytes: Vec<u8>,
    manifest_bytes: Vec<u8>,
    bundle_bytes: Vec<u8>,
    schema_catalog_bytes: Vec<u8>,
    schema_release_bytes: Vec<u8>,
    tool_catalog_bytes: Option<Vec<u8>>,
}

pub fn admit_package_release(
    journal: &mut Journal,
    fetcher: &mut dyn PackageReleaseFetcher,
    cache_root: &Path,
    request: &PackageReleaseAdmissionRequest,
) -> Result<PackageReleaseAdmission, PackageReleaseError> {
    let request_digest = request.request_digest()?;
    match journal.preflight_package_release_admission(request.idempotency_key(), &request_digest)? {
        PackageReleaseAdmissionPreflight::Continue => {}
        PackageReleaseAdmissionPreflight::Replay(receipt) => {
            return Ok(PackageReleaseAdmission::Replay(*receipt));
        }
        PackageReleaseAdmissionPreflight::Conflict => {
            return Err(PackageReleaseError::AdmissionConflict);
        }
    }

    let (verified, cache_adoption_needed) =
        match load_verified_cache(cache_root, request, &request_digest)? {
            Some(verified) => (verified, false),
            None => {
                let verified = fetch_and_verify(fetcher, request, &request_digest)?;
                (verified, true)
            }
        };

    match journal.preflight_package_release_selection(&verified.receipt, request.transition())? {
        PackageReleaseSelectionPreflight::Continue => {}
        PackageReleaseSelectionPreflight::AntiRollback => {
            return Err(PackageReleaseError::AntiRollback);
        }
        PackageReleaseSelectionPreflight::InvalidTransition => {
            return Err(PackageReleaseError::InvalidAdmissionRequest(
                "package release transition is not valid for the selected release",
            ));
        }
    }

    if cache_adoption_needed {
        adopt_cache(cache_root, &verified)?;
    }

    match journal.admit_package_release(
        request.idempotency_key(),
        &request_digest,
        &verified.receipt,
        request.transition(),
    )? {
        PackageReleaseJournalAdmission::Created(receipt) => {
            Ok(PackageReleaseAdmission::Created(*receipt))
        }
        PackageReleaseJournalAdmission::Replay(receipt) => {
            Ok(PackageReleaseAdmission::Replay(*receipt))
        }
        PackageReleaseJournalAdmission::Conflict => Err(PackageReleaseError::AdmissionConflict),
        PackageReleaseJournalAdmission::AntiRollback => Err(PackageReleaseError::AntiRollback),
        PackageReleaseJournalAdmission::InvalidTransition => {
            Err(PackageReleaseError::InvalidAdmissionRequest(
                "package release transition is not valid for the selected release",
            ))
        }
    }
}

fn fetch_and_verify(
    fetcher: &mut dyn PackageReleaseFetcher,
    request: &PackageReleaseAdmissionRequest,
    request_digest: &str,
) -> Result<VerifiedPackageRelease, PackageReleaseError> {
    let release_bytes = fetch_bounded(
        fetcher,
        request.release_ref(),
        request.release_ref().path(),
        MAX_RELEASE_BYTES,
    )?;
    if sha256_hex(&release_bytes) != request.expected_release_sha256 {
        return Err(PackageReleaseError::VerificationFailed(
            "release descriptor digest does not match expected pin",
        ));
    }

    let authorization_bytes = fetch_bounded(
        fetcher,
        request.release_ref(),
        AUTHORIZATION_PATH,
        MAX_AUTHORIZATION_BYTES,
    )?;
    if sha256_hex(&authorization_bytes) != request.trusted_authorization_sha256 {
        return Err(PackageReleaseError::VerificationFailed(
            "authorization digest does not match trust pin",
        ));
    }
    let dsse_bytes = fetch_bounded(fetcher, request.release_ref(), DSSE_PATH, MAX_DSSE_BYTES)?;

    let (release, authority, release_sequence) = verify_release_authority_and_signature(
        request,
        &release_bytes,
        &authorization_bytes,
        &dsse_bytes,
    )?;
    let manifest_bytes = fetch_expected_artifact(
        fetcher,
        request.release_ref(),
        &release.manifest.file,
        MAX_MANIFEST_BYTES,
    )?;
    let bundle_bytes = fetch_expected_artifact(
        fetcher,
        request.release_ref(),
        &release.bundle,
        MAX_BUNDLE_BYTES,
    )?;
    let schema_catalog_bytes = fetch_expected_artifact(
        fetcher,
        request.release_ref(),
        &release.schema_catalog,
        MAX_SCHEMA_CATALOG_BYTES,
    )?;
    let schema_release_bytes = fetch_expected_artifact(
        fetcher,
        request.release_ref(),
        &release.schema_release,
        MAX_SCHEMA_RELEASE_BYTES,
    )?;
    let tool_catalog_bytes = release
        .tool_catalog
        .as_ref()
        .map(|tool_catalog| {
            fetch_expected_artifact(
                fetcher,
                request.release_ref(),
                tool_catalog,
                MAX_TOOL_CATALOG_BYTES,
            )
        })
        .transpose()?;

    verify_manifest(&manifest_bytes, &release)?;
    verify_bundle(&bundle_bytes, &release)?;
    verify_schema_catalog(&schema_catalog_bytes)?;
    verify_schema_release(&schema_release_bytes, &release)?;
    if let Some(tool_catalog_bytes) = &tool_catalog_bytes {
        verify_tool_catalog(tool_catalog_bytes, &release)?;
    }

    Ok(VerifiedPackageRelease {
        receipt: build_receipt(
            request,
            request_digest,
            &release,
            &authority,
            release_sequence,
            &authorization_bytes,
            &dsse_bytes,
        ),
        artifacts: VerifiedArtifacts {
            release_bytes,
            authorization_bytes,
            dsse_bytes,
            manifest_bytes,
            bundle_bytes,
            schema_catalog_bytes,
            schema_release_bytes,
            tool_catalog_bytes,
        },
    })
}

fn load_verified_cache(
    cache_root: &Path,
    request: &PackageReleaseAdmissionRequest,
    request_digest: &str,
) -> Result<Option<VerifiedPackageRelease>, PackageReleaseError> {
    let directory = release_cache_dir(cache_root, &request.expected_release_sha256);
    if !directory.exists() {
        return Ok(None);
    }
    let artifacts = match read_cache_artifacts(&directory) {
        Ok(artifacts) => artifacts,
        Err(_) => return Err(PackageReleaseError::CacheCorrupt),
    };
    let verified = verify_cached_artifacts(request, request_digest, artifacts)
        .map_err(|_| PackageReleaseError::CacheCorrupt)?;
    Ok(Some(verified))
}

fn verify_cached_artifacts(
    request: &PackageReleaseAdmissionRequest,
    request_digest: &str,
    artifacts: VerifiedArtifacts,
) -> Result<VerifiedPackageRelease, PackageReleaseError> {
    if sha256_hex(&artifacts.release_bytes) != request.expected_release_sha256 {
        return Err(PackageReleaseError::VerificationFailed(
            "cached release digest mismatch",
        ));
    }
    if sha256_hex(&artifacts.authorization_bytes) != request.trusted_authorization_sha256 {
        return Err(PackageReleaseError::VerificationFailed(
            "cached authorization digest mismatch",
        ));
    }
    let (release, authority, release_sequence) = verify_release_authority_and_signature(
        request,
        &artifacts.release_bytes,
        &artifacts.authorization_bytes,
        &artifacts.dsse_bytes,
    )?;
    verify_artifact_bytes(&release.manifest.file, &artifacts.manifest_bytes)?;
    verify_artifact_bytes(&release.bundle, &artifacts.bundle_bytes)?;
    verify_artifact_bytes(&release.schema_catalog, &artifacts.schema_catalog_bytes)?;
    verify_artifact_bytes(&release.schema_release, &artifacts.schema_release_bytes)?;
    match (&release.tool_catalog, &artifacts.tool_catalog_bytes) {
        (Some(binding), Some(tool_catalog_bytes)) => {
            verify_artifact_bytes(binding, tool_catalog_bytes)?;
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(PackageReleaseError::VerificationFailed(
                "cached tool catalog does not match release binding",
            ))
        }
        (None, None) => {}
    }
    verify_manifest(&artifacts.manifest_bytes, &release)?;
    verify_bundle(&artifacts.bundle_bytes, &release)?;
    verify_schema_catalog(&artifacts.schema_catalog_bytes)?;
    verify_schema_release(&artifacts.schema_release_bytes, &release)?;
    if let Some(tool_catalog_bytes) = &artifacts.tool_catalog_bytes {
        verify_tool_catalog(tool_catalog_bytes, &release)?;
    }
    Ok(VerifiedPackageRelease {
        receipt: build_receipt(
            request,
            request_digest,
            &release,
            &authority,
            release_sequence,
            &artifacts.authorization_bytes,
            &artifacts.dsse_bytes,
        ),
        artifacts,
    })
}

fn fetch_bounded(
    fetcher: &mut dyn PackageReleaseFetcher,
    reference: &ImmutablePackageReleaseRef,
    path: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, PackageReleaseError> {
    if !safe_path(path) {
        return Err(PackageReleaseError::VerificationFailed(
            "artifact path is not confined",
        ));
    }
    let bytes = fetcher.fetch(reference, path, max_bytes)?;
    if bytes.len() > max_bytes {
        return Err(PackageReleaseError::FetchLimitExceeded(path.to_owned()));
    }
    Ok(bytes)
}

fn fetch_expected_artifact(
    fetcher: &mut dyn PackageReleaseFetcher,
    reference: &ImmutablePackageReleaseRef,
    binding: &FileBinding,
    max_bytes: usize,
) -> Result<Vec<u8>, PackageReleaseError> {
    let expected_size = usize::try_from(binding.size_bytes).map_err(|_| {
        PackageReleaseError::VerificationFailed("artifact size is not supported by this runtime")
    })?;
    if expected_size > max_bytes {
        return Err(PackageReleaseError::FetchLimitExceeded(
            binding.path.clone(),
        ));
    }
    let bytes = fetch_bounded(fetcher, reference, &binding.path, expected_size)?;
    verify_artifact_bytes(binding, &bytes)?;
    Ok(bytes)
}

fn verify_artifact_bytes(binding: &FileBinding, bytes: &[u8]) -> Result<(), PackageReleaseError> {
    if bytes.len() as u64 != binding.size_bytes || sha256_hex(bytes) != binding.sha256 {
        return Err(PackageReleaseError::VerificationFailed(
            "artifact bytes do not match release binding",
        ));
    }
    Ok(())
}

fn verify_release_authority_and_signature(
    request: &PackageReleaseAdmissionRequest,
    release_bytes: &[u8],
    authorization_bytes: &[u8],
    dsse_bytes: &[u8],
) -> Result<(ReleaseDescriptor, AuthorizationRecord, u64), PackageReleaseError> {
    let release_value = parse_canonical_value(release_bytes, true)?;
    let release: ReleaseDescriptor = serde_json::from_value(release_value.clone())
        .map_err(|_| PackageReleaseError::VerificationFailed("release profile is unsupported"))?;
    verify_release_shape(request, &release, &release_value)?;

    let release_sequence = verify_release_policy(request, &release)?;
    let authorization_value = parse_canonical_value(authorization_bytes, true)?;
    let authorization: AuthorizationRecord =
        serde_json::from_value(authorization_value).map_err(|_| {
            PackageReleaseError::VerificationFailed("authorization profile is unsupported")
        })?;
    verify_authorization_record(&authorization, release.expected_keyid())?;

    let public_key =
        decode_base64_exact(&authorization.public_key, 32, "authorization public key")?;
    let public_key = public_key.try_into().map_err(|_| {
        PackageReleaseError::VerificationFailed("authorization public key is unsupported")
    })?;
    verify_dsse_envelope(dsse_bytes, &authorization, &public_key, release_bytes)?;
    Ok((release, authorization, release_sequence))
}

fn verify_release_shape(
    request: &PackageReleaseAdmissionRequest,
    release: &ReleaseDescriptor,
    release_value: &Value,
) -> Result<(), PackageReleaseError> {
    if release.schema != RELEASE_SCHEMA || release.canonicalization != RELEASE_CANONICALIZATION {
        return Err(PackageReleaseError::VerificationFailed(
            "release schema is unsupported",
        ));
    }
    if release.authority.is_some() != release.tool_catalog.is_some() {
        return Err(PackageReleaseError::VerificationFailed(
            "release authority and tool catalog must be introduced together",
        ));
    }
    verify_file_binding(&release.bundle)?;
    verify_manifest_binding(&release.manifest)?;
    verify_file_binding(&release.schema_catalog)?;
    verify_file_binding(&release.schema_release)?;
    if let Some(tool_catalog) = &release.tool_catalog {
        verify_file_binding(tool_catalog)?;
        if tool_catalog.path != TOOL_CATALOG_PATH {
            return Err(PackageReleaseError::VerificationFailed(
                "release tool catalog path is unsupported",
            ));
        }
    }
    if release.package.package_id != PACKAGE_ID
        || release.package.package_version != PACKAGE_VERSION
        || release.package.supported_profile != request.policy.supported_profile
        || release.package.capability != request.policy.capability
        || !is_hex_len(&release.package.package_content_sha256, 64)
    {
        return Err(PackageReleaseError::VerificationFailed(
            "release package identity is unsupported",
        ));
    }
    if release.producer.name != "fkst-packages-testing"
        || release.producer.version != "1.0.0"
        || release.producer.generator != "scripts/generate_testing_package_release.py"
        || release.producer.generator_version != "1.0.0"
    {
        return Err(PackageReleaseError::VerificationFailed(
            "release producer identity is unsupported",
        ));
    }
    if release.runtime.lua != SUPPORTED_LUA || release.runtime.platform != request.policy.platform {
        return Err(PackageReleaseError::VerificationFailed(
            "release runtime identity is unsupported",
        ));
    }
    if release.executor.executor_id != EXECUTOR_ID
        || release.executor.module != EXECUTOR_MODULE
        || release.executor.function != EXECUTOR_FUNCTION
        || !safe_symbolic_module(&release.executor.module)
        || !metadata_string(&release.executor.function)
    {
        return Err(PackageReleaseError::VerificationFailed(
            "release executor identity is unsupported",
        ));
    }
    if release.mappings.len() != 1 {
        return Err(PackageReleaseError::VerificationFailed(
            "release must contain exactly one mapping",
        ));
    }
    let mapping = &release.mappings[0];
    if mapping.entrypoint != ENTRYPOINT
        || mapping.contract_major != CONTRACT_MAJOR
        || mapping.module != release.executor.module
        || mapping.function != release.executor.function
    {
        return Err(PackageReleaseError::VerificationFailed(
            "release mapping is unsupported",
        ));
    }
    if !is_hex_len(&release.source.repository_commit, 40)
        || !is_hex_len(&release.source.fkst_packages_commit, 40)
        || !is_hex_len(&release.source.fkst_substrate_commit, 40)
    {
        return Err(PackageReleaseError::VerificationFailed(
            "release source identities are unsupported",
        ));
    }
    verify_reducer(release, release_value)?;
    if release.result_authority.receipt_schema != RESULT_AUTHORITY_RECEIPT_SCHEMA {
        return Err(PackageReleaseError::VerificationFailed(
            "result authority receipt schema is unsupported",
        ));
    }
    if release.creation_metadata.build_id != "testing-package-release-walking-skeleton-v1"
        || !valid_utc_timestamp(&release.creation_metadata.created_at)
    {
        return Err(PackageReleaseError::VerificationFailed(
            "release creation metadata is unsupported",
        ));
    }
    Ok(())
}

fn verify_release_policy(
    request: &PackageReleaseAdmissionRequest,
    release: &ReleaseDescriptor,
) -> Result<u64, PackageReleaseError> {
    let release_sequence = match &release.authority {
        Some(authority) => {
            if authority.issuer != AUTHORITY_ISSUER
                || authority.signature_profile != SIGNATURE_PROFILE
                || authority.revocation_authority != REVOCATION_AUTHORITY
                || authority.release_sequence == 0
                || !valid_utc_timestamp(&authority.valid_from)
                || !valid_utc_timestamp(&authority.valid_until)
                || authority.valid_from >= authority.valid_until
                || request.policy.verification_time_utc < authority.valid_from
                || request.policy.verification_time_utc >= authority.valid_until
                || request
                    .policy
                    .revoked_keyids
                    .iter()
                    .any(|keyid| keyid == &authority.keyid)
            {
                return Err(PackageReleaseError::VerificationFailed(
                    "release authority policy is unsupported",
                ));
            }
            authority.release_sequence
        }
        None => 1,
    };
    if release_sequence < request.policy.minimum_release_sequence {
        return Err(PackageReleaseError::VerificationFailed(
            "release sequence is below the consumer minimum",
        ));
    }
    Ok(release_sequence)
}

fn verify_reducer(
    release: &ReleaseDescriptor,
    release_value: &Value,
) -> Result<(), PackageReleaseError> {
    if release.reducer.schema != REDUCER_SCHEMA
        || release.reducer.reducer_id != REDUCER_ID
        || release.reducer.reducer_version != REDUCER_VERSION
        || release.reducer.policy_profile != REDUCER_POLICY
        || release.reducer.supported_result_contract_majors != [RESULT_CONTRACT_MAJOR]
        || !is_hex_len(&release.reducer.reducer_sha256, 64)
    {
        return Err(PackageReleaseError::VerificationFailed(
            "release reducer identity is unsupported",
        ));
    }
    let mut reducer = release_value
        .get("reducer")
        .and_then(Value::as_object)
        .cloned()
        .ok_or(PackageReleaseError::VerificationFailed(
            "release reducer is not an object",
        ))?;
    reducer.remove("reducer_sha256");
    let reducer_without_digest = Value::Object(reducer);
    let canonical_reducer = canonical_json(&reducer_without_digest, false)?;
    let computed = sha256_hex(&canonical_reducer);
    if computed != release.reducer.reducer_sha256 {
        return Err(PackageReleaseError::VerificationFailed(
            "release reducer digest mismatch",
        ));
    }
    Ok(())
}

fn verify_authorization_record(
    authorization: &AuthorizationRecord,
    expected_keyid: &str,
) -> Result<(), PackageReleaseError> {
    if authorization.schema != "testing-package-release-key-authorization.v1"
        || authorization.algorithm != "ed25519"
        || authorization.keyid != expected_keyid
        || authorization.authorization.payload_type != PAYLOAD_TYPE
        || authorization.authorization.predicate_type != PREDICATE_TYPE
        || authorization.authorization.subject != SUBJECT_NAME
        || !metadata_string(&authorization.keyid)
    {
        return Err(PackageReleaseError::VerificationFailed(
            "authorization record profile is unsupported",
        ));
    }
    Ok(())
}

fn verify_dsse_envelope(
    dsse_bytes: &[u8],
    authorization: &AuthorizationRecord,
    public_key: &[u8; 32],
    release_bytes: &[u8],
) -> Result<(), PackageReleaseError> {
    let envelope_value = parse_canonical_value(dsse_bytes, true)?;
    let envelope: DsseEnvelope = serde_json::from_value(envelope_value)
        .map_err(|_| PackageReleaseError::VerificationFailed("DSSE envelope is unsupported"))?;
    if envelope.payload_type != PAYLOAD_TYPE || envelope.signatures.len() != 1 {
        return Err(PackageReleaseError::VerificationFailed(
            "DSSE envelope profile is unsupported",
        ));
    }
    let signed = &envelope.signatures[0];
    if signed.keyid != authorization.keyid {
        return Err(PackageReleaseError::VerificationFailed(
            "DSSE keyid is unsupported",
        ));
    }
    let payload = decode_base64(&envelope.payload, "DSSE payload")?;
    let signature = decode_base64_exact(&signed.sig, 64, "DSSE signature")?;
    let public_key = VerifyingKey::from_bytes(public_key).map_err(|_| {
        PackageReleaseError::VerificationFailed("authorization public key is unsupported")
    })?;
    let signature = Signature::try_from(signature.as_slice())
        .map_err(|_| PackageReleaseError::VerificationFailed("DSSE signature is unsupported"))?;
    public_key
        .verify(&dsse_pae(&payload), &signature)
        .map_err(|_| PackageReleaseError::VerificationFailed("Ed25519 DSSE verification failed"))?;

    let statement_value = parse_canonical_value(&payload, false)?;
    let statement: DsseStatement = serde_json::from_value(statement_value)
        .map_err(|_| PackageReleaseError::VerificationFailed("DSSE statement is unsupported"))?;
    if statement.statement_type != STATEMENT_TYPE
        || statement.predicate_type != PREDICATE_TYPE
        || statement.predicate != serde_json::Map::new()
        || statement.subject.len() != 1
    {
        return Err(PackageReleaseError::VerificationFailed(
            "DSSE statement profile is unsupported",
        ));
    }
    let subject = &statement.subject[0];
    if subject.name != SUBJECT_NAME || subject.digest.sha256 != sha256_hex(release_bytes) {
        return Err(PackageReleaseError::VerificationFailed(
            "DSSE release digest binding mismatch",
        ));
    }
    Ok(())
}

fn verify_manifest(
    manifest_bytes: &[u8],
    release: &ReleaseDescriptor,
) -> Result<(), PackageReleaseError> {
    let manifest_value = parse_canonical_value(manifest_bytes, false)?;
    let manifest: ManifestDescriptor = serde_json::from_value(manifest_value.clone())
        .map_err(|_| PackageReleaseError::VerificationFailed("manifest profile is unsupported"))?;
    if manifest.schema != MANIFEST_SCHEMA
        || manifest.canonicalization != MANIFEST_CANONICALIZATION
        || manifest.package_id != release.package.package_id
        || manifest.package_version != release.package.package_version
        || manifest.source_commit != release.source.repository_commit
        || manifest.package_content_sha256 != release.package.package_content_sha256
        || manifest.manifest_digest != release.manifest.manifest_digest
    {
        return Err(PackageReleaseError::VerificationFailed(
            "manifest release identity mismatch",
        ));
    }
    let mut manifest_without_digest =
        manifest_value
            .as_object()
            .cloned()
            .ok_or(PackageReleaseError::VerificationFailed(
                "manifest is not an object",
            ))?;
    manifest_without_digest.remove("manifest_digest");
    if sha256_hex(&canonical_json(
        &Value::Object(manifest_without_digest),
        false,
    )?) != manifest.manifest_digest
    {
        return Err(PackageReleaseError::VerificationFailed(
            "manifest canonical digest mismatch",
        ));
    }
    if manifest.supported_contracts.majors != [CONTRACT_MAJOR]
        || manifest.supported_contracts.canonicalization_profiles != [MANIFEST_CANONICALIZATION]
        || manifest.entrypoints.len() != 1
        || manifest.entrypoints[0].name != ENTRYPOINT
        || manifest.entrypoints[0].contract_major != CONTRACT_MAJOR
        || manifest.entrypoints[0].capabilities != [SUPPORTED_CAPABILITY]
        || manifest.semantic_capabilities != [SUPPORTED_CAPABILITY]
        || manifest.runtime_requirements.lua != SUPPORTED_LUA
        || manifest.runtime_requirements.platforms != [SUPPORTED_PLATFORM]
        || manifest.dependencies.fkst_packages.id != "fkst-packages"
        || manifest.dependencies.fkst_packages.commit != release.source.fkst_packages_commit
        || manifest.dependencies.fkst_substrate.id != "fkst-substrate"
        || manifest.dependencies.fkst_substrate.commit != release.source.fkst_substrate_commit
        || manifest.producer.name != "fkst-packages-testing"
        || manifest.producer.version != "1.0.0"
        || manifest.producer.toolchain != "testing-package-release-v1"
        || manifest.creation_metadata.build_id != "testing-package-release-walking-skeleton-v1"
        || !valid_utc_timestamp(&manifest.creation_metadata.created_at)
    {
        return Err(PackageReleaseError::VerificationFailed(
            "manifest capability or dependency lock is unsupported",
        ));
    }
    Ok(())
}

fn verify_bundle(
    bundle_bytes: &[u8],
    release: &ReleaseDescriptor,
) -> Result<(), PackageReleaseError> {
    let bundle_value = parse_canonical_value(bundle_bytes, true)?;
    let bundle: BundleDescriptor = serde_json::from_value(bundle_value)
        .map_err(|_| PackageReleaseError::VerificationFailed("bundle profile is unsupported"))?;
    if bundle.schema != BUNDLE_SCHEMA || bundle.files.is_empty() {
        return Err(PackageReleaseError::VerificationFailed(
            "bundle profile is unsupported",
        ));
    }

    let mut previous: Option<Vec<u8>> = None;
    let mut paths = Vec::with_capacity(bundle.files.len());
    let mut context = Sha256::new();
    for file in bundle.files {
        if !safe_path(&file.path) || !is_hex_len(&file.sha256, 64) {
            return Err(PackageReleaseError::VerificationFailed(
                "bundle file metadata is unsupported",
            ));
        }
        let path_bytes = file.path.as_bytes().to_vec();
        if previous
            .as_ref()
            .is_some_and(|previous| previous.as_slice() >= path_bytes.as_slice())
        {
            return Err(PackageReleaseError::VerificationFailed(
                "bundle file paths are not sorted and unique",
            ));
        }
        previous = Some(path_bytes);
        let content = decode_base64(&file.content_base64, "bundle file content")?;
        if content.len() as u64 != file.size_bytes || sha256_hex(&content) != file.sha256 {
            return Err(PackageReleaseError::VerificationFailed(
                "bundle file content binding mismatch",
            ));
        }
        context.update(file.path.as_bytes());
        context.update([0, 0x66]);
        context.update(&content);
        context.update([0]);
        paths.push(file.path);
    }
    if paths != BUNDLE_PATHS {
        return Err(PackageReleaseError::VerificationFailed(
            "bundle files do not match the runtime allowlist",
        ));
    }
    if hex_digest(context.finalize().as_ref()) != release.package.package_content_sha256 {
        return Err(PackageReleaseError::VerificationFailed(
            "bundle package content digest mismatch",
        ));
    }
    Ok(())
}

fn verify_schema_catalog(bytes: &[u8]) -> Result<(), PackageReleaseError> {
    let value = parse_canonical_value(bytes, true)?;
    let catalog: SchemaCatalog = serde_json::from_value(value)
        .map_err(|_| PackageReleaseError::VerificationFailed("schema catalog is unsupported"))?;
    if catalog.schema != SCHEMA_CATALOG_SCHEMA
        || catalog.canonicalization != SCHEMA_CATALOG_CANONICALIZATION
        || !is_hex_len(&catalog.catalog_sha256, 64)
    {
        return Err(PackageReleaseError::VerificationFailed(
            "schema catalog profile is unsupported",
        ));
    }
    let paths = catalog
        .schemas
        .iter()
        .map(|schema| schema.path.as_str())
        .collect::<BTreeSet<_>>();
    for required in [
        "schemas/testing-package-release.v1.schema.json",
        "schemas/testing-package-manifest.v1.schema.json",
        "schemas/testing-case-result-set.v2.schema.json",
    ] {
        if !paths.contains(required) {
            return Err(PackageReleaseError::VerificationFailed(
                "schema catalog is missing a required contract",
            ));
        }
    }
    for schema in catalog.schemas {
        if !safe_path(&schema.path)
            || !safe_path(&schema.fixture_set_path)
            || !is_hex_len(&schema.fixture_set_sha256, 64)
            || !is_hex_len(&schema.schema_sha256, 64)
            || schema.status != "stable"
            || schema.contract_major == 0
            || !metadata_string(&schema.schema_id)
            || !metadata_string(&schema.draft)
        {
            return Err(PackageReleaseError::VerificationFailed(
                "schema catalog entry is unsupported",
            ));
        }
        if let Some(profile) = schema.canonicalization_profile {
            if !metadata_string(&profile) {
                return Err(PackageReleaseError::VerificationFailed(
                    "schema catalog canonicalization profile is unsupported",
                ));
            }
        }
    }
    Ok(())
}

fn verify_schema_release(
    bytes: &[u8],
    release: &ReleaseDescriptor,
) -> Result<(), PackageReleaseError> {
    let value = parse_canonical_value(bytes, true)?;
    let schema_release: SchemaRelease = serde_json::from_value(value)
        .map_err(|_| PackageReleaseError::VerificationFailed("schema release is unsupported"))?;
    if schema_release.schema != SCHEMA_RELEASE_SCHEMA
        || schema_release.canonicalization != SCHEMA_RELEASE_CANONICALIZATION
        || schema_release.producer.name != "fkst-packages-testing"
        || schema_release.producer.version != "1.0.0"
        || schema_release.schema_catalog.kind != "testing-schema-catalog"
        || !schema_release
            .schema_catalog
            .ref_
            .starts_with("immutable://")
        || schema_release.schema_catalog.sha256 != release.schema_catalog.sha256
        || schema_release.package_manifest.kind != "testing-package-manifest"
        || !schema_release
            .package_manifest
            .ref_
            .starts_with("immutable://")
        || !is_hex_len(&schema_release.package_manifest.sha256, 64)
        || !is_hex_len(&schema_release.release_sha256, 64)
    {
        return Err(PackageReleaseError::VerificationFailed(
            "schema release profile is unsupported",
        ));
    }
    Ok(())
}

fn verify_tool_catalog(
    bytes: &[u8],
    release: &ReleaseDescriptor,
) -> Result<(), PackageReleaseError> {
    let value = parse_canonical_value(bytes, true)?;
    let catalog: ToolCatalog = serde_json::from_value(value)
        .map_err(|_| PackageReleaseError::VerificationFailed("tool catalog is unsupported"))?;
    if catalog.schema != "testing-package-tool-catalog.v1"
        || catalog.canonicalization != "fkst-testing-package-tool-catalog-canonical-json.v1"
        || catalog.execution_profile != release.package.supported_profile
        || catalog.tools.len() != 1
        || catalog.tools[0].capability != release.package.capability
        || !metadata_string(&catalog.tools[0].port)
    {
        return Err(PackageReleaseError::VerificationFailed(
            "tool catalog profile is unsupported",
        ));
    }
    Ok(())
}

fn build_receipt(
    request: &PackageReleaseAdmissionRequest,
    request_digest: &str,
    release: &ReleaseDescriptor,
    authority: &AuthorizationRecord,
    release_sequence: u64,
    authorization_bytes: &[u8],
    dsse_bytes: &[u8],
) -> PackageReleaseAdmissionReceipt {
    let release_sha256 = request.expected_release_sha256.clone();
    PackageReleaseAdmissionReceipt {
        schema: "fkst-local-qa-package-release-admission.v1".to_owned(),
        request_digest: request_digest.to_owned(),
        release: ReceiptRelease {
            repository: request.release_ref.repository.clone(),
            commit_sha: request.release_ref.commit_sha.clone(),
            path: request.release_ref.path.clone(),
            sha256: release_sha256.clone(),
            release_sequence,
        },
        authority: ReceiptAuthority {
            keyid: authority.keyid.clone(),
            authorization_sha256: sha256_hex(authorization_bytes),
            dsse_sha256: sha256_hex(dsse_bytes),
            payload_type: PAYLOAD_TYPE.to_owned(),
            signature_profile: SIGNATURE_PROFILE.to_owned(),
        },
        manifest: ReceiptManifestArtifact {
            path: release.manifest.file.path.clone(),
            sha256: release.manifest.file.sha256.clone(),
            size_bytes: release.manifest.file.size_bytes,
            manifest_digest: release.manifest.manifest_digest.clone(),
        },
        bundle: release.bundle.receipt(),
        schema_catalog: release.schema_catalog.receipt(),
        schema_release: release.schema_release.receipt(),
        dependency_lock: ReceiptDependencyLock {
            repository_commit: release.source.repository_commit.clone(),
            fkst_packages_commit: release.source.fkst_packages_commit.clone(),
            fkst_substrate_commit: release.source.fkst_substrate_commit.clone(),
        },
        package: ReceiptPackage {
            package_id: release.package.package_id.clone(),
            package_version: release.package.package_version.clone(),
            package_content_sha256: release.package.package_content_sha256.clone(),
            supported_profile: release.package.supported_profile.clone(),
        },
        executor: ReceiptExecutor {
            executor_id: release.executor.executor_id.clone(),
            module: release.executor.module.clone(),
            function: release.executor.function.clone(),
            entrypoint: release.mappings[0].entrypoint.clone(),
            contract_major: release.mappings[0].contract_major.clone(),
        },
        reducer: ReceiptReducer {
            reducer_id: release.reducer.reducer_id.clone(),
            reducer_version: release.reducer.reducer_version.clone(),
            reducer_sha256: release.reducer.reducer_sha256.clone(),
            policy_profile: release.reducer.policy_profile.clone(),
            supported_result_contract_majors: release
                .reducer
                .supported_result_contract_majors
                .clone(),
        },
        capability: release.package.capability.clone(),
        policy: request.policy.receipt(request.transition),
        claim: ReceiptClaim {
            claim_id: format!("fkst-local-qa/package-release/v1/{release_sha256}"),
            claim_kind: "runtime-owned-signed-release".to_owned(),
            status: "admitted".to_owned(),
        },
        cache_key: release_sha256,
    }
}

fn read_cache_artifacts(directory: &Path) -> Result<VerifiedArtifacts, std::io::Error> {
    Ok(VerifiedArtifacts {
        release_bytes: fs::read(directory.join("release.json"))?,
        authorization_bytes: fs::read(directory.join("authorization.json"))?,
        dsse_bytes: fs::read(directory.join("dsse.json"))?,
        manifest_bytes: fs::read(directory.join("manifest.json"))?,
        bundle_bytes: fs::read(directory.join("bundle.json"))?,
        schema_catalog_bytes: fs::read(directory.join("schema-catalog.json"))?,
        schema_release_bytes: fs::read(directory.join("schema-release.json"))?,
        tool_catalog_bytes: match fs::read(directory.join("tool-catalog.json")) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        },
    })
}

fn adopt_cache(
    cache_root: &Path,
    verified: &VerifiedPackageRelease,
) -> Result<(), PackageReleaseError> {
    let final_directory = release_cache_dir(cache_root, &verified.receipt.release.sha256);
    if final_directory.exists() {
        return Ok(());
    }
    let parent = final_directory.parent().ok_or(PackageReleaseError::Io(
        "cache directory has no parent".to_owned(),
    ))?;
    fs::create_dir_all(parent)?;
    let temp_directory = parent.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| PackageReleaseError::Io(error.to_string()))?
            .as_nanos()
    ));
    fs::create_dir(&temp_directory)?;
    let result = write_cache_artifacts(&temp_directory, &verified.artifacts)
        .and_then(|_| fs::rename(&temp_directory, &final_directory));
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temp_directory);
        return Err(PackageReleaseError::Io(error.to_string()));
    }
    Ok(())
}

fn write_cache_artifacts(
    directory: &Path,
    artifacts: &VerifiedArtifacts,
) -> Result<(), std::io::Error> {
    write_new_file(&directory.join("release.json"), &artifacts.release_bytes)?;
    write_new_file(
        &directory.join("authorization.json"),
        &artifacts.authorization_bytes,
    )?;
    write_new_file(&directory.join("dsse.json"), &artifacts.dsse_bytes)?;
    write_new_file(&directory.join("manifest.json"), &artifacts.manifest_bytes)?;
    write_new_file(&directory.join("bundle.json"), &artifacts.bundle_bytes)?;
    write_new_file(
        &directory.join("schema-catalog.json"),
        &artifacts.schema_catalog_bytes,
    )?;
    write_new_file(
        &directory.join("schema-release.json"),
        &artifacts.schema_release_bytes,
    )?;
    if let Some(tool_catalog_bytes) = &artifacts.tool_catalog_bytes {
        write_new_file(&directory.join("tool-catalog.json"), tool_catalog_bytes)?;
    }
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn release_cache_dir(cache_root: &Path, release_sha256: &str) -> PathBuf {
    cache_root
        .join("package-release")
        .join("sha256")
        .join(release_sha256)
}

fn parse_canonical_value(bytes: &[u8], trailing_lf: bool) -> Result<Value, PackageReleaseError> {
    let value = serde_json::from_slice::<Value>(bytes)
        .map_err(|_| PackageReleaseError::VerificationFailed("artifact is not valid JSON"))?;
    let expected = canonical_json(&value, trailing_lf)?;
    if bytes != expected {
        return Err(PackageReleaseError::VerificationFailed(
            "artifact bytes are not canonical",
        ));
    }
    Ok(value)
}

fn canonical_json(value: &Value, trailing_lf: bool) -> Result<Vec<u8>, PackageReleaseError> {
    let sorted = sorted_value(value);
    let mut bytes = serde_json::to_vec(&sorted)
        .map_err(|_| PackageReleaseError::VerificationFailed("artifact canonicalization failed"))?;
    if trailing_lf {
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn sorted_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(sorted_value).collect()),
        Value::Object(object) => {
            let sorted = object
                .iter()
                .map(|(key, value)| (key.clone(), sorted_value(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        scalar => scalar.clone(),
    }
}

fn verify_file_binding(binding: &FileBinding) -> Result<(), PackageReleaseError> {
    if !safe_path(&binding.path) || !is_hex_len(&binding.sha256, 64) || binding.size_bytes == 0 {
        return Err(PackageReleaseError::VerificationFailed(
            "release file binding is unsupported",
        ));
    }
    Ok(())
}

fn verify_manifest_binding(binding: &ManifestBinding) -> Result<(), PackageReleaseError> {
    verify_file_binding(&binding.file)?;
    if !is_hex_len(&binding.manifest_digest, 64) {
        return Err(PackageReleaseError::VerificationFailed(
            "release manifest digest is unsupported",
        ));
    }
    Ok(())
}

fn decode_base64(value: &str, field: &'static str) -> Result<Vec<u8>, PackageReleaseError> {
    let bytes = STANDARD
        .decode(value)
        .map_err(|_| PackageReleaseError::VerificationFailed(field))?;
    if STANDARD.encode(&bytes) != value {
        return Err(PackageReleaseError::VerificationFailed(field));
    }
    Ok(bytes)
}

fn decode_base64_exact(
    value: &str,
    expected_length: usize,
    field: &'static str,
) -> Result<Vec<u8>, PackageReleaseError> {
    let bytes = decode_base64(value, field)?;
    if bytes.len() != expected_length {
        return Err(PackageReleaseError::VerificationFailed(field));
    }
    Ok(bytes)
}

fn dsse_pae(payload: &[u8]) -> Vec<u8> {
    let payload_type = PAYLOAD_TYPE.as_bytes();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"DSSEv1 ");
    bytes.extend_from_slice(payload_type.len().to_string().as_bytes());
    bytes.push(b' ');
    bytes.extend_from_slice(payload_type);
    bytes.push(b' ');
    bytes.extend_from_slice(payload.len().to_string().as_bytes());
    bytes.push(b' ');
    bytes.extend_from_slice(payload);
    bytes
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_digest(digest.as_ref())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_hex_len(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_idempotency_key(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn safe_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.bytes().any(|byte| byte <= 0x1f || byte == 0x7f)
        && value
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn safe_symbolic_module(value: &str) -> bool {
    metadata_string(value)
        && !value.contains('/')
        && !value.contains('\\')
        && value.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

fn metadata_string(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 180
        && !value.bytes().any(|byte| byte <= 0x1f || byte == 0x7f)
}

fn valid_utc_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z'
        && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
            .into_iter()
            .all(|index| bytes[index].is_ascii_digit())
        && (&bytes[5..7] >= b"01".as_slice() && &bytes[5..7] <= b"12".as_slice())
        && (&bytes[8..10] >= b"01".as_slice() && &bytes[8..10] <= b"31".as_slice())
        && (&bytes[11..13] <= b"23".as_slice())
        && (&bytes[14..16] <= b"59".as_slice())
        && (&bytes[17..19] <= b"59".as_slice())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseDescriptor {
    bundle: FileBinding,
    canonicalization: String,
    creation_metadata: CreationMetadata,
    executor: ExecutorIdentity,
    manifest: ManifestBinding,
    mappings: Vec<Mapping>,
    package: ReleasePackage,
    producer: ReleaseProducer,
    reducer: ReleaseReducer,
    result_authority: ResultAuthority,
    runtime: RuntimeIdentity,
    schema: String,
    schema_catalog: FileBinding,
    schema_release: FileBinding,
    source: ReleaseSource,
    authority: Option<ReleaseAuthority>,
    tool_catalog: Option<FileBinding>,
}

impl ReleaseDescriptor {
    fn expected_keyid(&self) -> &str {
        self.authority
            .as_ref()
            .map_or(LEGACY_KEY_ID, |authority| authority.keyid.as_str())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileBinding {
    path: String,
    sha256: String,
    size_bytes: u64,
}

impl FileBinding {
    fn receipt(&self) -> ReceiptArtifact {
        ReceiptArtifact {
            path: self.path.clone(),
            sha256: self.sha256.clone(),
            size_bytes: self.size_bytes,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestBinding {
    #[serde(flatten)]
    file: FileBinding,
    manifest_digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreationMetadata {
    build_id: String,
    created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutorIdentity {
    executor_id: String,
    module: String,
    function: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    contract_major: String,
    entrypoint: String,
    module: String,
    function: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleasePackage {
    capability: String,
    package_content_sha256: String,
    package_id: String,
    package_version: String,
    supported_profile: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseProducer {
    generator: String,
    generator_version: String,
    name: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseReducer {
    policy_profile: String,
    reducer_id: String,
    reducer_sha256: String,
    reducer_version: String,
    schema: String,
    supported_result_contract_majors: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultAuthority {
    receipt_schema: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeIdentity {
    lua: String,
    platform: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseSource {
    fkst_packages_commit: String,
    fkst_substrate_commit: String,
    repository_commit: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseAuthority {
    issuer: String,
    keyid: String,
    release_sequence: u64,
    revocation_authority: String,
    signature_profile: String,
    valid_from: String,
    valid_until: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorizationRecord {
    algorithm: String,
    authorization: AuthorizationScope,
    keyid: String,
    #[serde(rename = "publicKey")]
    public_key: String,
    schema: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorizationScope {
    #[serde(rename = "payloadType")]
    payload_type: String,
    #[serde(rename = "predicateType")]
    predicate_type: String,
    subject: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DsseEnvelope {
    payload: String,
    #[serde(rename = "payloadType")]
    payload_type: String,
    signatures: Vec<DsseSignature>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DsseSignature {
    keyid: String,
    sig: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DsseStatement {
    #[serde(rename = "_type")]
    statement_type: String,
    predicate: serde_json::Map<String, Value>,
    #[serde(rename = "predicateType")]
    predicate_type: String,
    subject: Vec<DsseSubject>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DsseSubject {
    digest: DsseSubjectDigest,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DsseSubjectDigest {
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDescriptor {
    canonicalization: String,
    creation_metadata: CreationMetadata,
    dependencies: ManifestDependencies,
    entrypoints: Vec<ManifestEntrypoint>,
    manifest_digest: String,
    package_content_sha256: String,
    package_id: String,
    package_version: String,
    producer: ManifestProducer,
    runtime_requirements: RuntimeRequirements,
    schema: String,
    semantic_capabilities: Vec<String>,
    source_commit: String,
    supported_contracts: SupportedContracts,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDependencies {
    fkst_packages: DependencyPin,
    fkst_substrate: DependencyPin,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DependencyPin {
    commit: String,
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestEntrypoint {
    capabilities: Vec<String>,
    contract_major: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestProducer {
    name: String,
    toolchain: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeRequirements {
    lua: String,
    platforms: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportedContracts {
    canonicalization_profiles: Vec<String>,
    majors: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleDescriptor {
    files: Vec<BundleFile>,
    schema: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleFile {
    content_base64: String,
    path: String,
    sha256: String,
    size_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaCatalog {
    canonicalization: String,
    catalog_sha256: String,
    schema: String,
    schemas: Vec<SchemaCatalogEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaCatalogEntry {
    canonicalization_profile: Option<String>,
    contract_major: u64,
    draft: String,
    fixture_set_path: String,
    fixture_set_sha256: String,
    path: String,
    schema_id: String,
    schema_sha256: String,
    status: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaRelease {
    canonicalization: String,
    package_manifest: SchemaReleaseRef,
    producer: SchemaReleaseProducer,
    release_sha256: String,
    schema: String,
    schema_catalog: SchemaReleaseRef,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaReleaseRef {
    kind: String,
    #[serde(rename = "ref")]
    ref_: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaReleaseProducer {
    name: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolCatalog {
    canonicalization: String,
    execution_profile: String,
    schema: String,
    tools: Vec<ToolCatalogEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolCatalogEntry {
    capability: String,
    port: String,
}
