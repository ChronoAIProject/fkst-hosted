//! Rendering tests for the unified error type and its JSON envelope.

use super::*;
use http_body_util::BodyExt;

async fn render(err: AppError) -> (StatusCode, serde_json::Value, Vec<(String, String)>) {
    let response = err.into_response();
    let status = response.status();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
        .collect();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json = serde_json::from_slice(&bytes).expect("json body");
    (status, json, headers)
}

#[tokio::test]
async fn validation_renders_400_invalid_request() {
    let (status, body, _headers) = render(AppError::Validation("bad field".into())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_request");
    assert_eq!(body["message"], "bad field");
}

#[tokio::test]
async fn not_found_renders_404_not_found() {
    let (status, body, _headers) =
        render(AppError::NotFound("package \"foo\" does not exist".into())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");
    assert_eq!(body["message"], "package \"foo\" does not exist");
}

#[tokio::test]
async fn conflict_renders_409_conflict() {
    let (status, body, _headers) = render(AppError::Conflict("duplicate name".into())).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "conflict");
    assert_eq!(body["message"], "duplicate name");
}

#[tokio::test]
async fn unavailable_renders_503_unavailable() {
    let (status, body, _headers) = render(AppError::Unavailable("github unreachable".into())).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "unavailable");
    assert_eq!(body["message"], "github unreachable");
}

#[tokio::test]
async fn unauthorized_renders_401_with_www_authenticate_bearer() {
    let (status, body, headers) =
        render(AppError::Unauthorized("missing bearer token".into())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "unauthorized");
    assert_eq!(body["message"], "missing bearer token");
    let www = headers.iter().find(|(k, _)| k == "www-authenticate");
    assert!(www.is_some(), "WWW-Authenticate header must be present");
    assert_eq!(www.unwrap().1, "Bearer");
}

#[tokio::test]
async fn forbidden_renders_403_forbidden() {
    let (status, body, headers) = render(AppError::Forbidden("insufficient scope".into())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "forbidden");
    assert_eq!(body["message"], "insufficient scope");
    let www = headers.iter().find(|(k, _)| k == "www-authenticate");
    assert!(www.is_none(), "Forbidden must NOT set WWW-Authenticate");
}

#[tokio::test]
async fn bson_renders_500_without_leaking_inner_text() {
    // BSON document keys must be strings; integer keys produce a real
    // bson::ser::Error.
    let bad_keys: std::collections::HashMap<u32, &str> =
        std::collections::HashMap::from([(1, "leaky-detail")]);
    let bson_err = bson::to_document(&bad_keys).expect_err("non-string keys must fail");
    let err = AppError::Bson(bson_err);
    let inner_text = format!("{err}");

    let (status, body, _headers) = render(err).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"], "internal");
    assert_eq!(body["message"], "internal server error");
    // The serializer's own message never reaches the client.
    let rendered = body.to_string();
    assert!(!rendered.contains(inner_text.trim_start_matches("bson serialization error: ")));
}

#[tokio::test]
async fn internal_renders_500_without_leaking_inner_text() {
    let err = AppError::Internal(anyhow::anyhow!("db creds: secret"));
    // The inner text stays reachable for logging via Display/Debug.
    assert!(format!("{err}").contains("db creds: secret"));
    assert!(format!("{err:?}").contains("db creds: secret"));

    let (status, body, _headers) = render(err).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"], "internal");
    assert_eq!(body["message"], "internal server error");
    assert!(!body.to_string().contains("db creds"));
    assert!(!body.to_string().contains("secret"));
}

#[tokio::test]
async fn config_renders_500_without_leaking_inner_text() {
    let err = AppError::Config("envy: missing FOO".into());
    // The inner text stays reachable for logging via Display/Debug.
    assert!(format!("{err}").contains("envy: missing FOO"));
    assert!(format!("{err:?}").contains("envy: missing FOO"));

    let (status, body, _headers) = render(err).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["error"], "internal");
    assert_eq!(body["message"], "internal server error");
    assert!(!body.to_string().contains("envy"));
    assert!(!body.to_string().contains("FOO"));
}

#[tokio::test]
async fn unprocessable_renders_422() {
    let (status, body, _headers) = render(AppError::Unprocessable("semantic issue".into())).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "unprocessable");
    assert_eq!(body["message"], "semantic issue");
}

#[tokio::test]
async fn rate_limited_renders_429_with_retry_after_header() {
    let (status, body, headers) = render(AppError::RateLimited {
        message: "github rate limited; retry later".into(),
        retry_after_secs: 42,
    })
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"], "rate_limited");
    assert_eq!(body["message"], "github rate limited; retry later");
    let retry = headers.iter().find(|(k, _)| k == "retry-after");
    assert!(retry.is_some(), "Retry-After header must be present");
    assert_eq!(retry.unwrap().1, "42");
}

#[tokio::test]
async fn upstream_renders_502_upstream_error() {
    let (status, body, _headers) = render(AppError::Upstream("github returned 500".into())).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "upstream_error");
    assert_eq!(body["message"], "github returned 500");
}

