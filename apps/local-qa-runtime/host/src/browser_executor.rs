use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fkst_local_qa_browser_adapter::{
    prepare_fixed_browser_session, FixedBrowserObservation, PreparedFixedBrowserSession,
};
use fkst_local_qa_evidence_stager::{
    EvidenceMediaType, EvidenceRole, EvidenceStager, StageRequest,
    StageSanitizedObservationRequest, StagedEvidence, StagedSanitizedObservation,
};
use fkst_qa_contracts::{
    canonical_bytes, encode_local_worker_frame, validate_local_worker_capability_request,
    validate_local_worker_capability_result, validate_local_worker_invocation,
    validate_local_worker_terminal_result, AttemptBindingV2, LocalWorkerFrameDecoder,
    LocalWorkerInputSequence, ValidatedValue,
};
use serde_json::{json, Value};

use crate::admission::{
    CurrentClaimVerification, CurrentClaimVerifier, Mvp0DeterministicCurrentClaimVerifier,
};
use crate::executor::{
    ExecutorDescriptor, ExecutorRequest, ExecutorResult, ExecutorSelection, VersionedExecutor,
};
use crate::worker_process::WorkerProcess;
use crate::RunError;

const PROTOCOL: &str = "qa.local-worker-protocol/v1";
const INVOCATION_ID: &str = "invocation/0";
const SELECTOR: &str = r#"[data-local-qa="status"]"#;
const EXPECTED_TEXT: &str = "READY";
const RUNNER_LOG: &[u8] = b"navigation accepted\nassertion passed\n";
const EXECUTION_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn browser_executor_selection() -> ExecutorSelection {
    ExecutorSelection {
        schema_version: "qa.local-executor/v1".to_owned(),
        executor_id: "fake.browser".to_owned(),
        executor_version: "1.0.0".to_owned(),
        capability_digest:
            "sha256:0f447361154fd5aa70f1b6c830547ae0401a3b185174177a123d9dbce1dc41b1".to_owned(),
        required_capability: "browser.observe".to_owned(),
    }
}

fn browser_executor_descriptor() -> ExecutorDescriptor {
    ExecutorDescriptor {
        schema_version: "qa.local-executor/v1".to_owned(),
        executor_id: "fake.browser".to_owned(),
        executor_version: "1.0.0".to_owned(),
        capabilities: vec!["browser.observe".to_owned()],
        capability_digest:
            "sha256:0f447361154fd5aa70f1b6c830547ae0401a3b185174177a123d9dbce1dc41b1".to_owned(),
    }
}

#[derive(Default)]
pub(crate) struct EffectCounters {
    worker_spawns: AtomicUsize,
    capability_exchanges: AtomicUsize,
    browser_observations: AtomicUsize,
    browser_closes: AtomicUsize,
    evidence_stages: AtomicUsize,
    fixture_url: Mutex<Option<String>>,
}

struct HostObservation {
    final_url: String,
    observed_text: String,
    staged: StagedSanitizedObservation,
}

pub(crate) struct BrowserWorkerExecutor {
    descriptor: ExecutorDescriptor,
    node_executable: PathBuf,
    worker_entrypoint: PathBuf,
    chrome_executable: PathBuf,
    staging_root: PathBuf,
    attempt_binding: AttemptBindingV2,
    current_claim_verifier: Arc<dyn CurrentClaimVerifier>,
    claim_now: String,
    started_at: String,
    finished_at: String,
    started_monotonic_ms: u64,
    finished_monotonic_ms: u64,
    counters: Arc<EffectCounters>,
}

