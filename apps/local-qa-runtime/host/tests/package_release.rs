use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use fkst_local_qa_host::package_release::{
    admit_package_release, ImmutablePackageReleaseRef, PackageReleaseAdmission,
    PackageReleaseAdmissionRequest, PackageReleaseError, PackageReleaseFetcher,
    PackageReleasePolicy, TESTING_PACKAGES_REPOSITORY,
};
use fkst_local_qa_host::Journal;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const COMMIT_SHA: &str = "1111111111111111111111111111111111111111";
const SOURCE_COMMIT: &str = "2222222222222222222222222222222222222222";
const FKST_PACKAGES_COMMIT: &str = "3333333333333333333333333333333333333333";
const FKST_SUBSTRATE_COMMIT: &str = "4444444444444444444444444444444444444444";
const RELEASE_PATH: &str = "package-release/testing-package-release.v1.json";
const AUTHORIZATION_PATH: &str = "package-release/testing-package-release.v1.key.json";
const DSSE_PATH: &str = "package-release/testing-package-release.v1.dsse.json";
const TOOL_CATALOG_PATH: &str = "package-release/testing-package-tool-catalog.v1.json";
const UPDATE_COMMIT_SHA: &str = "5555555555555555555555555555555555555555";

struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    fn new(label: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "fkst-local-qa-package-release-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("temporary directory must be created");
        Self { path }
    }

    fn database_path(&self) -> PathBuf {
        self.path.join("journal.sqlite")
    }

    fn cache_root(&self) -> PathBuf {
        self.path.join("cache")
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).expect("temporary directory must be removed");
    }
}

#[derive(Clone)]
struct Fixture {
    release_commit_sha: String,
    expected_release_sha256: String,
    trusted_authorization_sha256: String,
    files: BTreeMap<String, Vec<u8>>,
}

struct MemoryFetcher {
    expected_commit_sha: String,
    files: BTreeMap<String, Vec<u8>>,
    calls: Vec<String>,
}

impl MemoryFetcher {
    fn new(fixture: &Fixture) -> Self {
        Self {
            expected_commit_sha: fixture.release_commit_sha.clone(),
            files: fixture.files.clone(),
            calls: Vec::new(),
        }
    }

    fn empty() -> Self {
        Self {
            expected_commit_sha: COMMIT_SHA.to_owned(),
            files: BTreeMap::new(),
            calls: Vec::new(),
        }
    }
}

impl PackageReleaseFetcher for MemoryFetcher {
    fn fetch(
        &mut self,
        reference: &ImmutablePackageReleaseRef,
        path: &str,
        _max_bytes: usize,
    ) -> Result<Vec<u8>, PackageReleaseError> {
        assert_eq!(reference.repository(), TESTING_PACKAGES_REPOSITORY);
        assert_eq!(reference.commit_sha(), self.expected_commit_sha.as_str());
        self.calls.push(path.to_owned());
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| PackageReleaseError::FetchUnavailable(path.to_owned()))
    }
}

#[test]
fn fixed_signed_release_is_verified_cached_and_atomically_admitted() {
    let fixture = signed_fixture();
    let temp = TempDirectory::new("admit");
    let mut journal = Journal::open(&temp.database_path()).expect("journal must open");
    let mut fetcher = MemoryFetcher::new(&fixture);
    let request = admission_request("release-admit-001", &fixture);

    let first = admit_package_release(&mut journal, &mut fetcher, &temp.cache_root(), &request)
        .expect("signed release must be admitted");
    let PackageReleaseAdmission::Created(receipt) = first else {
        panic!("first admission must create a durable receipt");
    };
    assert_eq!(receipt.schema, "fkst-local-qa-package-release-admission.v1");
    assert_eq!(receipt.release.sha256, fixture.expected_release_sha256);
    assert_eq!(receipt.release.repository, TESTING_PACKAGES_REPOSITORY);
    assert_eq!(receipt.release.commit_sha, COMMIT_SHA);
    assert_eq!(
        receipt.manifest.sha256,
        sha256_hex(&fixture.files["package-release/testing-package-manifest.v1.json"])
    );
    assert_eq!(
        receipt.schema_catalog.sha256,
        sha256_hex(&fixture.files["schema-release/testing-schema-catalog.v1.json"])
    );
    assert_eq!(
        receipt.dependency_lock.fkst_packages_commit,
        FKST_PACKAGES_COMMIT
    );
    assert_eq!(
        receipt.dependency_lock.fkst_substrate_commit,
        FKST_SUBSTRATE_COMMIT
    );
    assert_eq!(receipt.package.package_id, "testing-runner");
    assert_eq!(
        receipt.executor.executor_id,
        "testing-package-executor.browser-title.v1"
    );
    assert_eq!(receipt.executor.entrypoint, "testing-runner.run");
    assert_eq!(
        receipt.reducer.reducer_id,
        "testing.assertion-reducer.browser-title-equals"
    );
    assert_eq!(receipt.capability, "browser.read-title.v1");
    assert_eq!(receipt.policy.minimum_release_sequence, 1);
    assert_eq!(
        receipt.claim.claim_id,
        format!(
            "fkst-local-qa/package-release/v1/{}",
            receipt.release.sha256
        )
    );
    assert_eq!(receipt.request_digest.len(), 64);
    assert_eq!(fetcher.calls[0], RELEASE_PATH);
    assert!(cache_dir(&temp.cache_root(), &receipt.release.sha256)
        .join("release.json")
        .is_file());
    let selected = journal
        .selected_package_release()
        .expect("selected release query must succeed")
        .expect("admitted release must be selected");
    assert_eq!(selected.release_sha256, receipt.release.sha256);
    assert_eq!(selected.release_sequence, 1);
    assert_eq!(selected.executor, receipt.executor);
    assert_eq!(selected.receipt, receipt);

    drop(journal);
    let mut restarted = Journal::open(&temp.database_path()).expect("journal must reopen");
    let mut replay_fetcher = MemoryFetcher::empty();
    let replay = admit_package_release(
        &mut restarted,
        &mut replay_fetcher,
        &temp.cache_root(),
        &request,
    )
    .expect("restart replay must return the durable receipt");
    assert_eq!(replay, PackageReleaseAdmission::Replay(receipt));
    assert!(
        replay_fetcher.calls.is_empty(),
        "replay must not refetch or rerun verification"
    );
}

