//! Exhaustive tests for the fail-closed log [`Redactor`]. Split into a sibling file
//! so `redact.rs` stays well under the 500-line module cap. This library carries the
//! hard no-leak guarantee, so every layer, every derived secret form, the chunk
//! carry-over, and the overflow fail-closed path is asserted here.

use super::*;

use base64::engine::general_purpose::{STANDARD, URL_SAFE};

/// Convenience: does `text` still contain `needle` (i.e. did a secret leak)?
fn leaks(text: &str, needle: &str) -> bool {
    text.contains(needle)
}

// ---- Layer 1: known-secret exact + derived forms --------------------------------

#[test]
fn exact_secret_is_masked_with_its_label() {
    let secret = "ghs_verysecrettokenvalue";
    let redactor = Redactor::new(&[("app-token", secret)]);
    let out = redactor.redact_line(&format!("using token {secret} now"));
    assert!(!leaks(&out, secret), "raw secret must be gone: {out}");
    assert!(
        out.contains("«REDACTED:app-token»"),
        "Layer 1 label wins over the generic denylist: {out}"
    );
}

#[test]
fn base64_and_base64url_and_percent_derived_forms_are_masked() {
    let secret = "my-secret-value-xyz";
    let redactor = Redactor::new(&[("app-token", secret)]);

    let std_b64 = STANDARD.encode(secret);
    let url_b64 = URL_SAFE.encode(secret);
    let out = redactor.redact_line(&format!("std={std_b64} url={url_b64}"));
    assert!(!leaks(&out, &std_b64), "standard base64 form leaked: {out}");
    assert!(!leaks(&out, &url_b64), "base64url form leaked: {out}");
    assert!(out.contains("«REDACTED:app-token»"));

    // A value with URL-reserved chars, appearing percent-encoded.
    let pw = "a b/c";
    let redactor = Redactor::new(&[("pw", pw)]);
    let encoded = percent_encode(pw); // "a%20b%2Fc"
    let out = redactor.redact_line(&format!("q={encoded}&x=1"));
    assert!(!leaks(&out, &encoded), "percent-encoded form leaked: {out}");
    assert!(out.contains("«REDACTED:pw»"));
}

#[test]
fn url_composed_x_access_token_form_is_masked() {
    let token = "ghs_TOKENVALUE";
    let redactor = Redactor::new(&[("app-token", token)]);
    let out = redactor.redact_line(&format!(
        "cloning https://x-access-token:{token}@github.com/o/r.git"
    ));
    assert!(!leaks(&out, token), "token leaked from clone URL: {out}");
    assert!(
        !leaks(&out, "x-access-token"),
        "the composed prefix should be consumed too: {out}"
    );
    assert!(out.contains("«REDACTED:app-token»"));
}

#[test]
fn rotated_secret_added_at_runtime_is_masked_and_old_survives() {
    let old = "ghs_oldtokenvalue";
    let new = "ghs_newrotatedtoken";
    let mut redactor = Redactor::new(&[("app-token", old)]);
    redactor.add_secret("app-token", new);

    let out = redactor.redact_line(&format!("old={old} new={new}"));
    assert!(!leaks(&out, old), "old secret must still be masked: {out}");
    assert!(!leaks(&out, new), "rotated secret must be masked: {out}");
    assert!(out.matches("«REDACTED:app-token»").count() >= 2);
}

#[test]
fn a_redactor_with_no_secrets_still_runs_the_later_layers() {
    // An empty-secret redactor's Layer 1 is a no-op, but Layer 2/3 still fire.
    let redactor = Redactor::new(&[]);
    let out = redactor.redact_line("Authorization: Bearer abc.def.ghijk");
    assert!(out.contains("«REDACTED:authorization»"), "{out}");
}

// ---- Layer 2: pattern denylist (one line per shape) -----------------------------

/// Redact `line` with a secret-free redactor and assert it now carries `label` and no
/// longer contains `leak`.
fn assert_denylist(line: &str, label: &str, leak: &str) {
    let out = Redactor::new(&[]).redact_line(line);
    assert!(out.contains(label), "expected {label} in: {out}");
    assert!(!leaks(&out, leak), "{leak:?} leaked through: {out}");
}

#[test]
fn denylist_masks_github_token() {
    let tok = "ghp_0123456789abcdefghijklmnopqrstuvwxyz12";
    assert_denylist(&format!("token: {tok}"), "«REDACTED:github-token»", tok);
}

#[test]
fn denylist_masks_openai_style_key() {
    let key = "sk-abcdefghijklmnopqrstuvwxyz012345";
    assert_denylist(&format!("OPENAI={key}"), "«REDACTED:api-key»", key);
}

#[test]
fn denylist_masks_url_credentials() {
    assert_denylist(
        "remote https://alice:hunter2@example.com/repo",
        "«REDACTED:url-credential»",
        "alice:hunter2@",
    );
}

#[test]
fn denylist_masks_jwt() {
    let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV";
    assert_denylist(&format!("bearer {jwt}"), "«REDACTED:jwt»", jwt);
}

#[test]
fn denylist_masks_password_assignment() {
    assert_denylist(
        "db password=hunter2secretvalue extra",
        "«REDACTED:password»",
        "hunter2secretvalue",
    );
}

#[test]
fn denylist_masks_authorization_header() {
    assert_denylist(
        "Authorization: Basic dXNlcjpwYXNz",
        "«REDACTED:authorization»",
        "dXNlcjpwYXNz",
    );
}

#[test]
fn denylist_masks_netrc_line() {
    assert_denylist(
        "machine github.com login alice password s3cr3tpass",
        "«REDACTED:netrc»",
        "s3cr3tpass",
    );
}