impl BrowserWorkerExecutor {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        node_executable: PathBuf,
        worker_entrypoint: PathBuf,
        chrome_executable: PathBuf,
        staging_root: PathBuf,
        attempt_binding: AttemptBindingV2,
        current_claim_verifier: Arc<dyn CurrentClaimVerifier>,
        claim_now: String,
        started_at: String,
        finished_at: String,
        started_monotonic_ms: u64,
        finished_monotonic_ms: u64,
        counters: Arc<EffectCounters>,
    ) -> Result<Self, RunError> {
        validate_executable(&node_executable)?;
        validate_regular_file(&worker_entrypoint)?;
        validate_executable(&chrome_executable)?;
        if !staging_root.is_absolute() {
            return Err(RunError::Contract("Browser staging root must be absolute"));
        }
        if finished_monotonic_ms < started_monotonic_ms {
            return Err(RunError::Contract("Browser clock samples are invalid"));
        }
        Ok(Self {
            descriptor: browser_executor_descriptor(),
            node_executable,
            worker_entrypoint,
            chrome_executable,
            staging_root,
            attempt_binding,
            current_claim_verifier,
            claim_now,
            started_at,
            finished_at,
            started_monotonic_ms,
            finished_monotonic_ms,
            counters,
        })
    }

    fn verify_current_claim(&self) -> Result<(), RunError> {
        if self
            .current_claim_verifier
            .verify(&self.attempt_binding, &self.claim_now)
            == CurrentClaimVerification::Verified
        {
            Ok(())
        } else {
            Err(RunError::Contract("Browser current claim is not verified"))
        }
    }

    fn execute_worker(&self, request: &ExecutorRequest) -> Result<(), RunError> {
        self.verify_current_claim()?;
        let mut browser = Some(
            prepare_fixed_browser_session(&self.chrome_executable)
                .map_err(|_| RunError::Contract("Browser preparation failed"))?,
        );
        let fixture_url = browser
            .as_ref()
            .expect("prepared Browser exists")
            .fixture_url()
            .to_owned();
        *self
            .counters
            .fixture_url
            .lock()
            .map_err(|_| RunError::Contract("Browser fixture counter lock poisoned"))? =
            Some(fixture_url.clone());
        let stager = EvidenceStager::new(&self.staging_root);
        let mut process = match WorkerProcess::spawn(&self.node_executable, &self.worker_entrypoint)
        {
            Ok(process) => {
                self.counters.worker_spawns.fetch_add(1, Ordering::SeqCst);
                process
            }
            Err(error) => {
                close_browser(&mut browser, &self.counters)?;
                return Err(error);
            }
        };
        let deadline = Instant::now() + EXECUTION_TIMEOUT;
        let protocol = self.walk_protocol(
            request,
            &fixture_url,
            &stager,
            &mut browser,
            &mut process,
            deadline,
        );
        match protocol {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = close_browser(&mut browser, &self.counters);
                process.terminate();
                let cleanup = stager
                    .cleanup_attempt(&request.run_id, 1)
                    .map_err(|_| RunError::Contract("partial Browser staging cleanup failed"))?;
                if !cleanup.is_complete() {
                    return Err(RunError::Contract(
                        "partial Browser staging cleanup left residuals",
                    ));
                }
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_protocol(
        &self,
        request: &ExecutorRequest,
        fixture_url: &str,
        stager: &EvidenceStager,
        browser: &mut Option<PreparedFixedBrowserSession>,
        process: &mut WorkerProcess,
        deadline: Instant,
    ) -> Result<(), RunError> {
        let mut input_sequence = LocalWorkerInputSequence::default();
        let invocation = validate_frame(
            json!({
                "protocol": PROTOCOL,
                "kind": "invocation",
                "invocation_id": INVOCATION_ID,
                "operation": "browser-smoke",
                "input": fixed_request(fixture_url),
            }),
            validate_local_worker_invocation,
            "invalid Browser Worker invocation",
        )?;
        write_host_frame(process, &mut input_sequence, &invocation)?;

        let mut decoder = LocalWorkerFrameDecoder::default();
        let mut observation: Option<HostObservation> = None;
        let mut screenshot: Option<StagedEvidence> = None;
        let mut runner_log: Option<StagedEvidence> = None;
        for index in 0..7 {
            let frame = process.read_frame(&mut decoder, deadline)?;
            let expected_input = expected_capability_input(index, fixture_url);
            let capability = capability_name(index);
            let expected = json!({
                "protocol": PROTOCOL,
                "kind": "capability_request",
                "invocation_id": INVOCATION_ID,
                "request_id": format!("capability/{index}"),
                "capability": capability,
                "input": expected_input,
            });
            require_exact_capability_request(&frame, &expected)?;
            self.counters
                .capability_exchanges
                .fetch_add(1, Ordering::SeqCst);

            let output = match index {
                0 => json!({ "value": self.started_at }),
                1 => json!({ "value": self.started_monotonic_ms }),
                2 => {
                    self.verify_current_claim()?;
                    self.counters
                        .browser_observations
                        .fetch_add(1, Ordering::SeqCst);
                    let observed = browser
                        .as_mut()
                        .ok_or(RunError::Contract("Browser session closed before run"))?
                        .observe()
                        .map_err(|_| RunError::Contract("Browser observation failed"))?;
                    validate_host_observation(fixture_url, &observed)?;
                    let observed_final_url = observed.final_url.clone();
                    let observed_text = observed.observed_text.clone();
                    let staged_observation = stager
                        .stage_sanitized_observation(StageSanitizedObservationRequest {
                            run_id: &request.run_id,
                            attempt: 1,
                            observation_id: "observation/0",
                            fixture_url,
                            final_url: &observed.final_url,
                            selector: SELECTOR,
                            expected_text: EXPECTED_TEXT,
                            observed_text: &observed.observed_text,
                        })
                        .map_err(|_| RunError::Contract("sanitized observation staging failed"))?;
                    stager
                        .verify_sanitized_observation(&staged_observation)
                        .map_err(|_| {
                            RunError::Contract("sanitized observation verification failed")
                        })?;
                    self.counters.evidence_stages.fetch_add(1, Ordering::SeqCst);
                    let staged_screenshot = stager
                        .stage(StageRequest {
                            run_id: &request.run_id,
                            attempt: 1,
                            object_id: "evidence/0",
                            role: EvidenceRole::BrowserScreenshot,
                            media_type: EvidenceMediaType::Png,
                            bytes: &observed.screenshot.bytes,
                        })
                        .map_err(|_| RunError::Contract("screenshot Evidence staging failed"))?;
                    stager.verify(&staged_screenshot).map_err(|_| {
                        RunError::Contract("screenshot Evidence verification failed")
                    })?;
                    let output = json!({
                        "finalUrl": observed.final_url,
                        "observedText": observed.observed_text,
                        "sanitizedObservationRef": staged_observation.observation_ref().value(),
                        "screenshotEvidenceRef": staged_screenshot.object_ref().value(),
                    });
                    observation = Some(HostObservation {
                        final_url: observed_final_url,
                        observed_text,
                        staged: staged_observation,
                    });
                    screenshot = Some(staged_screenshot);
                    output
                }
                3 => {
                    self.counters.evidence_stages.fetch_add(1, Ordering::SeqCst);
                    let staged = stager
                        .stage(StageRequest {
                            run_id: &request.run_id,
                            attempt: 1,
                            object_id: "evidence/1",
                            role: EvidenceRole::RunnerLog,
                            media_type: EvidenceMediaType::PlainTextUtf8,
                            bytes: RUNNER_LOG,
                        })
                        .map_err(|_| RunError::Contract("runner-log Evidence staging failed"))?;
                    stager.verify(&staged).map_err(|_| {
                        RunError::Contract("runner-log Evidence verification failed")
                    })?;
                    let output = json!({ "runnerLogEvidenceRef": staged.object_ref().value() });
                    runner_log = Some(staged);
                    output
                }
                4 => {
                    close_browser(browser, &self.counters)?;
                    json!({})
                }
                5 => json!({ "value": self.finished_at }),
                6 => json!({ "value": self.finished_monotonic_ms }),
                _ => unreachable!(),
            };
            let result = validate_frame(
                json!({
                    "protocol": PROTOCOL,
                    "kind": "capability_result",
                    "invocation_id": INVOCATION_ID,
                    "request_id": format!("capability/{index}"),
                    "capability": capability,
                    "output": output,
                }),
                validate_local_worker_capability_result,
                "invalid Browser capability result",
            )?;
            write_host_frame(process, &mut input_sequence, &result)?;
        }
        input_sequence
            .finish()
            .map_err(|_| RunError::Contract("incomplete Browser capability sequence"))?;
        process.close_stdin()?;

        let terminal = process.read_frame(&mut decoder, deadline)?;
        let terminal_bytes = canonical_bytes(&terminal)
            .map_err(|_| RunError::Contract("invalid Browser terminal serialization"))?;
        let terminal = validate_local_worker_terminal_result(&terminal_bytes)
            .map_err(|_| RunError::Contract("invalid Browser terminal result"))?;
        let observation = observation.ok_or(RunError::Contract("missing sanitized observation"))?;
        let screenshot = screenshot.ok_or(RunError::Contract("missing screenshot Evidence"))?;
        let runner_log = runner_log.ok_or(RunError::Contract("missing runner-log Evidence"))?;
        let expected_terminal = json!({
            "protocol": PROTOCOL,
            "kind": "terminal_result",
            "invocation_id": INVOCATION_ID,
            "outcome": "passed",
            "result": {
                "version": "local-qa-browser-smoke/result-v1",
                "outcome": "passed",
                "observation": {
                    "fixtureUrl": fixture_url,
                    "finalUrl": observation.final_url,
                    "selector": SELECTOR,
                    "expectedText": EXPECTED_TEXT,
                    "observedText": observation.observed_text,
                    "sanitizedObservationRef": observation.staged.observation_ref().value(),
                },
                "startedAt": self.started_at,
                "finishedAt": self.finished_at,
                "durationMs": self.finished_monotonic_ms - self.started_monotonic_ms,
                "evidence": [
                    {
                        "objectId": "evidence/0",
                        "role": "screenshot",
                        "artifactRef": screenshot.object_ref().value(),
                    },
                    {
                        "objectId": "evidence/1",
                        "role": "runner-log",
                        "artifactRef": runner_log.object_ref().value(),
                    }
                ],
            },
        });
        if terminal.value() != &expected_terminal {
            return Err(RunError::Contract(
                "Browser terminal result relation failed",
            ));
        }
        process.require_clean_eof(&mut decoder, deadline)?;
        process.wait_success(deadline)
    }
}

impl VersionedExecutor for BrowserWorkerExecutor {
    fn descriptor(&self) -> &ExecutorDescriptor {
        &self.descriptor
    }

    fn execute(&self, request: &ExecutorRequest) -> Result<ExecutorResult, RunError> {
        self.execute_worker(request)?;
        Ok(ExecutorResult {
            schema_version: "qa.local-executor/v1".to_owned(),
            run_id: request.run_id.clone(),
            executor_id: self.descriptor.executor_id.clone(),
            executor_version: self.descriptor.executor_version.clone(),
            capability_digest: self.descriptor.capability_digest.clone(),
            execution_outcome: "passed".to_owned(),
        })
    }
}

fn fixed_request(fixture_url: &str) -> Value {
    json!({
        "version": "local-qa-browser-smoke/request-v1",
        "fixtureUrl": fixture_url,
        "selector": SELECTOR,
        "expectedText": EXPECTED_TEXT,
        "timeoutMs": 5000,
    })
}

fn capability_name(index: usize) -> &'static str {
    [
        "clock.now/v1",
        "clock.monotonic-ms/v1",
        "browser-session.run/v1",
        "evidence.stage-fixed-runner-log/v1",
        "browser-session.close/v1",
        "clock.now/v1",
        "clock.monotonic-ms/v1",
    ][index]
}

