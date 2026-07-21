//! Kubernetes Lease leader election for the control-plane side-effect loops.
//!
//! Ownership changes use whole-object replacements carrying the last observed
//! `metadata.resourceVersion`. A stale contender therefore receives a 409 rather
//! than overwriting a newer holder. The Lease is never deleted or cleared: its
//! holder and transition count remain durable evidence across process shutdowns.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;
use k8s_openapi::chrono::{DateTime, Utc};
use kube::api::{Api, PostParams};
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::error::AppError;
use crate::leader_config::LeaderElectionConfig;
use crate::recovery::RecoveryMonitor;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
enum LeaseApiError {
    #[error("Lease does not exist")]
    NotFound,
    #[error("Lease already exists")]
    AlreadyExists,
    #[error("Lease update conflict")]
    Conflict,
    #[error("Lease API access denied")]
    Forbidden,
    #[error("Lease API request failed")]
    Api,
    #[error("Lease transport failed")]
    Transport,
    #[error("Lease object is missing required concurrency metadata")]
    InvalidObject,
}

#[async_trait]
trait LeaseApi: Send + Sync {
    async fn get(&self, name: &str) -> Result<Lease, LeaseApiError>;
    async fn create(&self, lease: &Lease) -> Result<Lease, LeaseApiError>;
    async fn replace(&self, name: &str, lease: &Lease) -> Result<Lease, LeaseApiError>;
}

struct KubernetesLeaseApi {
    api: Api<Lease>,
}

impl KubernetesLeaseApi {
    fn new(client: kube::Client, namespace: &str) -> Self {
        Self {
            api: Api::namespaced(client, namespace),
        }
    }
}

#[async_trait]
impl LeaseApi for KubernetesLeaseApi {
    async fn get(&self, name: &str) -> Result<Lease, LeaseApiError> {
        self.api.get(name).await.map_err(map_kube_error)
    }

    async fn create(&self, lease: &Lease) -> Result<Lease, LeaseApiError> {
        self.api
            .create(&PostParams::default(), lease)
            .await
            .map_err(map_kube_error)
    }

    async fn replace(&self, name: &str, lease: &Lease) -> Result<Lease, LeaseApiError> {
        self.api
            .replace(name, &PostParams::default(), lease)
            .await
            .map_err(map_kube_error)
    }
}