#[test]
fn same_key_different_digest_conflicts_before_fetch_or_cache_mutation() {
    let fixture = signed_fixture();
    let temp = TempDirectory::new("conflict");
    let mut journal = Journal::open(&temp.database_path()).expect("journal must open");
    let mut fetcher = MemoryFetcher::new(&fixture);
    let first_request = admission_request("release-admit-001", &fixture);
    admit_package_release(
        &mut journal,
        &mut fetcher,
        &temp.cache_root(),
        &first_request,
    )
    .expect("first admission must succeed");

    let before_cache_entries = cache_entry_count(&temp.cache_root());
    let mut conflict_fixture = fixture.clone();
    conflict_fixture.expected_release_sha256 =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned();
    let conflict_request = admission_request("release-admit-001", &conflict_fixture);
    let mut conflict_fetcher = MemoryFetcher::new(&fixture);

    let error = admit_package_release(
        &mut journal,
        &mut conflict_fetcher,
        &temp.cache_root(),
        &conflict_request,
    )
    .expect_err("same admission key with a different digest must conflict");
    assert_eq!(error, PackageReleaseError::AdmissionConflict);
    assert!(
        conflict_fetcher.calls.is_empty(),
        "conflict must be detected before fetch"
    );
    assert_eq!(cache_entry_count(&temp.cache_root()), before_cache_entries);
    assert_eq!(package_release_rows(&temp.database_path()), 1);
}

#[test]
fn corrupt_content_addressed_cache_fails_closed_without_journal_mutation() {
    let fixture = signed_fixture();
    let temp = TempDirectory::new("cache-corrupt");
    let cache_dir = cache_dir(&temp.cache_root(), &fixture.expected_release_sha256);
    fs::create_dir_all(&cache_dir).expect("cache directory must be created");
    fs::write(cache_dir.join("release.json"), b"corrupt\n")
        .expect("corrupt cache file must be written");

    let mut journal = Journal::open(&temp.database_path()).expect("journal must open");
    let mut fetcher = MemoryFetcher::new(&fixture);
    let error = admit_package_release(
        &mut journal,
        &mut fetcher,
        &temp.cache_root(),
        &admission_request("release-admit-001", &fixture),
    )
    .expect_err("corrupt cache must fail closed");
    assert_eq!(error, PackageReleaseError::CacheCorrupt);
    assert!(
        fetcher.calls.is_empty(),
        "existing corrupt cache must not be overwritten by fetch"
    );
    assert_eq!(package_release_rows(&temp.database_path()), 0);
}

#[test]
fn floating_release_refs_are_rejected_before_fetch() {
    let error = ImmutablePackageReleaseRef::new(TESTING_PACKAGES_REPOSITORY, "dev", RELEASE_PATH)
        .expect_err("floating ref must be rejected");
    assert_eq!(error, PackageReleaseError::MutableReleaseReference);
}

#[test]
fn authority_outage_fails_before_cache_or_journal_mutation() {
    let mut fixture = signed_fixture();
    fixture.files.remove(AUTHORIZATION_PATH);
    let temp = TempDirectory::new("authority-outage");
    let mut journal = Journal::open(&temp.database_path()).expect("journal must open");
    let mut fetcher = MemoryFetcher::new(&fixture);
    let error = admit_package_release(
        &mut journal,
        &mut fetcher,
        &temp.cache_root(),
        &admission_request("release-admit-001", &fixture),
    )
    .expect_err("missing authorization must fail");
    assert_eq!(
        error,
        PackageReleaseError::FetchUnavailable(AUTHORIZATION_PATH.to_owned())
    );
    assert!(!temp.cache_root().exists());
    assert_eq!(package_release_rows(&temp.database_path()), 0);
}

