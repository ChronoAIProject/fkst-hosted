//! The log-bundle half of the canary harness: a REAL gzipped tar bundle, served
//! by a mock chrono-storage, for a session the canary caller is authorized to
//! read.
//!
//! The spec names "log path/content" as a hostile location, and the two halves
//! are genuinely different. The PATH is caller-supplied text that arrives in the
//! query string. The CONTENT arrives from storage, is decompressed, tailed, and
//! written into the response — which is exactly the kind of value that reaches a
//! record by accident. Proving it does not requires a deployment where a log
//! read actually succeeds: with `storage: None` every log route short-circuits
//! before a bundle is ever read, so a canary planted in a bundle would be a
//! canary nothing ever looked at.

use std::sync::Arc;

use fkst_control_plane::models::RepoRef;
use fkst_control_plane::reconcile::creator::SessionCreator;
use fkst_control_plane::session_access::{
    SessionAccessContext, SessionAccessRegistry, SessionAccessState,
};
use fkst_control_plane::storage::{ChronoStorageClient, ChronoStorageConfig};
use flate2::write::GzEncoder;
use flate2::Compression;
use secrecy::SecretString;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The session whose bundle the harness serves.
pub const SESSION: &str = "8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e";
/// A path that really exists in that bundle, so a file read returns content
/// rather than 404.
pub const FILE_PATH: &str = "fkst-substrate/codex/codex.log";
/// The canary the served file contains, verbatim.
pub const FILE_CONTENT: &str = "canary-log-content";
/// The App installation the session belongs to (correlation only — the log gate
/// authorizes on the creator, not on repository visibility).
const INSTALLATION_ID: i64 = 7;

fn append(builder: &mut tar::Builder<&mut GzEncoder<Vec<u8>>>, name: &str, data: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, name, data)
        .expect("append bundle entry");
}

/// A minimal redacted bundle in the collector's fixed layout, whose codex log is
/// the canary.
fn bundle() -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut builder = tar::Builder::new(&mut encoder);
        append(&mut builder, "fkst-hosted/driver.log", b"driver line\n");
        append(
            &mut builder,
            FILE_PATH,
            format!("first line\n{FILE_CONTENT}\n").as_bytes(),
        );
        append(&mut builder, "README.md", b"# readme\n");
        builder.finish().expect("finish tar");
    }
    encoder.finish().expect("finish gzip")
}

/// A chrono-storage mock that mints a token and serves the bundle at the
/// session's `latest` object key.
pub async fn storage() -> (Arc<ChronoStorageClient>, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "sa-token",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/buckets/logs/objects/download"))
        .and(query_param("key", format!("logs/{SESSION}/latest.tar.gz")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bundle()))
        .mount(&server)
        .await;
    let config = ChronoStorageConfig {
        base_url: server.uri(),
        bucket: "logs".to_string(),
        nyxid_token_url: format!("{}/oauth/token", server.uri()),
        nyxid_client_id: "sa-client".to_string(),
        nyxid_client_secret: SecretString::from("sa-secret".to_string()),
    };
    (
        Arc::new(ChronoStorageClient::new(reqwest::Client::new(), config)),
        server,
    )
}

/// A registry naming `login`/`id` as the session's creator, so the canary caller
/// passes the log gate and the read reaches the bundle.
pub fn access(login: &str, id: i64) -> SessionAccessState {
    let repo = RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    };
    let registry = SessionAccessRegistry::new(false);
    registry.replace_repo(
        INSTALLATION_ID,
        &repo,
        vec![(
            SESSION.to_string(),
            SessionAccessContext {
                installation_id: INSTALLATION_ID,
                repo: repo.clone(),
                trigger_issue: 9,
                creator: SessionCreator {
                    login: login.to_string(),
                    id: Some(id),
                },
                collaborators: Vec::new(),
                log_access: Vec::new(),
            },
        )],
    );
    SessionAccessState::new(registry)
}
