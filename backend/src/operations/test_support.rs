//! Shared, credential-free fixtures for the activity-query unit tests.
//!
//! Nothing here needs a network, a cluster, or a token: a visibility constraint
//! is minted through the real sealed path (a verified viewer plus, where a
//! session is involved, a real policy decision), so a test can never assert
//! against a constraint the production code could not have produced.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use k8s_openapi::chrono::{DateTime, Duration, TimeZone, Utc};

use crate::access_policy::AccessPolicy;
use crate::github_identity::GithubUser;
use crate::session_access::policy::{
    decide, PolicyEnvironment, SessionAccessRequest, SessionCapability, VerifiedCaller,
};
use crate::session_access::test_support::{context, policy_with_admins};
use crate::session_access::{
    authorize_lifecycle_session, ActivityVisibilityConstraint, AuthenticatedViewer,
    AuthorizedSessionId, RequestedScope, ScopeRequest, ViewerScope,
};

use super::filters::TimeRange;
use super::record::{
    ActivityRecord, ActivitySourceKind, ApiRequestRecord, DeliveryState, RecordActor,
    RecordCorrelation, RecordPrincipal, SandboxLifecycleRecord,
};
use super::source::{ActivitySource, SourceError, SourcePage, SourceQuery};

/// A fixed instant every fixture is anchored to, so ordering assertions read as
/// arithmetic rather than as wall-clock luck.
pub(crate) fn anchor() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 31, 12, 0, 0)
        .single()
        .expect("a valid fixed instant")
}

/// `anchor() - 24h .. anchor()`, the endpoint's own default window.
pub(crate) fn range() -> TimeRange {
    TimeRange {
        from: anchor() - Duration::hours(24),
        to: anchor(),
    }
}

/// A verified viewer with the given id/login, admitted by an open policy.
pub(crate) fn viewer(id: i64, login: &str, access: &AccessPolicy) -> AuthenticatedViewer {
    AuthenticatedViewer::new(
        GithubUser {
            login: login.to_string(),
            id,
        },
        access,
    )
}

/// An open access policy naming `global_admins` (comma-separated logins/ids).
pub(crate) fn access(global_admins: &str) -> AccessPolicy {
    policy_with_admins(global_admins)
}

/// The personal scope of a regular verified viewer.
pub(crate) fn personal_scope(id: i64, login: &str) -> ViewerScope {
    let access = access("");
    viewer(id, login, &access)
        .resolve_scope(ScopeRequest::new(Some(RequestedScope::Personal)))
        .expect("a regular viewer always resolves the personal scope")
}

/// The global scope of a verified administrator.
pub(crate) fn global_scope(id: i64, login: &str) -> ViewerScope {
    let access = access(login);
    viewer(id, login, &access)
        .resolve_scope(ScopeRequest::new(Some(RequestedScope::Global)))
        .expect("an administrator resolves the global scope")
}

/// An authorized session token, minted through a REAL allowing policy decision so
/// the seal is exercised rather than bypassed.
pub(crate) fn authorized_session(
    session_id: &str,
    viewer_id: i64,
    viewer_login: &str,
) -> AuthorizedSessionId {
    let access = access("");
    let facts = context(Some(viewer_id), viewer_login, &[], &[]);
    let decision = decide(&SessionAccessRequest::new(
        SessionCapability::OperationsVisibility,
        VerifiedCaller::from_github_metadata(viewer_id, viewer_login),
        facts.facts(),
        PolicyEnvironment {
            access: &access,
            legacy_log_admins: &[],
            github_bot_login: None,
        },
    ));
    authorize_lifecycle_session(session_id, &decision)
        .expect("the session creator is authorized for operations visibility")
}

/// The personal constraint for a viewer, optionally carrying an authorized
/// lifecycle session.
pub(crate) fn mine(
    viewer_id: i64,
    viewer_login: &str,
    session: Option<AuthorizedSessionId>,
) -> ActivityVisibilityConstraint {
    ActivityVisibilityConstraint::for_scope(&personal_scope(viewer_id, viewer_login), session)
}

/// The global constraint for an administrator.
pub(crate) fn all(admin_id: i64, admin_login: &str) -> ActivityVisibilityConstraint {
    ActivityVisibilityConstraint::for_scope(&global_scope(admin_id, admin_login), None)
}