#[test]
fn tampered_dsse_signature_fails_before_cache_or_journal_mutation() {
    let mut fixture = signed_fixture();
    let mut envelope: Value =
        serde_json::from_slice(&fixture.files[DSSE_PATH]).expect("fixture DSSE must parse");
    envelope["signatures"][0]["sig"] = Value::String("A".repeat(86) + "==");
    fixture
        .files
        .insert(DSSE_PATH.to_owned(), canonical(&envelope, true));
    let temp = TempDirectory::new("tampered-dsse");
    let mut journal = Journal::open(&temp.database_path()).expect("journal must open");
    let mut fetcher = MemoryFetcher::new(&fixture);
    let error = admit_package_release(
        &mut journal,
        &mut fetcher,
        &temp.cache_root(),
        &admission_request("release-admit-001", &fixture),
    )
    .expect_err("tampered DSSE signature must fail closed");
    assert_eq!(
        error,
        PackageReleaseError::VerificationFailed("Ed25519 DSSE verification failed")
    );
    assert!(!temp.cache_root().exists());
    assert_eq!(package_release_rows(&temp.database_path()), 0);
}

#[test]
fn fkst_packages_testing_680_release_vector_is_consumed_by_host_admission() {
    let fixture = fkst_packages_testing_680_fixture();
    let temp = TempDirectory::new("real-vector-680");
    let mut journal = Journal::open(&temp.database_path()).expect("journal must open");
    let mut fetcher = MemoryFetcher::new(&fixture);
    let request = admission_request("release-admit-680", &fixture);

    let first = admit_package_release(&mut journal, &mut fetcher, &temp.cache_root(), &request)
        .expect("real #680 release vector must be admitted");
    let PackageReleaseAdmission::Created(receipt) = first else {
        panic!("real vector must create a durable receipt");
    };

    assert_eq!(
        receipt.release.sha256,
        "fc34643f3837daa098e1d7be81c27b91d56a6d3be4d0d6f244fe20146f516b56"
    );
    assert_eq!(
        receipt.authority.authorization_sha256,
        "16ec5e60a1d95d86f3594f245c49abf5c59ef471b4629ba16c095e507a4734ab"
    );
    assert_eq!(
        receipt.authority.dsse_sha256,
        "926de87c6b4be29dbf20ce58827967d84c8660f3d9b09d8e9b64a2f1312178d5"
    );
    assert_eq!(
        receipt.manifest.sha256,
        "5ccfd55cfa9c64ab36759a7ab77a1daa4989fe06c25ea722029e6166becd6143"
    );
    assert_eq!(
        receipt.manifest.manifest_digest,
        "17e252beaff18745cf09f602d1febdfb6d03810b216d197648a230553594a403"
    );
    assert_eq!(
        receipt.bundle.sha256,
        "448d480b024fc22c0b62495e5da7d8e9bbbe8758c88c60b374c243c3b1309474"
    );
    assert_eq!(
        receipt.schema_catalog.sha256,
        "0683eef00682f46435d5f2d6c86af58f678357e8ca7946daf4360fdd6097003a"
    );
    assert_eq!(
        receipt.schema_release.sha256,
        "b46cff7ecb5ff6482c4fb10ef1d4c60558d28d2577e03c7778bbd70662be0bf6"
    );
    assert_eq!(receipt.package.package_version, "1.0.0");
    assert_eq!(
        receipt.package.package_content_sha256,
        "3a4b4c2230ea807d87e4390c6fdd29b89a9ce39456c8fed4efee144b8de7f058"
    );
    assert_eq!(receipt.release.release_sequence, 1);
    assert_eq!(
        fetcher.calls,
        [
            RELEASE_PATH,
            AUTHORIZATION_PATH,
            DSSE_PATH,
            "package-release/testing-package-manifest.v1.json",
            "package-release/testing-package-bundle.v1.json",
            "schema-release/testing-schema-catalog.v1.json",
            "schema-release/testing-package-schema-release.v1.json",
        ]
    );

    drop(journal);
    let mut restarted = Journal::open(&temp.database_path()).expect("journal must reopen");
    let mut replay_fetcher = MemoryFetcher::empty();
    let replay = admit_package_release(
        &mut restarted,
        &mut replay_fetcher,
        &temp.cache_root(),
        &request,
    )
    .expect("real vector restart replay must be durable");
    assert_eq!(replay, PackageReleaseAdmission::Replay(receipt));
    assert!(replay_fetcher.calls.is_empty());
}