fn expected_capability_input(index: usize, fixture_url: &str) -> Value {
    match index {
        0 | 1 | 4 | 5 | 6 => json!({}),
        2 => fixed_request(fixture_url),
        3 => json!({
            "name": "runner.log",
            "mediaType": "text/plain; charset=utf-8",
            "template": "fixed-browser-smoke-runner-log/v1",
        }),
        _ => unreachable!(),
    }
}

fn require_exact_capability_request(
    frame: &ValidatedValue,
    expected: &Value,
) -> Result<(), RunError> {
    let bytes = canonical_bytes(frame)
        .map_err(|_| RunError::Contract("invalid Browser capability request serialization"))?;
    let validated = validate_local_worker_capability_request(&bytes)
        .map_err(|_| RunError::Contract("invalid Browser capability request"))?;
    if validated.value() == expected {
        Ok(())
    } else {
        Err(RunError::Contract("unexpected Browser capability request"))
    }
}

fn validate_frame(
    value: Value,
    validator: fn(&[u8]) -> Result<ValidatedValue, fkst_qa_contracts::ContractError>,
    error: &'static str,
) -> Result<ValidatedValue, RunError> {
    let bytes = serde_json::to_vec(&value).map_err(|_| RunError::Contract(error))?;
    validator(&bytes).map_err(|_| RunError::Contract(error))
}

