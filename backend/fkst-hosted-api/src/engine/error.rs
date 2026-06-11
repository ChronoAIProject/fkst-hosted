//! Runner error type and the stderr-capture truncation helper.
//!
//! Classification intent for the future Sessions API layer:
//! - [`RunnerError::InvalidPackage`] -> 400 (caller-supplied package is bad).
//! - [`RunnerError::ConformanceFailed`] with exit code 1 -> 400-class (the
//!   engine's pre-flight rejected the package content); exit code 2 or -1
//!   (timeout / group-killed) -> 500-class / session `failed`.
//! - [`RunnerError::StartupFailed`] -> session `failed` (supervise died,
//!   panicked, or never became ready).
//! - [`RunnerError::Spawn`] / [`RunnerError::Io`] /
//!   [`RunnerError::Signal`] -> 500 (host-side failure).

use std::borrow::Cow;

/// Errors produced by the session runner. Never panics on engine
/// misbehavior — a crashed/exited engine is an error value, not an abort.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    /// The package failed structural validation (empty files, missing
    /// engine entry, bad name, path traversal). Maps to API 400.
    #[error("invalid package: {0}")]
    InvalidPackage(String),
    /// The `conformance` pre-flight exited non-zero (`code` 1 = check
    /// failure -> 400-class, 2 = engine SDK/IO error) or timed out
    /// (`code` -1, conformance process group killed).
    #[error("conformance failed (exit {code})")]
    ConformanceFailed { code: i32, stderr: String },
    /// `supervise` was spawned but exited, panicked, or failed to emit the
    /// ready markers within the ready timeout. Carries the stderr tail.
    #[error("engine startup failed")]
    StartupFailed { stderr: String },
    /// Could not launch the engine binary at all (e.g. bad
    /// `framework_bin`). Maps to API 500.
    #[error("failed to spawn fkst-framework")]
    Spawn(#[source] std::io::Error),
    /// Materialization / temp-dir / filesystem failure. Maps to API 500.
    #[error("io error")]
    Io(#[from] std::io::Error),
    /// Signalling failure (e.g. an unreapable process group). Maps to
    /// API 500.
    #[error("signal error")]
    Signal(#[source] nix::Error),
}

/// Lossily decode captured process output and truncate it to at most `cap`
/// bytes, never splitting a UTF-8 character.
///
/// Non-UTF-8 input is decoded with `String::from_utf8_lossy` first, so the
/// cap applies to the decoded text (U+FFFD replacement chars included).
pub fn truncate_output_lossy(bytes: &[u8], cap: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= cap {
        return text.into_owned();
    }
    let mut cut = cap;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    match text {
        Cow::Borrowed(s) => s[..cut].to_owned(),
        Cow::Owned(mut s) => {
            s.truncate(cut);
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- truncate_output_lossy ---------------------------------------------

    #[test]
    fn short_ascii_is_returned_verbatim() {
        assert_eq!(
            truncate_output_lossy(b"conformance FAIL", 8192),
            "conformance FAIL"
        );
    }

    #[test]
    fn ascii_is_cut_exactly_at_the_cap() {
        assert_eq!(truncate_output_lossy(b"abcdef", 4), "abcd");
    }

    #[test]
    fn ascii_exactly_at_the_cap_is_not_cut() {
        assert_eq!(truncate_output_lossy(b"abcd", 4), "abcd");
    }

    #[test]
    fn zero_cap_yields_empty_string() {
        assert_eq!(truncate_output_lossy(b"abc", 0), "");
    }

    #[test]
    fn multibyte_char_is_never_split() {
        // "héllo": 'é' spans bytes 1..3; a cap of 2 lands mid-'é' and must
        // back off to the previous boundary.
        let input = "héllo".as_bytes();
        assert_eq!(truncate_output_lossy(input, 2), "h");
        assert_eq!(truncate_output_lossy(input, 3), "hé");
    }

    #[test]
    fn four_byte_char_is_never_split() {
        // U+1F980 (4 bytes) repeated; caps inside any char back off cleanly.
        let input = "🦀🦀🦀".as_bytes();
        for cap in 0..4 {
            assert_eq!(truncate_output_lossy(input, cap), "");
        }
        for cap in 4..8 {
            assert_eq!(truncate_output_lossy(input, cap), "🦀");
        }
        assert_eq!(truncate_output_lossy(input, 8), "🦀🦀");
    }

    #[test]
    fn non_utf8_input_is_lossily_decoded_then_capped() {
        // 0xFF decodes to U+FFFD (3 bytes); the cap applies post-decode and
        // must not split a replacement char.
        let input = [b'a', 0xFF, 0xFF];
        let decoded = truncate_output_lossy(&input, 64);
        assert_eq!(decoded, "a\u{FFFD}\u{FFFD}");
        assert_eq!(truncate_output_lossy(&input, 2), "a");
        assert_eq!(truncate_output_lossy(&input, 4), "a\u{FFFD}");
    }

    #[test]
    fn result_is_always_valid_utf8_within_cap() {
        let input = "αβγδε mixed 🦀 bytes".as_bytes();
        for cap in 0..=input.len() + 2 {
            let out = truncate_output_lossy(input, cap);
            assert!(out.len() <= cap, "cap {cap}: {} bytes", out.len());
            // String construction itself proves UTF-8 validity.
        }
    }

    // ---- RunnerError display -------------------------------------------------

    #[test]
    fn error_display_carries_classification_detail() {
        assert_eq!(
            RunnerError::InvalidPackage("no files".into()).to_string(),
            "invalid package: no files"
        );
        assert_eq!(
            RunnerError::ConformanceFailed {
                code: 1,
                stderr: "FAIL".into()
            }
            .to_string(),
            "conformance failed (exit 1)"
        );
        assert_eq!(
            RunnerError::StartupFailed { stderr: "x".into() }.to_string(),
            "engine startup failed"
        );
        assert_eq!(
            RunnerError::Spawn(std::io::Error::other("nope")).to_string(),
            "failed to spawn fkst-framework"
        );
        assert_eq!(
            RunnerError::Io(std::io::Error::other("disk")).to_string(),
            "io error"
        );
        assert_eq!(
            RunnerError::Signal(nix::Error::ESRCH).to_string(),
            "signal error"
        );
    }

    #[test]
    fn io_error_converts_via_from() {
        let err: RunnerError = std::io::Error::other("boom").into();
        assert!(matches!(err, RunnerError::Io(_)));
    }

    #[test]
    fn source_chain_is_preserved() {
        use std::error::Error as _;
        let err = RunnerError::Spawn(std::io::Error::other("missing bin"));
        let source = err.source().expect("spawn must chain its io source");
        assert!(source.to_string().contains("missing bin"));
    }
}