#[test]
fn explicit_update_and_approved_rollback_transition_are_durable() {
    let baseline = signed_fixture();
    let update = signed_authority_fixture(2, UPDATE_COMMIT_SHA);
    let temp = TempDirectory::new("transition");
    let mut journal = Journal::open(&temp.database_path()).expect("journal must open");

    let mut baseline_fetcher = MemoryFetcher::new(&baseline);
    let baseline_request = admission_request("release-baseline", &baseline);
    let baseline_admission = admit_package_release(
        &mut journal,
        &mut baseline_fetcher,
        &temp.cache_root(),
        &baseline_request,
    )
    .expect("baseline admission must succeed");
    let PackageReleaseAdmission::Created(baseline_receipt) = baseline_admission else {
        panic!("baseline admission must create a receipt");
    };
    assert_eq!(baseline_receipt.release.release_sequence, 1);

    let before_rejected_update_cache_entries = cache_entry_count(&temp.cache_root());
    let mut rejected_update_fetcher = MemoryFetcher::new(&update);
    let rejected_update = admit_package_release(
        &mut journal,
        &mut rejected_update_fetcher,
        &temp.cache_root(),
        &admission_request("release-update-missing-transition", &update),
    )
    .expect_err("newer release must require an explicit update transition");
    assert_eq!(
        rejected_update,
        PackageReleaseError::InvalidAdmissionRequest(
            "package release transition is not valid for the selected release"
        )
    );
    assert_eq!(
        cache_entry_count(&temp.cache_root()),
        before_rejected_update_cache_entries
    );
    assert_eq!(package_release_rows(&temp.database_path()), 1);

    let mut update_fetcher = MemoryFetcher::new(&update);
    let update_request = admission_request("release-update", &update).with_update_transition();
    let update_admission = admit_package_release(
        &mut journal,
        &mut update_fetcher,
        &temp.cache_root(),
        &update_request,
    )
    .expect("explicit update admission must succeed");
    let PackageReleaseAdmission::Created(update_receipt) = update_admission else {
        panic!("update admission must create a receipt");
    };
    assert_eq!(update_receipt.release.release_sequence, 2);
    assert_eq!(update_receipt.policy.transition, "update");
    assert_eq!(
        journal
            .selected_package_release()
            .expect("selected release query must succeed")
            .expect("update must select a release")
            .release_sha256,
        update_receipt.release.sha256
    );

    let before_rollback_cache_entries = cache_entry_count(&temp.cache_root());
    let mut rollback_fetcher = MemoryFetcher::empty();
    let rollback_request =
        admission_request("release-rollback", &baseline).with_approved_rollback_transition();
    let rollback_admission = admit_package_release(
        &mut journal,
        &mut rollback_fetcher,
        &temp.cache_root(),
        &rollback_request,
    )
    .expect("approved rollback to a previously admitted release must succeed");
    let PackageReleaseAdmission::Created(rollback_receipt) = rollback_admission else {
        panic!("rollback admission must create a receipt");
    };
    assert_eq!(
        rollback_receipt.release.sha256,
        baseline_receipt.release.sha256
    );
    assert_eq!(rollback_receipt.release.release_sequence, 1);
    assert_eq!(rollback_receipt.policy.transition, "approved-rollback");
    assert!(rollback_fetcher.calls.is_empty());
    assert_eq!(
        cache_entry_count(&temp.cache_root()),
        before_rollback_cache_entries
    );
    assert_eq!(package_release_rows(&temp.database_path()), 3);
    assert_eq!(
        journal
            .selected_package_release()
            .expect("selected release query must succeed")
            .expect("rollback must select a release")
            .release_sha256,
        baseline_receipt.release.sha256
    );
}

fn admission_request(idempotency_key: &str, fixture: &Fixture) -> PackageReleaseAdmissionRequest {
    PackageReleaseAdmissionRequest::new(
        idempotency_key,
        ImmutablePackageReleaseRef::new(
            TESTING_PACKAGES_REPOSITORY,
            &fixture.release_commit_sha,
            RELEASE_PATH,
        )
        .expect("fixture ref must be immutable"),
        &fixture.expected_release_sha256,
        &fixture.trusted_authorization_sha256,
        PackageReleasePolicy::production("2026-09-04T00:00:01Z")
            .expect("fixture policy must be valid"),
    )
    .expect("fixture request must be valid")
}