fn write_host_frame(
    process: &mut WorkerProcess,
    sequence: &mut LocalWorkerInputSequence,
    frame: &ValidatedValue,
) -> Result<(), RunError> {
    sequence
        .accept(frame)
        .map_err(|_| RunError::Contract("invalid Host Worker input sequence"))?;
    let encoded = encode_local_worker_frame(frame)
        .map_err(|_| RunError::Contract("Browser Worker frame encoding failed"))?;
    process.write(&encoded)
}

fn validate_host_observation(
    fixture_url: &str,
    observation: &FixedBrowserObservation,
) -> Result<(), RunError> {
    if observation.final_url != fixture_url {
        return Err(RunError::Contract(
            "Browser final URL did not match fixture",
        ));
    }
    if observation.observed_text != EXPECTED_TEXT {
        return Err(RunError::Contract("Browser observed text did not pass"));
    }
    Ok(())
}

fn close_browser(
    browser: &mut Option<PreparedFixedBrowserSession>,
    counters: &EffectCounters,
) -> Result<(), RunError> {
    let Some(browser) = browser.take() else {
        return Err(RunError::Contract("Browser session was already closed"));
    };
    counters.browser_closes.fetch_add(1, Ordering::SeqCst);
    browser
        .close()
        .map_err(|_| RunError::Contract("Browser cleanup failed"))
}

