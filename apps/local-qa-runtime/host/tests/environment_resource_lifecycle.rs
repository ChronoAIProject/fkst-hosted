use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use fkst_local_qa_host::{
    check_environment_readiness, environment_status, stop_environment, CreateRequest,
    EnvironmentProvider, EnvironmentStatus, EnvironmentStopReceipt, FixedClock, OwnedHandle,
    ProviderReadinessReceipt, ProviderResource, ProviderResourceState, ProviderStatusReceipt,
    ProviderStopReceipt, ReadinessEndpointClass, ReadinessRequest, RunError,
};

const NOW: &str = "2026-09-10T00:00:00Z";
const READINESS_DEADLINE: &str = "2026-09-10T00:00:30Z";
const ENVIRONMENT_DEADLINE: &str = "2026-09-10T00:01:00Z";

struct LifecycleProvider {
    resource: ProviderResource,
    state: ProviderResourceState,
    readiness: ProviderReadinessReceipt,
    stop_calls: usize,
}

impl EnvironmentProvider for LifecycleProvider {
    fn discover(
        &mut self,
        _stable_provider_key: &str,
    ) -> Result<Option<ProviderResource>, RunError> {
        Ok(Some(self.resource.clone()))
    }

    fn create(&mut self, _request: CreateRequest) -> Result<ProviderResource, RunError> {
        panic!("lifecycle hook tests must not create resources")
    }

    fn status(&mut self, _resource: &ProviderResource) -> Result<ProviderStatusReceipt, RunError> {
        Ok(ProviderStatusReceipt {
            resource: self.resource.clone(),
            state: self.state,
        })
    }

    fn stop(&mut self, _resource: &ProviderResource) -> Result<ProviderStopReceipt, RunError> {
        self.stop_calls += 1;
        self.state = ProviderResourceState::Stopped;
        Ok(ProviderStopReceipt {
            resource: self.resource.clone(),
            stopped: true,
        })
    }

    fn readiness(
        &mut self,
        _resource: &ProviderResource,
        _request: &ReadinessRequest,
    ) -> Result<ProviderReadinessReceipt, RunError> {
        Ok(self.readiness.clone())
    }
}

#[test]
fn typed_readiness_and_repeated_status_stop_preserve_exact_identity() {
    let handle = owned_handle();
    let resource = provider_resource(&handle);
    let mut provider = LifecycleProvider {
        resource: resource.clone(),
        state: ProviderResourceState::Active,
        readiness: readiness_receipt(resource),
        stop_calls: 0,
    };
    assert_eq!(
        environment_status(&mut provider, &handle).unwrap(),
        EnvironmentStatus::Active
    );

    let request = readiness_request();
    let receipt = check_environment_readiness(
        &mut provider,
        &handle,
        &request,
        &FixedClock::new(NOW).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt.intent_id, handle.intent_id);
    assert_eq!(receipt.provider_identity, handle.provider_identity);
    assert_eq!(receipt.service_id, "app");
    assert_eq!(receipt.endpoint_class, ReadinessEndpointClass::LoopbackHttp);
    assert_eq!(receipt.endpoint.port(), 43123);
    assert_eq!(receipt.attempts, 3);
    assert_eq!(receipt.elapsed_ms, 1200);

    assert_eq!(
        stop_environment(&mut provider, &handle).unwrap(),
        EnvironmentStopReceipt {
            intent_id: handle.intent_id.clone(),
            provider_identity: handle.provider_identity.clone(),
            already_stopped: false,
        }
    );
    assert_eq!(provider.stop_calls, 1);
    assert_eq!(
        environment_status(&mut provider, &handle).unwrap(),
        EnvironmentStatus::Stopped
    );
    assert!(
        stop_environment(&mut provider, &handle)
            .unwrap()
            .already_stopped
    );
    assert_eq!(provider.stop_calls, 1);
}

#[test]
fn readiness_identity_endpoint_and_budget_mismatches_fail_closed() {
    let handle = owned_handle();
    let resource = provider_resource(&handle);
    let request = readiness_request();
    let clock = FixedClock::new(NOW).unwrap();

    let mut provider = LifecycleProvider {
        resource: resource.clone(),
        state: ProviderResourceState::Active,
        readiness: readiness_receipt(resource.clone()),
        stop_calls: 0,
    };
    provider.readiness.resource.provider_identity = "other-provider".to_owned();
    assert!(check_environment_readiness(&mut provider, &handle, &request, &clock).is_err());

    provider.readiness = readiness_receipt(resource.clone());
    provider.readiness.endpoint = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)), 43123);
    assert!(check_environment_readiness(&mut provider, &handle, &request, &clock).is_err());

    provider.readiness = readiness_receipt(resource.clone());
    provider.readiness.attempts = request.max_attempts + 1;
    assert!(check_environment_readiness(&mut provider, &handle, &request, &clock).is_err());

    provider.readiness = readiness_receipt(resource);
    provider.readiness.ready = false;
    assert!(check_environment_readiness(&mut provider, &handle, &request, &clock).is_err());
}

#[test]
fn unknown_or_conflicting_environment_ownership_blocks_status_and_stop() {
    let handle = owned_handle();
    let resource = provider_resource(&handle);
    let mut provider = LifecycleProvider {
        resource: resource.clone(),
        state: ProviderResourceState::Unknown,
        readiness: readiness_receipt(resource),
        stop_calls: 0,
    };
    assert!(environment_status(&mut provider, &handle).is_err());
    assert!(stop_environment(&mut provider, &handle).is_err());
    assert_eq!(provider.stop_calls, 0);

    provider.state = ProviderResourceState::Active;
    provider.resource.stable_provider_key = "fkst-local-qa/environment/v1/unrelated".to_owned();
    assert!(environment_status(&mut provider, &handle).is_err());
    assert!(stop_environment(&mut provider, &handle).is_err());
    assert_eq!(provider.stop_calls, 0);
}