fn signed_fixture() -> Fixture {
    let seed = [7_u8; 32];
    let signing_key = SigningKey::from_bytes(&seed);
    let keyid = "fkst-packages-testing-release-v1-2026-09-04";
    let package_content_sha256 = package_content_sha256();
    let reducer_without_digest = json!({
        "policy_profile": "browser-title-equals.v1",
        "reducer_id": "testing.assertion-reducer.browser-title-equals",
        "reducer_version": "1.0.0",
        "schema": "testing-assertion-reducer-identity.v1",
        "supported_result_contract_majors": ["testing-case-result-set.v2"]
    });
    let reducer_sha256 = sha256_hex(&canonical(&reducer_without_digest, false));
    let manifest_without_digest = manifest_value(&package_content_sha256, None);
    let manifest_digest = sha256_hex(&canonical(&manifest_without_digest, false));
    let manifest = manifest_value(&package_content_sha256, Some(&manifest_digest));
    let manifest_bytes = canonical(&manifest, false);
    let bundle = bundle_value();
    let bundle_bytes = canonical(&bundle, true);
    let schema_catalog = schema_catalog_value();
    let schema_catalog_bytes = canonical(&schema_catalog, true);
    let schema_catalog_sha256 = sha256_hex(&schema_catalog_bytes);
    let schema_release = schema_release_value(&schema_catalog_sha256);
    let schema_release_bytes = canonical(&schema_release, true);
    let manifest_sha256 = sha256_hex(&manifest_bytes);
    let bundle_sha256 = sha256_hex(&bundle_bytes);
    let schema_release_sha256 = sha256_hex(&schema_release_bytes);
    let release_bindings = ReleaseBindings {
        package_content_sha256: &package_content_sha256,
        manifest_digest: &manifest_digest,
        manifest_sha256: &manifest_sha256,
        manifest_size: manifest_bytes.len(),
        bundle_sha256: &bundle_sha256,
        bundle_size: bundle_bytes.len(),
        catalog_sha256: &schema_catalog_sha256,
        catalog_size: schema_catalog_bytes.len(),
        schema_release_sha256: &schema_release_sha256,
        schema_release_size: schema_release_bytes.len(),
        reducer_sha256: &reducer_sha256,
    };
    let release = release_value(&release_bindings);
    let release_bytes = canonical(&release, true);
    let release_sha256 = sha256_hex(&release_bytes);
    let payload = canonical(
        &json!({
            "_type": "https://in-toto.io/Statement/v1",
            "predicate": {},
            "predicateType": "https://chronoaiproject.github.io/fkst-packages-testing/attestations/testing-package-release/v1",
            "subject": [{
                "digest": {"sha256": release_sha256},
                "name": RELEASE_PATH
            }]
        }),
        false,
    );
    let signature = signing_key.sign(&dsse_pae(&payload));
    let envelope = json!({
        "payload": STANDARD.encode(&payload),
        "payloadType": "application/vnd.in-toto+json",
        "signatures": [{
            "keyid": keyid,
            "sig": STANDARD.encode(signature.to_bytes())
        }]
    });
    let envelope_bytes = canonical(&envelope, true);
    let authorization = json!({
        "algorithm": "ed25519",
        "authorization": {
            "payloadType": "application/vnd.in-toto+json",
            "predicateType": "https://chronoaiproject.github.io/fkst-packages-testing/attestations/testing-package-release/v1",
            "subject": RELEASE_PATH
        },
        "keyid": keyid,
        "publicKey": STANDARD.encode(signing_key.verifying_key().as_bytes()),
        "schema": "testing-package-release-key-authorization.v1"
    });
    let authorization_bytes = canonical(&authorization, true);

    let mut files = BTreeMap::new();
    files.insert(RELEASE_PATH.to_owned(), release_bytes);
    files.insert(AUTHORIZATION_PATH.to_owned(), authorization_bytes.clone());
    files.insert(DSSE_PATH.to_owned(), envelope_bytes);
    files.insert(
        "package-release/testing-package-manifest.v1.json".to_owned(),
        manifest_bytes,
    );
    files.insert(
        "package-release/testing-package-bundle.v1.json".to_owned(),
        bundle_bytes,
    );
    files.insert(
        "schema-release/testing-schema-catalog.v1.json".to_owned(),
        schema_catalog_bytes,
    );
    files.insert(
        "schema-release/testing-package-schema-release.v1.json".to_owned(),
        schema_release_bytes,
    );

    Fixture {
        release_commit_sha: COMMIT_SHA.to_owned(),
        expected_release_sha256: release_sha256,
        trusted_authorization_sha256: sha256_hex(&authorization_bytes),
        files,
    }
}