#[tokio::test]
async fn github_app_not_installed_renders_422_with_hint() {
    let err: AppError = crate::github_app::GithubAppError::NotInstalled {
        owner_repo: "acme/site".to_string(),
        install_url: Some("https://github.com/apps/fkst-hosted/installations/new".to_string()),
    }
    .into();
    let (status, body, _headers) = render(err).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "unprocessable");
    let msg = body["message"].as_str().expect("message");
    assert!(msg.contains("acme/site"), "message: {msg}");
    assert!(msg.contains("fkst-hosted"), "message: {msg}");
}

#[tokio::test]
async fn github_app_not_installed_without_slug_gives_admin_hint() {
    let err: AppError = crate::github_app::GithubAppError::NotInstalled {
        owner_repo: "acme/site".to_string(),
        install_url: None,
    }
    .into();
    let (status, body, _headers) = render(err).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let msg = body["message"].as_str().expect("message");
    assert!(msg.contains("ask an admin"), "message: {msg}");
}

#[tokio::test]
async fn github_app_rate_limited_renders_503() {
    let err: AppError = crate::github_app::GithubAppError::RateLimited(120).into();
    let (status, body, _headers) = render(err).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "unavailable");
    assert!(body["message"].as_str().unwrap().contains("rate limited"));
}

#[tokio::test]
async fn github_app_token_rejected_detail_never_reaches_client() {
    let err: AppError =
        crate::github_app::GithubAppError::TokenRequestRejected("secret detail".to_string()).into();
    let (status, body, _headers) = render(err).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        !body.to_string().contains("secret detail"),
        "rejected detail must not leak: {}",
        body
    );
}

#[tokio::test]
async fn github_app_installation_mismatch_renders_422_without_repository_details() {
    let err: AppError = crate::github_app::GithubAppError::InstallationMismatch {
        repositories: vec![
            "private-owner/lifecycle".to_string(),
            "private-owner/implementation".to_string(),
        ],
    }
    .into();
    let (status, body, _headers) = render(err).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "unprocessable");
    assert_eq!(
        body["message"],
        "cross-repository delivery repositories must share one github app installation"
    );
    assert!(!body.to_string().contains("private-owner"));
}

#[tokio::test]
async fn github_app_auth_and_key_errors_render_500() {
    for err in [
        AppError::from(crate::github_app::GithubAppError::AppAuth),
        AppError::from(crate::github_app::GithubAppError::InvalidKey),
        AppError::from(crate::github_app::GithubAppError::Http(
            "network failure".to_string(),
        )),
    ] {
        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal");
        assert_eq!(body["message"], "internal server error");
    }
}

#[tokio::test]
async fn github_app_invalid_repo_ref_renders_400() {
    let err: AppError = crate::github_app::GithubAppError::InvalidRepoRef.into();
    let (status, body, _headers) = render(err).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_request");
}

#[tokio::test]
async fn no_error_or_debug_output_contains_minted_token_or_key() {
    use crate::github_app::GithubAppError;
    let secret_token = "ghs_SECRET_INSTALLATION_TOKEN_12345";
    let secret_pem = "-----BEGIN RSA PRIVATE KEY-----\nSECRET\n-----END RSA PRIVATE KEY-----";
    let errors: Vec<GithubAppError> = vec![
        GithubAppError::NotInstalled {
            owner_repo: "a/b".to_string(),
            install_url: None,
        },
        GithubAppError::InstallationGone {
            owner_repo: "a/b".to_string(),
        },
        GithubAppError::AppAuth,
        GithubAppError::RateLimited(60),
        GithubAppError::TokenRequestRejected(format!("permission denied for {secret_token}")),
        GithubAppError::InvalidKey,
        GithubAppError::InvalidRepoRef,
        GithubAppError::RefExists,
        GithubAppError::Http(format!("request failed with {secret_pem}")),
    ];
    for err in &errors {
        let display = format!("{err}");
        let debug = format!("{err:?}");
        assert!(
            !display.contains(secret_token),
            "Display leaked token: {display}"
        );
        assert!(!debug.contains(secret_token), "Debug leaked token: {debug}");
        assert!(
            !display.contains(secret_pem),
            "Display leaked key: {display}"
        );
        assert!(!debug.contains(secret_pem), "Debug leaked key: {debug}");
    }
    // The AppError mapping also must not leak.
    for err in &errors {
        let app_err: AppError = err.clone().into();
        let display = format!("{app_err}");
        let debug = format!("{app_err:?}");
        assert!(
            !display.contains(secret_token),
            "AppError Display leaked token: {display}"
        );
        assert!(
            !debug.contains(secret_token),
            "AppError Debug leaked token: {debug}"
        );
    }
}

#[tokio::test]
async fn scope_forbidden_renders_403_with_its_own_stable_code() {
    // A distinct code from plain `forbidden`: "this scope is admin-only" and
    // "this deployment does not admit you" need different client remedies.
    let (status, body, _headers) =
        render(AppError::ScopeForbidden("admins only".to_string())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "operations_scope_forbidden");
    assert_eq!(body["message"], "admins only");
}

#[tokio::test]
async fn session_visibility_unavailable_renders_503_with_its_own_stable_code() {
    let (status, body, _headers) = render(AppError::SessionVisibilityUnavailable(
        "still recovering".to_string(),
    ))
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "session_visibility_unavailable");
    assert_eq!(body["message"], "still recovering");
}