#[test]
fn deadline_regression_readiness_fractional_observations_and_environment_ceiling() {
    for (environment, deadline, observed, allowed) in [
        (
            ENVIRONMENT_DEADLINE,
            "2026-09-10T00:00:02.5Z",
            "2026-09-10T00:00:02Z",
            true,
        ),
        (
            ENVIRONMENT_DEADLINE,
            "2026-09-10T00:00:02Z",
            "2026-09-10T00:00:02.5Z",
            false,
        ),
        (
            ENVIRONMENT_DEADLINE,
            READINESS_DEADLINE,
            READINESS_DEADLINE,
            false,
        ),
        (
            "2026-09-10T00:00:30.5Z",
            READINESS_DEADLINE,
            "2026-09-10T00:00:02Z",
            true,
        ),
        (
            READINESS_DEADLINE,
            "2026-09-10T00:00:30.5Z",
            "2026-09-10T00:00:02Z",
            false,
        ),
        (
            READINESS_DEADLINE,
            READINESS_DEADLINE,
            "2026-09-10T00:00:02Z",
            true,
        ),
        ("short", READINESS_DEADLINE, "2026-09-10T00:00:02Z", false),
        (ENVIRONMENT_DEADLINE, READINESS_DEADLINE, "☃", false),
        (
            ENVIRONMENT_DEADLINE,
            READINESS_DEADLINE,
            "2026-09-10T00:00:02.0Z",
            false,
        ),
    ] {
        let mut handle = owned_handle();
        handle.deadline_utc = environment.to_owned();
        let resource = provider_resource(&handle);
        let mut provider = LifecycleProvider {
            resource: resource.clone(),
            state: ProviderResourceState::Active,
            readiness: readiness_receipt(resource),
            stop_calls: 0,
        };
        provider.readiness.observed_at_utc = observed.to_owned();
        let mut request = readiness_request();
        request.deadline_utc = deadline.to_owned();
        let result = check_environment_readiness(
            &mut provider,
            &handle,
            &request,
            &FixedClock::new(NOW).unwrap(),
        );
        assert_eq!(
            result.is_ok(),
            allowed,
            "environment={environment}, deadline={deadline}, observed={observed}: {result:?}"
        );
    }
}

#[test]
fn deadline_regression_readiness_observes_host_clock_after_callback() {
    struct ReturnClock(std::cell::Cell<bool>, &'static str);
    impl fkst_local_qa_host::Clock for ReturnClock {
        fn now_utc(&self) -> Result<String, RunError> {
            Ok(if self.0.replace(true) { self.1 } else { NOW }.to_owned())
        }
    }
    for returned_at in [READINESS_DEADLINE, "2026-09-10T00:00:30.5Z", "malformed"] {
        let handle = owned_handle();
        let resource = provider_resource(&handle);
        let mut provider = LifecycleProvider {
            resource: resource.clone(),
            state: ProviderResourceState::Active,
            readiness: readiness_receipt(resource),
            stop_calls: 0,
        };
        let result = check_environment_readiness(
            &mut provider,
            &handle,
            &readiness_request(),
            &ReturnClock(std::cell::Cell::new(false), returned_at),
        );
        assert!(
            result.is_err(),
            "late/invalid Host observation {returned_at}: {result:?}"
        );
        assert_eq!(
            environment_status(&mut provider, &handle).unwrap(),
            EnvironmentStatus::Active
        );
        stop_environment(&mut provider, &handle).unwrap();
        assert_eq!(provider.stop_calls, 1);
    }
}

fn owned_handle() -> OwnedHandle {
    OwnedHandle {
        intent_id: "intent-env-001".to_owned(),
        run_id: "00000000-0000-4000-8000-000000000301".to_owned(),
        profile_id: "profile-001".to_owned(),
        environment_id: "environment-001".to_owned(),
        generation: 1,
        deadline_utc: ENVIRONMENT_DEADLINE.to_owned(),
        stable_provider_key: "fkst-local-qa/environment/v1/intent-env-001".to_owned(),
        provider_identity: "provider-env-001".to_owned(),
        state: "active".to_owned(),
    }
}

fn provider_resource(handle: &OwnedHandle) -> ProviderResource {
    ProviderResource {
        stable_provider_key: handle.stable_provider_key.clone(),
        labels: BTreeMap::from([
            ("fkst.local-qa/run-id".to_owned(), handle.run_id.clone()),
            (
                "fkst.local-qa/profile-id".to_owned(),
                handle.profile_id.clone(),
            ),
            (
                "fkst.local-qa/environment-id".to_owned(),
                handle.environment_id.clone(),
            ),
        ]),
        provider_identity: handle.provider_identity.clone(),
    }
}

fn readiness_request() -> ReadinessRequest {
    ReadinessRequest {
        service_id: "app".to_owned(),
        endpoint_class: ReadinessEndpointClass::LoopbackHttp,
        deadline_utc: READINESS_DEADLINE.to_owned(),
        max_attempts: 5,
        max_duration_ms: 5_000,
    }
}

fn readiness_receipt(resource: ProviderResource) -> ProviderReadinessReceipt {
    ProviderReadinessReceipt {
        resource,
        service_id: "app".to_owned(),
        endpoint_class: ReadinessEndpointClass::LoopbackHttp,
        endpoint: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 43123),
        observed_at_utc: "2026-09-10T00:00:02Z".to_owned(),
        attempts: 3,
        elapsed_ms: 1_200,
        ready: true,
    }
}