fn fkst_packages_testing_680_fixture() -> Fixture {
    let mut files = BTreeMap::new();
    for (path, bytes) in [
        (
            RELEASE_PATH,
            include_bytes!(
                "fixtures/fkst-packages-testing/package-release/testing-package-release.v1.json"
            )
            .as_slice(),
        ),
        (
            AUTHORIZATION_PATH,
            include_bytes!(
                "fixtures/fkst-packages-testing/package-release/testing-package-release.v1.key.json"
            )
            .as_slice(),
        ),
        (
            DSSE_PATH,
            include_bytes!(
                "fixtures/fkst-packages-testing/package-release/testing-package-release.v1.dsse.json"
            )
            .as_slice(),
        ),
        (
            "package-release/testing-package-manifest.v1.json",
            include_bytes!(
                "fixtures/fkst-packages-testing/package-release/testing-package-manifest.v1.json"
            )
            .as_slice(),
        ),
        (
            "package-release/testing-package-bundle.v1.json",
            include_bytes!(
                "fixtures/fkst-packages-testing/package-release/testing-package-bundle.v1.json"
            )
            .as_slice(),
        ),
        (
            "schema-release/testing-schema-catalog.v1.json",
            include_bytes!(
                "fixtures/fkst-packages-testing/schema-release/testing-schema-catalog.v1.json"
            )
            .as_slice(),
        ),
        (
            "schema-release/testing-package-schema-release.v1.json",
            include_bytes!(
                "fixtures/fkst-packages-testing/schema-release/testing-package-schema-release.v1.json"
            )
            .as_slice(),
        ),
    ] {
        files.insert(path.to_owned(), bytes.to_vec());
    }

    Fixture {
        release_commit_sha: "b0d08185aa81b58a8da57ab64777ac3d48739326".to_owned(),
        expected_release_sha256: sha256_hex(&files[RELEASE_PATH]),
        trusted_authorization_sha256: sha256_hex(&files[AUTHORIZATION_PATH]),
        files,
    }
}

fn signed_authority_fixture(sequence: u64, commit_sha: &str) -> Fixture {
    let seed = [7_u8; 32];
    let signing_key = SigningKey::from_bytes(&seed);
    let keyid = "fkst-packages-testing-release-v1-2026-09-04";
    let mut fixture = signed_fixture();
    fixture.release_commit_sha = commit_sha.to_owned();

    let tool_catalog = tool_catalog_value();
    let tool_catalog_bytes = canonical(&tool_catalog, true);
    let tool_catalog_sha256 = sha256_hex(&tool_catalog_bytes);
    let mut release: Value =
        serde_json::from_slice(&fixture.files[RELEASE_PATH]).expect("release fixture must parse");
    let release_object = release
        .as_object_mut()
        .expect("release fixture must be an object");
    release_object.insert(
        "authority".to_owned(),
        json!({
            "issuer": "https://releases.chronoaiproject.org/fkst-packages-testing",
            "keyid": keyid,
            "release_sequence": sequence,
            "revocation_authority": "https://releases.chronoaiproject.org/fkst-packages-testing/revocations/v1",
            "signature_profile": "dsse-ed25519.v1",
            "valid_from": "2026-09-04T00:00:00Z",
            "valid_until": "2026-09-05T00:00:00Z"
        }),
    );
    release_object.insert(
        "tool_catalog".to_owned(),
        json!({
            "path": TOOL_CATALOG_PATH,
            "sha256": tool_catalog_sha256,
            "size_bytes": tool_catalog_bytes.len()
        }),
    );

    let release_bytes = canonical(&release, true);
    let release_sha256 = sha256_hex(&release_bytes);
    let payload = canonical(
        &json!({
            "_type": "https://in-toto.io/Statement/v1",
            "predicate": {},
            "predicateType": "https://chronoaiproject.github.io/fkst-packages-testing/attestations/testing-package-release/v1",
            "subject": [{
                "digest": {"sha256": release_sha256},
                "name": RELEASE_PATH
            }]
        }),
        false,
    );
    let signature = signing_key.sign(&dsse_pae(&payload));
    let envelope = json!({
        "payload": STANDARD.encode(&payload),
        "payloadType": "application/vnd.in-toto+json",
        "signatures": [{
            "keyid": keyid,
            "sig": STANDARD.encode(signature.to_bytes())
        }]
    });

    fixture.files.insert(RELEASE_PATH.to_owned(), release_bytes);
    fixture
        .files
        .insert(DSSE_PATH.to_owned(), canonical(&envelope, true));
    fixture
        .files
        .insert(TOOL_CATALOG_PATH.to_owned(), tool_catalog_bytes);
    fixture.expected_release_sha256 = release_sha256;
    fixture
}

struct ReleaseBindings<'a> {
    package_content_sha256: &'a str,
    manifest_digest: &'a str,
    manifest_sha256: &'a str,
    manifest_size: usize,
    bundle_sha256: &'a str,
    bundle_size: usize,
    catalog_sha256: &'a str,
    catalog_size: usize,
    schema_release_sha256: &'a str,
    schema_release_size: usize,
    reducer_sha256: &'a str,
}