#[test]
fn denylist_masks_multi_line_pem_private_key_span() {
    let pem = "-----BEGIN RSA PRIVATE KEY-----\n\
               MIIBOgIBAAJBAKj34GkxFhD90vcNLYLInFEX6Ppy1tPf9Cnzj4p4WGeKLs1Pt8Q\n\
               uKUpRKfFLfRYC9AIKjbJTWit+CqvjSFmk/eAmn0=\n\
               -----END RSA PRIVATE KEY-----";
    let out = Redactor::new(&[]).redact_line(pem);
    assert!(
        out.contains("«REDACTED:private-key»"),
        "PEM span not masked: {out}"
    );
    assert!(!leaks(&out, "MIIBOgIBAAJBAKj34"), "PEM body leaked: {out}");
    assert!(!leaks(&out, "BEGIN"), "PEM markers leaked: {out}");
}

// ---- Layer 3: entropy fallback + allowlist --------------------------------------

#[test]
fn entropy_masks_a_high_entropy_run() {
    let blob = "Gq8xZ2vKpL4mNc7bY1wTe9RfHj0sAdU3iOl6PkQr5Zt";
    let out = Redactor::new(&[]).redact_line(&format!("nonce={blob}"));
    assert!(
        out.contains("«REDACTED:high-entropy»"),
        "high-entropy run not masked: {out}"
    );
    assert!(!leaks(&out, blob), "high-entropy run leaked: {out}");
}

#[test]
fn entropy_allowlists_a_40_hex_git_sha() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    assert_eq!(sha.len(), 40);
    let out = Redactor::new(&[]).redact_line(&format!("HEAD is at {sha}"));
    assert!(out.contains(sha), "a git SHA must survive: {out}");
    assert!(!out.contains("«REDACTED"), "no mask expected: {out}");
}

#[test]
fn entropy_allowlists_a_uuid() {
    let uuid = "550e8400-e29b-41d4-a716-446655440000";
    let out = Redactor::new(&[]).redact_line(&format!("session {uuid} started"));
    assert!(out.contains(uuid), "a UUID must survive: {out}");
    assert!(!out.contains("«REDACTED"), "no mask expected: {out}");
}

#[test]
fn a_clean_line_passes_through_unchanged() {
    let clean = "Cloning into 'my-repo'... remote: Enumerating objects: 42, done.";
    let out = Redactor::new(&[("app-token", "ghs_unused")]).redact_line(clean);
    assert_eq!(out, clean, "a clean line must be returned verbatim");
}

// ---- Fail-closed framing: chunk carry-over + overflow ----------------------------

#[test]
fn chunk_carry_over_never_leaks_a_split_secret() {
    let secret = "supersecretvalue123";
    let mut redactor = Redactor::new(&[("app-token", secret)]);

    // The secret straddles the boundary: no newline yet, so nothing is emitted.
    let out1 = redactor.redact_chunk("prefix supersecre");
    assert!(
        out1.is_empty(),
        "an unterminated tail must be held: {out1:?}"
    );
    assert!(!leaks(&out1, "supersecre"), "partial secret leaked: {out1}");

    // The rest arrives and a newline completes the line — now it is scanned whole.
    let out2 = redactor.redact_chunk("tvalue123 suffix\n");
    assert!(!leaks(&out2, secret), "reassembled secret leaked: {out2}");
    assert!(out2.contains("«REDACTED:app-token»"), "{out2}");
    assert!(
        out2.ends_with('\n'),
        "the framed newline is preserved: {out2:?}"
    );
}

#[test]
fn flush_redacts_a_final_unterminated_line() {
    let secret = "ghs_flushme";
    let mut redactor = Redactor::new(&[("app-token", secret)]);
    let mid = redactor.redact_chunk(&format!("tail {secret}"));
    assert!(mid.is_empty(), "no newline yet: {mid:?}");
    let end = redactor.flush();
    assert!(!leaks(&end, secret), "flush leaked the tail secret: {end}");
    assert!(end.contains("«REDACTED:app-token»"));
}

#[test]
fn oversized_line_fails_closed_to_overflow() {
    let secret = "ghs_secretinsidealongline";
    let redactor = Redactor::new(&[("app-token", secret)]).with_max_line_bytes(16);
    let line = format!("this line is far longer than the cap and hides {secret}");
    let out = redactor.redact_line(&line);
    assert_eq!(out, "«REDACTED:overflow»", "over-cap line must fail closed");
    assert!(
        !leaks(&out, secret),
        "an over-cap line must never leak: {out}"
    );
}

#[test]
fn oversized_unterminated_chunk_tail_fails_closed() {
    let secret = "ghs_secrethiddenintail";
    let mut redactor = Redactor::new(&[("app-token", secret)]).with_max_line_bytes(16);
    // A long chunk with no newline overruns the held-tail cap.
    let out = redactor.redact_chunk(&format!("no newline here at all but {secret} lurks"));
    assert!(out.contains("«REDACTED:overflow»"), "{out}");
    assert!(!leaks(&out, secret), "over-cap tail must never leak: {out}");
}

// ---- Pure helpers ---------------------------------------------------------------

#[test]
fn percent_encode_escapes_reserved_bytes_only() {
    assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
    assert_eq!(percent_encode("plain-Token_1.0~"), "plain-Token_1.0~");
}

#[test]
fn shannon_entropy_orders_random_above_repetitive() {
    let random = shannon_entropy("Gq8xZ2vKpL4mNc7bY1wTe9RfHj0sAdU3");
    let repeated = shannon_entropy("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(random > repeated, "random={random} repeated={repeated}");
    assert!(
        repeated < 1.0,
        "a single-char run has ~zero entropy: {repeated}"
    );
}
