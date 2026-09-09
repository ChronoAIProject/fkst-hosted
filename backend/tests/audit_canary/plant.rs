//! The requests that plant the corpus, driven through the real router once.
//!
//! Separated from the harness next door because the two answer different
//! questions and change for different reasons: `mod.rs` owns the DEPLOYMENT the
//! canaries are planted into (its config, its mocks, its identities), while this
//! file owns WHERE each hostile value is put. A new hostile location is an edit
//! here alone, and a change to how the fixture deployment is wired is an edit
//! there alone.

use axum::body::Body;
use axum::http::Request;

use super::{log_bundle, sign, Canary, BOGUS_SIGNATURE};

/// A create-session body carrying a canary in every free-text location.
pub fn create_session_body() -> serde_json::Value {
    serde_json::json!({
        "name": "canary-session-name",
        "packages": ["acme/pkgs@main:packages/devloop"],
        "manifests": ["acme/pkgs@main:manifests/default.json"],
        "work_label": "fkst:work",
        "source_branch": "main",
        "target_branch": "fkst-hosted-default",
        "auto_merge": true,
        "log_access": ["grantee-one", "grantee-two"],
        "collaborators": ["collab-one"],
        "output_lang": "zh-CN",
        "disposable_environment": {
            "install": ["canary-install-command"],
            "variables": { "CANARY_VARIABLE_KEY": "canary-variable-value" },
            "secrets": { "CANARY_SECRET_KEY": "canary-secret-value" },
        },
    })
}