fn map_kube_error(error: kube::Error) -> LeaseApiError {
    match error {
        kube::Error::Api(response) => match response.code {
            403 => LeaseApiError::Forbidden,
            404 => LeaseApiError::NotFound,
            409 if response.reason == "AlreadyExists" => LeaseApiError::AlreadyExists,
            409 => LeaseApiError::Conflict,
            _ => LeaseApiError::Api,
        },
        _ => LeaseApiError::Transport,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ClaimOutcome {
    Holder {
        lease_transitions: u64,
    },
    Follower {
        holder: Option<String>,
        lease_transitions: u64,
    },
}

/// Start the contender loop and return a watch channel that is true only while
/// this process has confirmed Lease ownership. The first acquisition attempt is
/// immediate; subsequent acquisition and renewal attempts use the configured
/// retry period.
pub fn spawn_leader_election(
    client: kube::Client,
    namespace: &str,
    config: LeaderElectionConfig,
    recovery: RecoveryMonitor,
) -> Result<watch::Receiver<bool>, AppError> {
    let identity = config.identity.clone().ok_or_else(|| {
        AppError::Config("FKST_LEADER_IDENTITY must be set when leader election starts".to_string())
    })?;
    recovery.enable_leader_election(identity.clone());

    let api: Arc<dyn LeaseApi> = Arc::new(KubernetesLeaseApi::new(client, namespace));
    let (sender, receiver) = watch::channel(false);
    tokio::spawn(run_election_loop(api, config, recovery, sender));
    Ok(receiver)
}

/// Start and stop one workload future per observed leadership lifetime. Loss is
/// processed serially: `on_follower` runs first, the current token is cancelled,
/// and that generation is fully joined before a later acquisition may start.
/// This makes overlapping worker generations impossible within one process.
pub async fn supervise_leader_generations<Start, Work, OnFollower>(
    mut leadership: watch::Receiver<bool>,
    mut on_follower: OnFollower,
    mut start_generation: Start,
) where
    Start: FnMut(CancellationToken) -> Work,
    Work: Future<Output = ()> + Send + 'static,
    OnFollower: FnMut(),
{
    let mut worker: Option<(CancellationToken, tokio::task::JoinHandle<()>)> = None;
    loop {
        let is_leader = *leadership.borrow_and_update();
        if is_leader && worker.is_none() {
            let cancellation = CancellationToken::new();
            let task = tokio::spawn(start_generation(cancellation.clone()));
            worker = Some((cancellation, task));
        } else if !is_leader {
            on_follower();
            if let Some((cancellation, task)) = worker.take() {
                cancellation.cancel();
                let _ = task.await;
            }
        }

        if leadership.changed().await.is_err() {
            on_follower();
            if let Some((cancellation, task)) = worker.take() {
                cancellation.cancel();
                let _ = task.await;
            }
            return;
        }
    }
}

async fn run_election_loop(
    api: Arc<dyn LeaseApi>,
    config: LeaderElectionConfig,
    recovery: RecoveryMonitor,
    sender: watch::Sender<bool>,
) {
    let identity = config
        .identity
        .as_deref()
        .expect("validated leader identity")
        .to_string();
    let retry_period = Duration::from_secs(config.retry_period_secs);
    let renew_deadline = Duration::from_secs(config.renew_deadline_secs);
    let mut state = ElectionState::default();

    tracing::info!(
        lease = %config.lease_name,
        identity = %identity,
        lease_duration_secs = config.lease_duration_secs,
        renew_deadline_secs = config.renew_deadline_secs,
        retry_period_secs = config.retry_period_secs,
        "leader election: contender started"
    );

    loop {
        let attempt_started = Instant::now();
        // A wedged API request must not let a former leader run past its renew
        // deadline. Bound each operation by one retry period, then feed the
        // timeout through the same failure/deadline state machine.
        let result =
            match tokio::time::timeout(retry_period, claim_once(api.as_ref(), &config, Utc::now()))
                .await
            {
                Ok(result) => result,
                Err(_) => Err(LeaseApiError::Transport),
            };
        let monotonic_now = Instant::now();
        match state.observe(monotonic_now, renew_deadline, result) {
            ElectionEvent::Acquired { lease_transitions } => {
                recovery.record_leader_acquired(lease_transitions);
                tracing::info!(
                    lease = %config.lease_name,
                    identity = %identity,
                    lease_transitions,
                    "leader election: leadership acquired"
                );
                if sender.send(true).is_err() {
                    return;
                }
            }
            ElectionEvent::Renewed { lease_transitions } => {
                recovery.record_leader_renewed(lease_transitions);
                tracing::debug!(
                    lease = %config.lease_name,
                    identity = %identity,
                    lease_transitions,
                    "leader election: Lease renewed"
                );
            }
            ElectionEvent::Following {
                holder,
                lease_transitions,
            } => {
                recovery.record_leader_follower(holder.clone(), lease_transitions);
                tracing::debug!(
                    lease = %config.lease_name,
                    identity = %identity,
                    holder = holder.as_deref().unwrap_or(""),
                    lease_transitions,
                    "leader election: following current holder"
                );
            }
            ElectionEvent::Lost {
                holder,
                lease_transitions,
            } => {
                recovery.record_leader_lost(holder.clone(), lease_transitions);
                tracing::warn!(
                    lease = %config.lease_name,
                    identity = %identity,
                    holder = holder.as_deref().unwrap_or(""),
                    lease_transitions,
                    "leader election: leadership lost; cancelling worker generation"
                );
                if sender.send(false).is_err() {
                    return;
                }
            }
            ElectionEvent::Conflict { renewal, lost } => {
                recovery.record_leader_conflict();
                if renewal {
                    recovery.record_leader_api_failure(true);
                }
                tracing::warn!(
                    lease = %config.lease_name,
                    identity = %identity,
                    renewal,
                    lost,
                    "leader election: optimistic Lease conflict"
                );
                if lost {
                    recovery.record_leader_lost(None, state.lease_transitions);
                    if sender.send(false).is_err() {
                        return;
                    }
                }
            }
            ElectionEvent::Failure {
                error,
                renewal,
                lost,
            } => {
                recovery.record_leader_api_failure(renewal);
                tracing::warn!(
                    lease = %config.lease_name,
                    identity = %identity,
                    error = %error,
                    renewal,
                    lost,
                    "leader election: Lease operation failed"
                );
                if lost {
                    recovery.record_leader_lost(None, state.lease_transitions);
                    if sender.send(false).is_err() {
                        return;
                    }
                }
            }
        }
        tokio::time::sleep_until(attempt_started + retry_period).await;
    }
}

#[derive(Default)]
struct ElectionState {
    leader: bool,
    last_successful_renewal: Option<Instant>,
    lease_transitions: u64,
}

enum ElectionEvent {
    Acquired {
        lease_transitions: u64,
    },
    Renewed {
        lease_transitions: u64,
    },
    Following {
        holder: Option<String>,
        lease_transitions: u64,
    },
    Lost {
        holder: Option<String>,
        lease_transitions: u64,
    },
    Conflict {
        renewal: bool,
        lost: bool,
    },
    Failure {
        error: LeaseApiError,
        renewal: bool,
        lost: bool,
    },
}

impl ElectionState {
    fn observe(
        &mut self,
        now: Instant,
        renew_deadline: Duration,
        result: Result<ClaimOutcome, LeaseApiError>,
    ) -> ElectionEvent {
        match result {
            Ok(ClaimOutcome::Holder { lease_transitions }) => {
                let acquired = !self.leader;
                self.leader = true;
                self.last_successful_renewal = Some(now);
                self.lease_transitions = lease_transitions;
                if acquired {
                    ElectionEvent::Acquired { lease_transitions }
                } else {
                    ElectionEvent::Renewed { lease_transitions }
                }
            }
            Ok(ClaimOutcome::Follower {
                holder,
                lease_transitions,
            }) => {
                let lost = self.leader;
                self.leader = false;
                self.last_successful_renewal = None;
                self.lease_transitions = lease_transitions;
                if lost {
                    ElectionEvent::Lost {
                        holder,
                        lease_transitions,
                    }
                } else {
                    ElectionEvent::Following {
                        holder,
                        lease_transitions,
                    }
                }
            }
            Err(error) => {
                let renewal = self.leader;
                let lost = renewal
                    && self
                        .last_successful_renewal
                        .is_none_or(|last| now.duration_since(last) >= renew_deadline);
                if lost {
                    self.leader = false;
                    self.last_successful_renewal = None;
                }
                if error == LeaseApiError::Conflict {
                    ElectionEvent::Conflict { renewal, lost }
                } else {
                    ElectionEvent::Failure {
                        error,
                        renewal,
                        lost,
                    }
                }
            }
        }
    }
}

async fn claim_once(
    api: &dyn LeaseApi,
    config: &LeaderElectionConfig,
    now: DateTime<Utc>,
) -> Result<ClaimOutcome, LeaseApiError> {
    let identity = config
        .identity
        .as_deref()
        .ok_or(LeaseApiError::InvalidObject)?;
    let mut lease = match api.get(&config.lease_name).await {
        Ok(lease) => lease,
        Err(LeaseApiError::NotFound) => {
            let lease = new_lease(config, identity, now);
            return match api.create(&lease).await {
                Ok(created) => Ok(ClaimOutcome::Holder {
                    lease_transitions: transitions(&created),
                }),
                Err(LeaseApiError::AlreadyExists | LeaseApiError::Conflict) => {
                    Err(LeaseApiError::Conflict)
                }
                Err(error) => Err(error),
            };
        }
        Err(error) => return Err(error),
    };

    let spec = lease.spec.clone().unwrap_or_default();
    let current_holder = spec
        .holder_identity
        .as_deref()
        .map(str::trim)
        .filter(|holder| !holder.is_empty())
        .map(str::to_string);
    let lease_transitions = transitions(&lease);
    if current_holder.as_deref() != Some(identity) && !lease_expired(&spec, now) {
        return Ok(ClaimOutcome::Follower {
            holder: current_holder,
            lease_transitions,
        });
    }

    if lease.metadata.resource_version.is_none() {
        return Err(LeaseApiError::InvalidObject);
    }
    let holder_changed = current_holder.as_deref() != Some(identity);
    let mut desired = spec;
    if holder_changed {
        desired.acquire_time = Some(MicroTime(now));
        desired.lease_transitions = Some(
            desired
                .lease_transitions
                .unwrap_or_default()
                .saturating_add(1),
        );
    } else if desired.acquire_time.is_none() {
        desired.acquire_time = Some(MicroTime(now));
    }
    desired.holder_identity = Some(identity.to_string());
    desired.lease_duration_seconds = Some(config.lease_duration_secs as i32);
    desired.renew_time = Some(MicroTime(now));
    lease.spec = Some(desired);

    let replaced = api.replace(&config.lease_name, &lease).await?;
    let replaced_holder = replaced
        .spec
        .as_ref()
        .and_then(|spec| spec.holder_identity.as_deref());
    if replaced_holder != Some(identity) {
        return Err(LeaseApiError::InvalidObject);
    }
    Ok(ClaimOutcome::Holder {
        lease_transitions: transitions(&replaced),
    })
}

fn new_lease(config: &LeaderElectionConfig, identity: &str, now: DateTime<Utc>) -> Lease {
    Lease {
        metadata: kube::api::ObjectMeta {
            name: Some(config.lease_name.clone()),
            ..Default::default()
        },
        spec: Some(LeaseSpec {
            acquire_time: Some(MicroTime(now)),
            holder_identity: Some(identity.to_string()),
            lease_duration_seconds: Some(config.lease_duration_secs as i32),
            lease_transitions: Some(0),
            renew_time: Some(MicroTime(now)),
            strategy: None,
            preferred_holder: None,
        }),
    }
}

fn transitions(lease: &Lease) -> u64 {
    lease
        .spec
        .as_ref()
        .and_then(|spec| spec.lease_transitions)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or_default()
}

fn lease_expired(spec: &LeaseSpec, now: DateTime<Utc>) -> bool {
    let Some(holder) = spec
        .holder_identity
        .as_deref()
        .map(str::trim)
        .filter(|holder| !holder.is_empty())
    else {
        return true;
    };
    let _ = holder;
    let Some(duration) = spec.lease_duration_seconds.filter(|duration| *duration > 0) else {
        return true;
    };
    let Some(base) = spec
        .renew_time
        .as_ref()
        .or(spec.acquire_time.as_ref())
        .map(|time| time.0)
    else {
        return true;
    };
    now >= base + k8s_openapi::chrono::Duration::seconds(i64::from(duration))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use k8s_openapi::chrono::TimeZone;

    use super::*;

    #[derive(Default)]
    struct FakeState {
        lease: Option<Lease>,
        next_resource_version: u64,
        create_calls: usize,
        replace_calls: usize,
        conflict_next_replace: bool,
    }

    #[derive(Default)]
    struct FakeLeaseApi {
        state: Mutex<FakeState>,
    }

    impl FakeLeaseApi {
        fn with_lease(lease: Lease) -> Self {
            Self {
                state: Mutex::new(FakeState {
                    lease: Some(lease),
                    next_resource_version: 2,
                    ..FakeState::default()
                }),
            }
        }

        fn lease(&self) -> Lease {
            self.state
                .lock()
                .expect("state")
                .lease
                .clone()
                .expect("lease")
        }

        fn conflict_next_replace(&self) {
            self.state.lock().expect("state").conflict_next_replace = true;
        }
    }

    #[async_trait]
    impl LeaseApi for FakeLeaseApi {
        async fn get(&self, _name: &str) -> Result<Lease, LeaseApiError> {
            self.state
                .lock()
                .expect("state")
                .lease
                .clone()
                .ok_or(LeaseApiError::NotFound)
        }

        async fn create(&self, lease: &Lease) -> Result<Lease, LeaseApiError> {
            let mut state = self.state.lock().expect("state");
            state.create_calls += 1;
            if state.lease.is_some() {
                return Err(LeaseApiError::AlreadyExists);
            }
            state.next_resource_version += 1;
            let mut created = lease.clone();
            created.metadata.resource_version = Some(state.next_resource_version.to_string());
            state.lease = Some(created.clone());
            Ok(created)
        }

        async fn replace(&self, _name: &str, lease: &Lease) -> Result<Lease, LeaseApiError> {
            let mut state = self.state.lock().expect("state");
            state.replace_calls += 1;
            if state.conflict_next_replace {
                state.conflict_next_replace = false;
                return Err(LeaseApiError::Conflict);
            }
            let current = state.lease.as_ref().ok_or(LeaseApiError::NotFound)?;
            if current.metadata.resource_version != lease.metadata.resource_version {
                return Err(LeaseApiError::Conflict);
            }
            state.next_resource_version += 1;
            let mut replaced = lease.clone();
            replaced.metadata.resource_version = Some(state.next_resource_version.to_string());
            state.lease = Some(replaced.clone());
            Ok(replaced)
        }
    }

    fn config(identity: &str) -> LeaderElectionConfig {
        LeaderElectionConfig {
            enabled: true,
            identity: Some(identity.to_string()),
            ..LeaderElectionConfig::default()
        }
    }

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).single().expect("time")
    }

    fn held_lease(holder: &str, renewed: DateTime<Utc>, transitions: i32) -> Lease {
        let mut lease = new_lease(&config(holder), holder, renewed);
        lease.metadata.resource_version = Some("1".to_string());
        lease.spec.as_mut().expect("spec").lease_transitions = Some(transitions);
        lease
    }

    #[tokio::test]
    async fn missing_lease_is_created_and_acquired() {
        let api = FakeLeaseApi::default();
        let outcome = claim_once(&api, &config("pod-a"), at(100))
            .await
            .expect("claim");
        assert_eq!(
            outcome,
            ClaimOutcome::Holder {
                lease_transitions: 0
            }
        );
        let lease = api.lease();
        assert_eq!(
            lease.spec.expect("spec").holder_identity.as_deref(),
            Some("pod-a")
        );
        assert_eq!(api.state.lock().expect("state").create_calls, 1);
    }

    #[tokio::test]
    async fn unexpired_holder_makes_another_contender_follow() {
        let api = FakeLeaseApi::with_lease(held_lease("pod-a", at(100), 4));
        let outcome = claim_once(&api, &config("pod-b"), at(110))
            .await
            .expect("follow");
        assert_eq!(
            outcome,
            ClaimOutcome::Follower {
                holder: Some("pod-a".to_string()),
                lease_transitions: 4,
            }
        );
        assert_eq!(api.state.lock().expect("state").replace_calls, 0);
    }

    #[tokio::test]
    async fn holder_renews_with_resource_version_and_preserves_acquisition() {
        let api = FakeLeaseApi::with_lease(held_lease("pod-a", at(100), 4));
        claim_once(&api, &config("pod-a"), at(110))
            .await
            .expect("renew");
        let lease = api.lease();
        assert_eq!(lease.metadata.resource_version.as_deref(), Some("3"));
        let spec = lease.spec.expect("spec");
        assert_eq!(spec.acquire_time.expect("acquire").0, at(100));
        assert_eq!(spec.renew_time.expect("renew").0, at(110));
        assert_eq!(spec.lease_transitions, Some(4));
    }

    #[tokio::test]
    async fn expired_lease_is_taken_over_in_one_cas_and_increments_transition() {
        let api = FakeLeaseApi::with_lease(held_lease("pod-a", at(100), 4));
        let outcome = claim_once(&api, &config("pod-b"), at(131))
            .await
            .expect("takeover");
        assert_eq!(
            outcome,
            ClaimOutcome::Holder {
                lease_transitions: 5
            }
        );
        let lease = api.lease();
        let spec = lease.spec.expect("spec");
        assert_eq!(spec.holder_identity.as_deref(), Some("pod-b"));
        assert_eq!(spec.acquire_time.expect("acquire").0, at(131));
        assert_eq!(api.state.lock().expect("state").replace_calls, 1);
    }

    #[tokio::test]
    async fn stale_replace_conflict_never_reports_ownership() {
        let api = FakeLeaseApi::with_lease(held_lease("pod-a", at(100), 4));
        api.conflict_next_replace();
        let error = claim_once(&api, &config("pod-b"), at(131))
            .await
            .expect_err("conflict");
        assert_eq!(error, LeaseApiError::Conflict);
        assert_eq!(
            api.lease().spec.expect("spec").holder_identity.as_deref(),
            Some("pod-a")
        );
    }

    #[test]
    fn renewal_failures_crossing_deadline_cancel_and_allow_reacquisition() {
        let mut state = ElectionState::default();
        let start = Instant::now();
        assert!(matches!(
            state.observe(
                start,
                Duration::from_secs(20),
                Ok(ClaimOutcome::Holder {
                    lease_transitions: 1
                })
            ),
            ElectionEvent::Acquired { .. }
        ));
        assert!(matches!(
            state.observe(
                start + Duration::from_secs(19),
                Duration::from_secs(20),
                Err(LeaseApiError::Transport)
            ),
            ElectionEvent::Failure { lost: false, .. }
        ));
        assert!(matches!(
            state.observe(
                start + Duration::from_secs(20),
                Duration::from_secs(20),
                Err(LeaseApiError::Transport)
            ),
            ElectionEvent::Failure { lost: true, .. }
        ));
        assert!(!state.leader);
        assert!(matches!(
            state.observe(
                start + Duration::from_secs(21),
                Duration::from_secs(20),
                Ok(ClaimOutcome::Holder {
                    lease_transitions: 1
                })
            ),
            ElectionEvent::Acquired { .. }
        ));
    }

    #[tokio::test]
    async fn supervisor_cancels_and_joins_before_reacquisition() {
        let (sender, receiver) = watch::channel(false);
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let starts = Arc::new(AtomicUsize::new(0));
        let follower_callbacks = Arc::new(AtomicUsize::new(0));

        let work_active = active.clone();
        let work_maximum = maximum.clone();
        let work_starts = starts.clone();
        let callback_count = follower_callbacks.clone();
        let task = tokio::spawn(supervise_leader_generations(
            receiver,
            move || {
                callback_count.fetch_add(1, Ordering::SeqCst);
            },
            move |cancellation| {
                let active = work_active.clone();
                let maximum = work_maximum.clone();
                let starts = work_starts.clone();
                async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(current, Ordering::SeqCst);
                    starts.fetch_add(1, Ordering::SeqCst);
                    cancellation.cancelled().await;
                    active.fetch_sub(1, Ordering::SeqCst);
                }
            },
        ));

        sender.send(true).expect("acquire");
        wait_for(|| starts.load(Ordering::SeqCst) == 1).await;
        sender.send(false).expect("loss");
        wait_for(|| active.load(Ordering::SeqCst) == 0).await;
        sender.send(true).expect("reacquire");
        wait_for(|| starts.load(Ordering::SeqCst) == 2).await;
        drop(sender);
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("supervisor stops")
            .expect("supervisor task");

        assert_eq!(maximum.load(Ordering::SeqCst), 1);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert!(follower_callbacks.load(Ordering::SeqCst) >= 2);
    }

    async fn wait_for(predicate: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while !predicate() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("condition");
    }
}