/// An API-request record at `offset` seconds before the anchor.
pub(crate) fn api_record(
    event_id: &str,
    actor_id: i64,
    offset_secs: i64,
    source: ActivitySourceKind,
) -> ActivityRecord {
    ActivityRecord::ApiRequest {
        record: Box::new(ApiRequestRecord {
            event_id: event_id.to_string(),
            request_id: Some(format!("req-{event_id}")),
            started_at: Some(anchor() - Duration::seconds(offset_secs + 1)),
            completed_at: anchor() - Duration::seconds(offset_secs),
            method: "GET".to_string(),
            route_template: "/api/v1/overview".to_string(),
            operation_id: "canvas_overview".to_string(),
            actor: RecordActor {
                kind: Some("github_user".to_string()),
                id: Some(actor_id),
                login: Some(format!("user-{actor_id}")),
            },
            principal: RecordPrincipal {
                kind: Some("github_user_token".to_string()),
                id: None,
            },
            arguments: serde_json::Map::new(),
            arguments_parse_status: Some("parsed".to_string()),
            status_code: Some(200),
            outcome: "success".to_string(),
            error_code: None,
            duration_ms: Some(12),
            correlation: RecordCorrelation::default(),
        }),
        delivery_state: DeliveryState::VerifiedInPosthog,
        source,
    }
}

/// A system lifecycle record for `session_id`.
pub(crate) fn lifecycle_record(
    event_id: &str,
    session_id: &str,
    offset_secs: i64,
    source: ActivitySourceKind,
) -> ActivityRecord {
    ActivityRecord::SandboxLifecycle {
        record: Box::new(SandboxLifecycleRecord {
            event_id: event_id.to_string(),
            occurred_at: anchor() - Duration::seconds(offset_secs),
            lifecycle_action: "created".to_string(),
            actor: RecordActor {
                kind: Some("system".to_string()),
                id: None,
                login: None,
            },
            principal: RecordPrincipal {
                kind: Some("reconciler".to_string()),
                id: Some("reconciler".to_string()),
            },
            session_id: session_id.to_string(),
            backend: Some("kubernetes".to_string()),
            runtime_id: Some(format!("fkst-sess-{session_id}")),
            creator_id: Some(101),
            creator_login: Some("alice".to_string()),
            trigger_author_id: Some(101),
            trigger_author_login: Some("alice".to_string()),
            created_at: Some(anchor() - Duration::seconds(offset_secs + 5)),
            reason_code: None,
            correlation: RecordCorrelation {
                session_id: Some(session_id.to_string()),
                ..RecordCorrelation::default()
            },
        }),
        delivery_state: DeliveryState::VerifiedInPosthog,
        source,
    }
}

/// A source that answers from a fixed script, recording the queries it received.
///
/// It is what proves the merge/partial semantics BEFORE the durable relay exists:
/// the same trait, the same typed constraint, a different implementation.
#[derive(Debug)]
pub(crate) struct FakeSource {
    kind: ActivitySourceKind,
    answer: Mutex<Option<Result<SourcePage, SourceError>>>,
    seen: Mutex<Vec<SourceQuery>>,
}

impl FakeSource {
    /// A source that returns `records` (with no dropped rows).
    pub(crate) fn ok(kind: ActivitySourceKind, records: Vec<ActivityRecord>) -> Arc<Self> {
        let raw_rows = records.len();
        Arc::new(Self {
            kind,
            answer: Mutex::new(Some(Ok(SourcePage {
                records,
                raw_rows,
                row_errors: 0,
            }))),
            seen: Mutex::new(Vec::new()),
        })
    }

    /// A source that fails.
    pub(crate) fn failing(kind: ActivitySourceKind, error: SourceError) -> Arc<Self> {
        Arc::new(Self {
            kind,
            answer: Mutex::new(Some(Err(error))),
            seen: Mutex::new(Vec::new()),
        })
    }

    /// Every query this source was asked. Empty proves it was never called.
    pub(crate) fn queries(&self) -> Vec<SourceQuery> {
        self.seen.lock().expect("fake source lock").clone()
    }
}

#[async_trait]
impl ActivitySource for FakeSource {
    fn kind(&self) -> ActivitySourceKind {
        self.kind
    }

    async fn fetch(&self, query: &SourceQuery) -> Result<SourcePage, SourceError> {
        self.seen
            .lock()
            .expect("fake source lock")
            .push(query.clone());
        match self.answer.lock().expect("fake source lock").take() {
            Some(Ok(page)) => Ok(page),
            Some(Err(error)) => Err(error),
            // A second call in one test would silently reuse a consumed script;
            // saying so loudly is better than an inexplicable empty page.
            None => Err(SourceError::Transient {
                kind: "fixture_exhausted",
            }),
        }
    }
}