fn release_value(bindings: &ReleaseBindings<'_>) -> Value {
    json!({
        "bundle": {
            "path": "package-release/testing-package-bundle.v1.json",
            "sha256": bindings.bundle_sha256,
            "size_bytes": bindings.bundle_size
        },
        "canonicalization": "fkst-testing-package-release-canonical-json.v1",
        "creation_metadata": {
            "build_id": "testing-package-release-walking-skeleton-v1",
            "created_at": "2026-09-04T00:00:00Z"
        },
        "executor": {
            "executor_id": "testing-package-executor.browser-title.v1",
            "function": "execute",
            "module": "testing_package_executor.executor"
        },
        "manifest": {
            "manifest_digest": bindings.manifest_digest,
            "path": "package-release/testing-package-manifest.v1.json",
            "sha256": bindings.manifest_sha256,
            "size_bytes": bindings.manifest_size
        },
        "mappings": [{
            "contract_major": "testing-runner.v1",
            "entrypoint": "testing-runner.run",
            "function": "execute",
            "module": "testing_package_executor.executor"
        }],
        "package": {
            "capability": "browser.read-title.v1",
            "package_content_sha256": bindings.package_content_sha256,
            "package_id": "testing-runner",
            "package_version": "1.0.0",
            "supported_profile": "browser-deterministic.v1"
        },
        "producer": {
            "generator": "scripts/generate_testing_package_release.py",
            "generator_version": "1.0.0",
            "name": "fkst-packages-testing",
            "version": "1.0.0"
        },
        "reducer": {
            "policy_profile": "browser-title-equals.v1",
            "reducer_id": "testing.assertion-reducer.browser-title-equals",
            "reducer_sha256": bindings.reducer_sha256,
            "reducer_version": "1.0.0",
            "schema": "testing-assertion-reducer-identity.v1",
            "supported_result_contract_majors": ["testing-case-result-set.v2"]
        },
        "result_authority": {
            "receipt_schema": "testing-result-authority-receipt.v1"
        },
        "runtime": {
            "lua": "5.4.0",
            "platform": "linux-amd64"
        },
        "schema": "testing-package-release.v1",
        "schema_catalog": {
            "path": "schema-release/testing-schema-catalog.v1.json",
            "sha256": bindings.catalog_sha256,
            "size_bytes": bindings.catalog_size
        },
        "schema_release": {
            "path": "schema-release/testing-package-schema-release.v1.json",
            "sha256": bindings.schema_release_sha256,
            "size_bytes": bindings.schema_release_size
        },
        "source": {
            "fkst_packages_commit": FKST_PACKAGES_COMMIT,
            "fkst_substrate_commit": FKST_SUBSTRATE_COMMIT,
            "repository_commit": SOURCE_COMMIT
        }
    })
}

fn manifest_value(package_content_sha256: &str, manifest_digest: Option<&str>) -> Value {
    let mut value = json!({
        "canonicalization": "fkst-testing-package-manifest-canonical-json.v1",
        "creation_metadata": {
            "build_id": "testing-package-release-walking-skeleton-v1",
            "created_at": "2026-09-04T00:00:00Z"
        },
        "dependencies": {
            "fkst_packages": {
                "commit": FKST_PACKAGES_COMMIT,
                "id": "fkst-packages"
            },
            "fkst_substrate": {
                "commit": FKST_SUBSTRATE_COMMIT,
                "id": "fkst-substrate"
            }
        },
        "entrypoints": [{
            "capabilities": ["browser.read-title.v1"],
            "contract_major": "testing-runner.v1",
            "name": "testing-runner.run"
        }],
        "package_content_sha256": package_content_sha256,
        "package_id": "testing-runner",
        "package_version": "1.0.0",
        "producer": {
            "name": "fkst-packages-testing",
            "toolchain": "testing-package-release-v1",
            "version": "1.0.0"
        },
        "runtime_requirements": {
            "lua": "5.4.0",
            "platforms": ["linux-amd64"]
        },
        "schema": "testing-package-manifest.v1",
        "semantic_capabilities": ["browser.read-title.v1"],
        "source_commit": SOURCE_COMMIT,
        "supported_contracts": {
            "canonicalization_profiles": ["fkst-testing-package-manifest-canonical-json.v1"],
            "majors": ["testing-runner.v1"]
        }
    });
    if let Some(digest) = manifest_digest {
        value
            .as_object_mut()
            .expect("manifest fixture must be an object")
            .insert(
                "manifest_digest".to_owned(),
                Value::String(digest.to_owned()),
            );
    }
    value
}

fn bundle_value() -> Value {
    json!({
        "files": bundle_records(),
        "schema": "testing-package-bundle.v1"
    })
}

fn bundle_records() -> Vec<Value> {
    bundle_paths()
        .iter()
        .map(|path| {
            let content = format!("-- fixture {path}\n").into_bytes();
            json!({
                "content_base64": STANDARD.encode(&content),
                "path": path,
                "sha256": sha256_hex(&content),
                "size_bytes": content.len()
            })
        })
        .collect()
}

fn bundle_paths() -> [&'static str; 10] {
    [
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
    ]
}

