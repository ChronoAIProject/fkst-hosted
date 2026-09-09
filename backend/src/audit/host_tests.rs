//! Unit tests for the shared `FKST_POSTHOG_HOST` rule.
//!
//! The control plane's and the relay's own configuration tests exercise this
//! through their real `from_vars`; these cover the rule itself, including the
//! shapes only a textual check can catch.

use super::*;

#[test]
fn a_tls_host_is_accepted_and_its_trailing_slash_removed() {
    assert_eq!(
        normalize("https://posthog.example/", "production").expect("tls host is valid"),
        "https://posthog.example"
    );
    // A self-hosted deployment behind a path prefix must keep its prefix.
    assert_eq!(
        normalize("https://obs.example/posthog", "production").expect("prefixed host is valid"),
        "https://obs.example/posthog"
    );
}

#[test]
fn plaintext_needs_an_explicit_test_or_local_environment() {
    for environment in ["production", "staging", ""] {
        let error = normalize("http://posthog.internal", environment)
            .expect_err("plaintext must fail closed outside test/local");
        assert!(error.to_string().contains("https"), "{environment}");
    }
    for environment in ["test", "Local"] {
        assert_eq!(
            normalize("http://127.0.0.1:8000", environment)
                .unwrap_or_else(|e| panic!("{environment} must permit plaintext: {e}")),
            "http://127.0.0.1:8000"
        );
    }
}

#[test]
fn userinfo_is_refused_on_both_paths_and_never_echoed() {
    for raw in [
        "https://svc:phc_canary_do_not_leak@posthog.example",
        "svc:phc_canary_do_not_leak@posthog.example",
    ] {
        let staged = stage(raw).expect_err("a staged credential must fail closed");
        assert!(staged.to_string().contains("userinfo"), "{raw}");
        assert!(
            !format!("{staged:?}").contains("phc_canary_do_not_leak"),
            "the rejection leaked the credential: {staged:?}"
        );
    }
    let dialled = normalize("https://svc:secret@posthog.example", "production")
        .expect_err("a dialled credential must fail closed");
    assert!(dialled.to_string().contains("userinfo"));
}

#[test]
fn a_blank_or_unparseable_or_wrong_scheme_host_is_refused() {
    assert!(normalize("", "production")
        .expect_err("blank")
        .to_string()
        .contains("FKST_POSTHOG_HOST"));
    assert!(normalize("posthog.example", "production")
        .expect_err("no scheme")
        .to_string()
        .contains("valid URL"));
    assert!(normalize("ftp://posthog.example", "production")
        .expect_err("wrong scheme")
        .to_string()
        .contains("http(s)"));
}

#[test]
fn staging_keeps_a_shape_the_dialled_check_would_refuse() {
    // The whole point of the staged path: a half-prepared rollout must not fail
    // an unrelated deploy, so plaintext, a bare name, and a missing scheme are
    // all retained verbatim (minus a trailing slash).
    for raw in ["http://posthog.internal/", "posthog.internal", "not a url"] {
        let staged = stage(raw).expect("a staged host is kept as written");
        assert_eq!(staged, raw.trim_end_matches('/'));
    }
}
