//! Unit tests for the frontend GitHub-OAuth login helpers (`crate::routes::auth`).
//!
//! Split out of `auth.rs` to keep that module under the file-size budget; `super`
//! resolves to the `auth` module, so these reach its private helpers directly.

use super::*;

#[test]
fn callback_redirect_uri_appends_the_path_and_trims_slash() {
    assert_eq!(
        callback_redirect_uri("https://fkst.example/"),
        "https://fkst.example/api/v1/auth/github/callback"
    );
    assert_eq!(
        callback_redirect_uri("https://fkst.example"),
        "https://fkst.example/api/v1/auth/github/callback"
    );
}

#[test]
fn a_freshly_signed_state_round_trips_and_is_fresh() {
    let secret = b"client-secret";
    let state = oauth::sign_state(secret, &login_state_message());
    let message = oauth::verify_state(secret, &state).expect("verifies");
    assert!(state_is_fresh(&message), "just-issued state must be fresh");
}

#[test]
fn an_old_state_is_rejected_as_stale() {
    // A timestamp well outside the window.
    let old = now_unix() - (STATE_MAX_AGE_SECS + 60);
    assert!(!state_is_fresh(&format!("login:{old}")));
}

#[test]
fn malformed_state_messages_are_not_fresh() {
    assert!(!state_is_fresh("not-a-login-message"));
    assert!(!state_is_fresh("login:not-a-number"));
    assert!(!state_is_fresh("login:"));
}

#[test]
fn a_login_state_is_not_fresh_as_a_broader_state_and_vice_versa() {
    // The `kind` prefix namespaces flows: a state minted for one flow must not
    // satisfy another's freshness check even though the HMAC would verify.
    let login = signed_state_message("login");
    assert!(state_is_fresh_for("login", &login));
    assert!(
        !state_is_fresh_for("broader", &login),
        "a login state must not pass a broader freshness check"
    );
    let broader = signed_state_message("broader");
    assert!(state_is_fresh_for("broader", &broader));
    assert!(!state_is_fresh_for("login", &broader));
}

#[test]
fn success_url_carries_all_tokens_in_the_fragment() {
    let tokens = oauth::TokenSet {
        access_token: SecretString::from("ghu_access".to_string()),
        refresh_token: Some(SecretString::from("ghr_refresh".to_string())),
        expires_in: Some(28800),
        refresh_token_expires_in: Some(15811200),
    };
    let url = frontend_success_url("https://app.example/fkst/", &tokens);
    assert_eq!(
        url,
        "https://app.example/fkst/#gh_token=ghu_access&gh_refresh=ghr_refresh&gh_expires_in=28800"
    );
}

#[test]
fn success_url_omits_refresh_when_absent() {
    let tokens = oauth::TokenSet {
        access_token: SecretString::from("ghu_access".to_string()),
        refresh_token: None,
        expires_in: None,
        refresh_token_expires_in: None,
    };
    let url = frontend_success_url("https://app.example/", &tokens);
    assert_eq!(url, "https://app.example/#gh_token=ghu_access");
}

#[test]
fn token_response_exposes_the_tokens_and_bearer_type() {
    let tokens = oauth::TokenSet {
        access_token: SecretString::from("ghu_x".to_string()),
        refresh_token: Some(SecretString::from("ghr_y".to_string())),
        expires_in: Some(28800),
        refresh_token_expires_in: None,
    };
    let resp = token_response(&tokens);
    assert_eq!(resp.access_token, "ghu_x");
    assert_eq!(resp.refresh_token.as_deref(), Some("ghr_y"));
    assert_eq!(resp.expires_in, Some(28800));
    assert_eq!(resp.token_type, "bearer");
}

#[test]
fn stateless_install_redirects_to_the_dashboard() {
    assert_eq!(
        post_install_redirect(
            None,
            Some("install"),
            Some("146704012"),
            "https://fkst.example/"
        ),
        Some("https://fkst.example/dashboard".to_string())
    );
    // installation_id alone is enough (setup_action can be absent).
    assert_eq!(
        post_install_redirect(Some(""), None, Some("1"), "https://fkst.example"),
        Some("https://fkst.example/dashboard".to_string())
    );
}

#[test]
fn a_real_login_callback_is_untouched() {
    // State present -> normal login path even if GitHub echoes extras.
    assert_eq!(
        post_install_redirect(
            Some("signed-state"),
            Some("install"),
            Some("1"),
            "https://f"
        ),
        None
    );
    // No install markers + no state -> not an install; the 400 path owns it.
    assert_eq!(post_install_redirect(None, None, None, "https://f"), None);
}