fn package_content_sha256() -> String {
    let mut bytes = Vec::new();
    for record in bundle_records() {
        let path = record["path"].as_str().expect("path must be a string");
        let content = STANDARD
            .decode(
                record["content_base64"]
                    .as_str()
                    .expect("content must be base64"),
            )
            .expect("content must decode");
        bytes.extend_from_slice(path.as_bytes());
        bytes.extend_from_slice(&[0, 0x66]);
        bytes.extend_from_slice(&content);
        bytes.push(0);
    }
    sha256_hex(&bytes)
}

fn schema_catalog_value() -> Value {
    json!({
        "canonicalization": "fkst-testing-schema-catalog-canonical-json.v1",
        "catalog_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "schema": "testing-schema-catalog.v1",
        "schemas": [{
            "canonicalization_profile": "fkst-testing-package-release-canonical-json.v1",
            "contract_major": 1,
            "draft": "https://json-schema.org/draft/2020-12/schema",
            "fixture_set_path": "schema-release/fixture-sets/testing-package-release.v1.json",
            "fixture_set_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "path": "schemas/testing-package-release.v1.schema.json",
            "schema_id": "https://chronoaiproject.github.io/fkst-packages-testing/schemas/testing-package-release.v1.schema.json",
            "schema_sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "status": "stable"
        }, {
            "canonicalization_profile": null,
            "contract_major": 1,
            "draft": "https://json-schema.org/draft/2020-12/schema",
            "fixture_set_path": "schema-release/fixture-sets/testing-package-manifest.v1.json",
            "fixture_set_sha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            "path": "schemas/testing-package-manifest.v1.schema.json",
            "schema_id": "https://chronoaiproject.github.io/fkst-packages-testing/schemas/testing-package-manifest.v1.schema.json",
            "schema_sha256": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "status": "stable"
        }, {
            "canonicalization_profile": "fkst-testing-results-canonical-json.v1",
            "contract_major": 2,
            "draft": "https://json-schema.org/draft/2020-12/schema",
            "fixture_set_path": "schema-release/fixture-sets/testing-case-result-set.v2.json",
            "fixture_set_sha256": "1212121212121212121212121212121212121212121212121212121212121212",
            "path": "schemas/testing-case-result-set.v2.schema.json",
            "schema_id": "https://chronoaiproject.github.io/fkst-packages-testing/schemas/testing-case-result-set.v2.schema.json",
            "schema_sha256": "3434343434343434343434343434343434343434343434343434343434343434",
            "status": "stable"
        }]
    })
}

fn schema_release_value(catalog_sha256: &str) -> Value {
    json!({
        "canonicalization": "fkst-testing-package-schema-release-canonical-json.v1",
        "package_manifest": {
            "kind": "testing-package-manifest",
            "ref": "immutable://fkst-packages-testing/1.0.0/testing-package-manifest.v1.json",
            "sha256": "5656565656565656565656565656565656565656565656565656565656565656"
        },
        "producer": {
            "name": "fkst-packages-testing",
            "version": "1.0.0"
        },
        "release_sha256": "7878787878787878787878787878787878787878787878787878787878787878",
        "schema": "testing-package-schema-release.v1",
        "schema_catalog": {
            "kind": "testing-schema-catalog",
            "ref": "immutable://fkst-packages-testing/1.0.0/testing-schema-catalog.v1.json",
            "sha256": catalog_sha256
        }
    })
}

fn tool_catalog_value() -> Value {
    json!({
        "canonicalization": "fkst-testing-package-tool-catalog-canonical-json.v1",
        "execution_profile": "browser-deterministic.v1",
        "schema": "testing-package-tool-catalog.v1",
        "tools": [{
            "capability": "browser.read-title.v1",
            "port": "browser-title"
        }]
    })
}

fn canonical(value: &Value, trailing_lf: bool) -> Vec<u8> {
    let sorted = sorted_value(value);
    let mut bytes = serde_json::to_vec(&sorted).expect("fixture JSON must serialize");
    if trailing_lf {
        bytes.push(b'\n');
    }
    bytes
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

fn dsse_pae(payload: &[u8]) -> Vec<u8> {
    let payload_type = b"application/vnd.in-toto+json";
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
    digest
        .as_slice()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn cache_dir(cache_root: &Path, release_sha256: &str) -> PathBuf {
    cache_root
        .join("package-release")
        .join("sha256")
        .join(release_sha256)
}

fn cache_entry_count(cache_root: &Path) -> usize {
    let root = cache_root.join("package-release").join("sha256");
    match fs::read_dir(root) {
        Ok(entries) => entries.count(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => 0,
        Err(error) => panic!("cache directory count must be readable: {error}"),
    }
}

fn package_release_rows(database_path: &Path) -> i64 {
    let connection = rusqlite::Connection::open(database_path).expect("database must open");
    connection
        .query_row(
            "SELECT COUNT(*) FROM package_release_admissions",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("package release row count must be readable")
}