/// Drive every canary-bearing request through the real router once.
pub async fn plant_every_canary(canary: &Canary) {
    // Credentials in a header, a cookie, and a custom header, on a route whose
    // arguments are recorded before anything else runs.
    canary
        .call(canary.authenticated(Request::get("/api/v1/overview")))
        .await;

    // OAuth code, state, and GitHub's own error slug, all in the query.
    canary
        .call(
            Request::get(
                "/api/v1/auth/github/callback\
                 ?code=canary-oauth-code&state=canary-oauth-state&error=canary-oauth-error",
            )
            .body(Body::empty())
            .expect("request builds"),
        )
        .await;

    // An unrouted path with a credential-shaped query value.
    canary
        .call(
            Request::get("/api/v1/canary-unrouted-path?token=canary-query-token")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;

    // A refresh: the caller's refresh token IS the credential, and the exchange
    // returns a fresh access token plus a rotated refresh token. None of the
    // three may ever be recorded.
    canary
        .call(
            Request::post("/api/v1/auth/github/refresh")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "refresh_token": "canary-refresh-token" }).to_string(),
                ))
                .expect("request builds"),
        )
        .await;

    // A login callback whose code really is redeemed, so the minted pair lands
    // in this process rather than only in a mock's response body.
    canary
        .call(
            Request::get("/api/v1/auth/github/callback?code=canary-oauth-code&state=fkst")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;

    // A session observation. Its upstream answer is the canary-bearing error
    // body, so the observe projection is exercised on the path where an upstream
    // payload could most easily become an audit property.
    canary
        .call(canary.authenticated(Request::get(format!(
            "/api/v1/sessions/{}/observe",
            log_bundle::SESSION
        ))))
        .await;

    // A repository description.
    canary
        .call(canary.authenticated_json(
            Request::post("/api/v1/repos"),
            serde_json::json!({
                "owner": null,
                "name": "site",
                "private": true,
                "description": "canary-repository-description",
            }),
        ))
        .await;

    // A session name plus disposable-environment keys, values, and commands.
    canary
        .call(canary.authenticated_json(
            Request::post("/api/v1/repos/acme/site/sessions"),
            create_session_body(),
        ))
        .await;

    // A branch value that only the ERROR MESSAGE quotes back.
    let mut invalid_branch = create_session_body();
    invalid_branch["source_branch"] = serde_json::json!("canary-invalid-branch name");
    canary
        .call(canary.authenticated_json(
            Request::post("/api/v1/repos/acme/site/sessions"),
            invalid_branch,
        ))
        .await;

    // A package reference that cannot parse. Kept as its OWN request rather than
    // added to the shared body: an unparseable package makes the whole DTO
    // unavailable, which would silently remove the safe-argument assertions the
    // valid create-session request exists to make.
    let mut invalid_package = create_session_body();
    invalid_package["packages"] = serde_json::json!(["canary-invalid-package-ref"]);
    canary
        .call(canary.authenticated_json(
            Request::post("/api/v1/repos/acme/site/sessions"),
            invalid_package,
        ))
        .await;

    // A work item's title and body.
    canary
        .call(canary.authenticated_json(
            Request::post("/api/v1/repos/acme/site/sessions/42/work-items"),
            serde_json::json!({
                "title": "canary-work-item-title",
                "body": "canary-work-item-body",
                "label": "fkst:work",
            }),
        ))
        .await;

    // Install commands, variable keys/values, and secret keys/values.
    canary
        .call(canary.authenticated_json(
            Request::put("/api/v1/users/me/environment-profiles/node-20"),
            serde_json::json!({
                "install": ["canary-install-command"],
                "variables": { "CANARY_VARIABLE_KEY": "canary-variable-value" },
                "secrets": { "CANARY_SECRET_KEY": "canary-secret-value" },
            }),
        ))
        .await;

    // A requested log PATH (the archive is matched on it, so an unmatched one is
    // a probe string).
    let session = log_bundle::SESSION;
    canary
        .call(canary.authenticated(Request::get(format!(
            "/api/v1/logs/{session}/file?path=canary-log-path&tail_bytes=4096"
        ))))
        .await;

    // A log file that really EXISTS: the bundle is fetched, decompressed, and
    // its canary-bearing content written into the response.
    canary
        .call(canary.authenticated(Request::get(format!(
            "/api/v1/logs/{session}/file?path={}&tail_bytes=4096",
            log_bundle::FILE_PATH
        ))))
        .await;

    // …and the whole bundle, streamed as an attachment carrying the same bytes.
    canary
        .call(canary.authenticated(Request::get(format!("/api/v1/logs/{session}"))))
        .await;

    // A rejected webhook delivery: everything it claims is attacker-controlled.
    canary
        .call(
            Request::post("/api/v1/github/app/webhook")
                .header("x-hub-signature-256", BOGUS_SIGNATURE)
                .header("x-github-event", "issues")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "action": "opened",
                        "issue": { "number": 9, "title": "canary-issue-title",
                                   "body": "canary-issue-body" },
                        "repository": { "owner": { "login": "acme" }, "name": "site" },
                        "installation": { "id": 7 },
                        "sender": { "login": "canary-webhook-signature", "id": 1 },
                    })
                    .to_string(),
                ))
                .expect("request builds"),
        )
        .await;

    // A VERIFIED delivery whose payload still carries issue free text.
    let verified = serde_json::json!({
        "action": "opened",
        "issue": { "number": 9, "title": "canary-issue-title", "body": "canary-issue-body" },
        "repository": { "owner": { "login": "acme" }, "name": "site" },
        "installation": { "id": 7 },
        "sender": { "login": "octocat", "id": 583_231 },
    })
    .to_string();
    canary
        .call(
            Request::post("/api/v1/github/app/webhook")
                .header("x-hub-signature-256", sign(&verified))
                .header("x-github-event", "issues")
                .header("content-type", "application/json")
                .body(Body::from(verified))
                .expect("request builds"),
        )
        .await;

    // A malformed body whose bytes are themselves a canary.
    canary
        .call(
            Request::post("/api/v1/repos")
                .header("authorization", "Bearer canary-bearer-token")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name": canary-log-content}"#))
                .expect("request builds"),
        )
        .await;
}