fn validate_regular_file(path: &Path) -> Result<(), RunError> {
    let metadata = fs::metadata(path)
        .map_err(|_| RunError::Contract("explicit Worker path is unavailable"))?;
    if !path.is_absolute() || !metadata.is_file() {
        return Err(RunError::Contract(
            "explicit Worker path must be an absolute regular file",
        ));
    }
    Ok(())
}

fn validate_executable(path: &Path) -> Result<(), RunError> {
    let metadata =
        fs::metadata(path).map_err(|_| RunError::Contract("explicit executable is unavailable"))?;
    if !path.is_absolute() || !metadata.is_file() {
        return Err(RunError::Contract(
            "explicit executable must be an absolute regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(RunError::Contract("explicit path is not executable"));
        }
    }
    Ok(())
}

pub(crate) fn mvp0_attempt_binding() -> AttemptBindingV2 {
    AttemptBindingV2 {
        qa_task_id: "qa-task-0002".to_owned(),
        qa_attempt_id: "qa-attempt-0002".to_owned(),
        machine_id: "machine-0002".to_owned(),
        worker_id: "worker-0002".to_owned(),
        installation_id: "installation-0002".to_owned(),
        generation: 1,
        fence_token: "dGVzdC1mZW5jZS0wMDAwMDAwMg".to_owned(),
        deadline: "2026-08-25T16:05:00Z".to_owned(),
    }
}

pub(crate) fn mvp0_executor(
    node: PathBuf,
    worker: PathBuf,
    chrome: PathBuf,
    staging_root: PathBuf,
    counters: Arc<EffectCounters>,
) -> Result<BrowserWorkerExecutor, RunError> {
    BrowserWorkerExecutor::new(
        node,
        worker,
        chrome,
        staging_root,
        mvp0_attempt_binding(),
        Arc::new(Mvp0DeterministicCurrentClaimVerifier),
        "2026-08-25T16:00:01Z".to_owned(),
        "2026-08-25T16:00:01Z".to_owned(),
        "2026-08-25T16:00:01.012Z".to_owned(),
        1_000,
        1_012,
        counters,
    )
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use std::net::TcpStream;
    #[cfg(target_os = "linux")]
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    #[cfg(target_os = "linux")]
    use crate::coordinator::CoordinatorHandle;
    #[cfg(target_os = "linux")]
    use crate::executor::ExecutorRegistry;
    #[cfg(target_os = "linux")]
    use crate::journal::Journal;

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires explicit Node, prebuilt Worker bundle, and system Chrome"]
    fn browser_worker_walking_skeleton() {
        let node = required_absolute_path("FKST_LOCAL_QA_NODE");
        let chrome = required_absolute_path("FKST_LOCAL_QA_CHROME");
        let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../workers/dist/worker-main.js")
            .canonicalize()
            .expect("prebuilt Worker bundle is required");
        let directory = temporary_directory("browser-worker-walking-skeleton");
        let database_path = directory.join("journal.sqlite");
        let staging_root = directory.join("staging");
        let run_id = "00000000-0000-0000-0000-000000000016";
        let counters = Arc::new(EffectCounters::default());
        let mut journal = Journal::open(&database_path).expect("Journal opens");
        journal
            .seed_executable_v1(
                run_id,
                "idem-6116",
                "6116611661166116611661166116611661166116611661166116611661166116",
            )
            .expect("executable v1 row is seeded");

        let registry = ExecutorRegistry::new(vec![Box::new(
            mvp0_executor(
                node.clone(),
                worker.clone(),
                chrome.clone(),
                staging_root.clone(),
                Arc::clone(&counters),
            )
            .expect("Browser executor constructs"),
        )])
        .expect("Browser registry constructs");
        let mut coordinator = CoordinatorHandle::start_versioned(
            &database_path,
            registry,
            browser_executor_selection(),
        )
        .expect("coordinator walks real Browser execution");
        coordinator.shutdown().expect("coordinator joins");

        assert_eq!(counters.worker_spawns.load(Ordering::SeqCst), 1);
        assert_eq!(counters.capability_exchanges.load(Ordering::SeqCst), 7);
        assert_eq!(counters.browser_observations.load(Ordering::SeqCst), 1);
        assert_eq!(counters.browser_closes.load(Ordering::SeqCst), 1);
        assert_eq!(counters.evidence_stages.load(Ordering::SeqCst), 2);
        let fixture_url = counters
            .fixture_url
            .lock()
            .expect("fixture URL lock")
            .clone()
            .expect("fixture URL recorded");
        let fixture_address = fixture_url
            .strip_prefix("http://")
            .and_then(|url| url.strip_suffix("/fixed-page.html"))
            .expect("fixed fixture URL shape");
        assert!(TcpStream::connect(fixture_address).is_err());

        let snapshot = journal
            .snapshot(run_id)
            .expect("snapshot reads")
            .expect("Run exists");
        assert_eq!(snapshot.state, "terminal");
        assert_eq!(snapshot.execution_outcome.as_deref(), Some("passed"));
        assert_eq!(snapshot.latest_event_sequence, 9);
        let events = journal
            .events(run_id, 0, 20)
            .expect("events read")
            .expect("Run events exist");
        assert_eq!(events.len(), 9);
        assert_eq!(
            events.last().expect("terminal event").event_type,
            "run.completed"
        );

        let stager = EvidenceStager::new(&staging_root);
        let observation = stager
            .load_sanitized_observation(run_id, 1, "observation/0")
            .expect("sanitized observation reloads");
        stager
            .verify_sanitized_observation(&observation)
            .expect("sanitized observation verifies");
        let screenshot = stager
            .load(
                run_id,
                1,
                "evidence/0",
                EvidenceRole::BrowserScreenshot,
                EvidenceMediaType::Png,
            )
            .expect("screenshot Evidence reloads");
        stager.verify(&screenshot).expect("screenshot verifies");
        let runner_log = stager
            .load(
                run_id,
                1,
                "evidence/1",
                EvidenceRole::RunnerLog,
                EvidenceMediaType::PlainTextUtf8,
            )
            .expect("runner-log Evidence reloads");
        stager.verify(&runner_log).expect("runner log verifies");
        let stored_before = stored_artifact_bytes(&staging_root, run_id);

        let restart_registry = ExecutorRegistry::new(vec![Box::new(
            mvp0_executor(
                node,
                worker,
                chrome,
                staging_root.clone(),
                Arc::clone(&counters),
            )
            .expect("restart Browser executor constructs"),
        )])
        .expect("restart registry constructs");
        let mut restarted = CoordinatorHandle::start_versioned(
            &database_path,
            restart_registry,
            browser_executor_selection(),
        )
        .expect("completed Journal restarts");
        restarted.shutdown().expect("restarted coordinator joins");
        assert_eq!(counters.worker_spawns.load(Ordering::SeqCst), 1);
        assert_eq!(counters.capability_exchanges.load(Ordering::SeqCst), 7);
        assert_eq!(counters.browser_observations.load(Ordering::SeqCst), 1);
        assert_eq!(counters.browser_closes.load(Ordering::SeqCst), 1);
        assert_eq!(counters.evidence_stages.load(Ordering::SeqCst), 2);
        assert_eq!(stored_artifact_bytes(&staging_root, run_id), stored_before);

        drop(journal);
        fs::remove_dir_all(directory).expect("temporary test directory removed");
    }

    #[test]
    fn host_observation_must_independently_pass_before_terminal_authority() {
        let fixture_url = "http://127.0.0.1:43123/fixed-page.html";
        let not_ready = FixedBrowserObservation {
            final_url: fixture_url.to_owned(),
            observed_text: "NOT READY".to_owned(),
            screenshot: fkst_local_qa_browser_adapter::FixedPngScreenshot {
                bytes: Vec::new(),
                media_type: "image/png".to_owned(),
                width_px: 1280,
                height_px: 720,
            },
        };
        assert!(validate_host_observation(fixture_url, &not_ready).is_err());

        let wrong_url = FixedBrowserObservation {
            final_url: "http://127.0.0.1:43124/fixed-page.html".to_owned(),
            observed_text: EXPECTED_TEXT.to_owned(),
            screenshot: not_ready.screenshot,
        };
        assert!(validate_host_observation(fixture_url, &wrong_url).is_err());
    }

    #[test]
    fn malformed_or_unexpected_capability_cannot_match() {
        let malformed = validate_local_worker_capability_request(b"{}")
            .expect_err("malformed capability request is rejected");
        assert!(malformed.to_string().contains("schema_violation"));

        let fixture_url = "http://127.0.0.1:43123/fixed-page.html";
        let wrong = validate_frame(
            json!({
                "protocol": PROTOCOL,
                "kind": "capability_request",
                "invocation_id": INVOCATION_ID,
                "request_id": "capability/0",
                "capability": "clock.monotonic-ms/v1",
                "input": {},
            }),
            validate_local_worker_capability_request,
            "invalid test request",
        )
        .expect("wrong but valid capability request");
        let expected = json!({
            "protocol": PROTOCOL,
            "kind": "capability_request",
            "invocation_id": INVOCATION_ID,
            "request_id": "capability/0",
            "capability": "clock.now/v1",
            "input": expected_capability_input(0, fixture_url),
        });
        assert!(require_exact_capability_request(&wrong, &expected).is_err());
    }

    #[cfg(target_os = "linux")]
    fn required_absolute_path(name: &str) -> PathBuf {
        let value = std::env::var_os(name).unwrap_or_else(|| panic!("{name} must be set"));
        PathBuf::from(value)
            .canonicalize()
            .unwrap_or_else(|_| panic!("{name} must name an existing path"))
    }

    #[cfg(target_os = "linux")]
    fn temporary_directory(label: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock follows Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "fkst-local-qa-host-{label}-{}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("temporary test directory created");
        directory
    }

    #[cfg(target_os = "linux")]
    fn stored_artifact_bytes(root: &Path, run_id: &str) -> Vec<(PathBuf, Vec<u8>)> {
        let attempt = root.join(run_id).join("1");
        let mut pending = vec![attempt.clone()];
        let mut stored = Vec::new();
        while let Some(path) = pending.pop() {
            for entry in fs::read_dir(path).expect("artifact directory reads") {
                let entry = entry.expect("artifact entry reads");
                let file_type = entry.file_type().expect("artifact type reads");
                if file_type.is_dir() {
                    pending.push(entry.path());
                } else if file_type.is_file() {
                    stored.push((
                        entry
                            .path()
                            .strip_prefix(&attempt)
                            .expect("artifact is attempt-scoped")
                            .to_path_buf(),
                        fs::read(entry.path()).expect("artifact bytes read"),
                    ));
                }
            }
        }
        stored.sort_by(|left, right| left.0.cmp(&right.0));
        stored
    }
}
