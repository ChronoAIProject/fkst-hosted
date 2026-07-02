//! The native, PURE, fail-closed log redactor (log-streaming Wave 1).
//!
//! This library carries the hard no-leak guarantee for a session pod's log stream:
//! given a LINE-FRAMED text stream it returns a copy with every credential-shaped
//! run masked as `«REDACTED:<label>»`, and it is designed so that NO input — however
//! pathological — can cause it to emit unredacted content. It is deliberately I/O
//! free: the effectful fan-out that pipes a live pod's logs through it is a later
//! wave. Everything here is a pure function of its inputs, exhaustively unit-tested.
//!
//! Three defence-in-depth layers run on every line, most-specific first:
//!  1. **Known-secret exact match** — an [`aho_corasick`] automaton over each injected
//!     secret plaintext AND its derived encodings (base64, base64url, percent-encoded,
//!     and the `x-access-token:<v>@` / `<user>:<v>@` URL-composed forms). A rotated
//!     secret is added at runtime via [`Redactor::add_secret`], which REBUILDS the
//!     automaton and never drops a prior value.
//!  2. **Pattern denylist** — a [`regex::RegexSet`] of credential shapes (GitHub /
//!     OpenAI-style tokens, URL creds, PEM private-key spans, JWTs, `password=`,
//!     `Authorization:` headers, and `.netrc` lines).
//!  3. **Entropy fallback** — any base64/hex run ≥20 chars whose Shannon entropy
//!     clears a tuned threshold is masked, EXCEPT 40-hex git SHAs and UUIDs, which
//!     are allow-listed so real identifiers survive.
//!
//! Fail-closed: a line over the size cap (or any internal panic) returns
//! `«REDACTED:overflow»` rather than the raw line, and [`Redactor::redact_chunk`]
//! holds the unterminated tail so a secret straddling a chunk boundary is never
//! emitted before it can be scanned whole.

use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

use aho_corasick::{AhoCorasick, MatchKind};
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine as _;
use regex::{Captures, Regex, RegexSet};

/// Default per-line size cap (64 KiB). A line larger than this is treated as
/// pathological and masked wholesale rather than scanned.
const DEFAULT_MAX_LINE_BYTES: usize = 64 * 1024;
/// The sentinel a fail-closed path emits instead of raw content.
const OVERFLOW_MASK: &str = "«REDACTED:overflow»";
/// Label used for a Layer-3 entropy hit.
const ENTROPY_LABEL: &str = "high-entropy";
/// Minimum run length (chars) the entropy layer considers.
const MIN_ENTROPY_RUN: usize = 20;
/// Shannon-entropy thresholds (bits/char), tuned per alphabet. Pure-hex runs top out
/// near 4.0 bits/char, so a lower bar catches random hex secrets while structured hex
/// (repeats) passes; base64-ish runs reach higher, so a higher bar avoids flagging
/// ordinary hyphen/underscore identifiers. The 40-hex git SHA and UUID shapes clear
/// these bars but are allow-listed separately.
const HEX_ENTROPY_THRESHOLD: f64 = 3.2;
const BASE64_ENTROPY_THRESHOLD: f64 = 3.9;

/// The Layer-2 pattern denylist as `(label, regex)` pairs. The label rides the mask
/// so a reader can tell WHICH shape fired. `(?s)` on the PEM entry lets it span the
/// newlines of a multi-line private key when a whole block is scanned at once.
const DENYLIST: &[(&str, &str)] = &[
    ("github-token", r"gh[psuor]_[A-Za-z0-9]{36,}"),
    ("api-key", r"sk-[A-Za-z0-9-]{20,}"),
    ("url-credential", r"https://[^/@\s]+:[^/@\s]+@"),
    (
        "private-key",
        r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
    ),
    ("jwt", r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+"),
    ("password", r"password=\S+"),
    ("authorization", r"Authorization:\s*(?:Basic|Bearer)\s+\S+"),
    ("netrc", r"machine\s+\S+\s+login\s+\S+\s+password\s+\S+"),
];

/// A fail-closed, line-framed log redactor. Cheap to clone-free reuse across a whole
/// stream; [`redact_chunk`](Self::redact_chunk) additionally carries the pending tail
/// as mutable state.
pub struct Redactor {
    /// Every known secret as `(label, value)`, retained so [`add_secret`](Self::add_secret)
    /// can rebuild the automaton over the FULL set (a rotated secret never drops a
    /// prior one).
    secrets: Vec<(String, String)>,
    /// Layer-1 automaton over all derived secret forms; `None` when no secret has a
    /// non-empty value (so Layer 1 is a no-op).
    automaton: Option<AhoCorasick>,
    /// Label for each automaton pattern id (parallel to the patterns fed to the
    /// builder), so a match resolves back to its secret's label.
    pattern_labels: Vec<String>,
    /// Per-line size cap; a longer line fails closed to [`OVERFLOW_MASK`].
    max_line_bytes: usize,
    /// The unterminated tail held between [`redact_chunk`](Self::redact_chunk) calls.
    carry: String,
}

impl Redactor {
    /// Build a redactor over the given `(label, value)` secrets. Empty-valued secrets
    /// contribute nothing (they would otherwise match everywhere).
    pub fn new(secrets: &[(&str, &str)]) -> Self {
        let secrets: Vec<(String, String)> = secrets
            .iter()
            .map(|(l, v)| (l.to_string(), v.to_string()))
            .collect();
        let (automaton, pattern_labels) = build_automaton(&secrets);
        Self {
            secrets,
            automaton,
            pattern_labels,
            max_line_bytes: DEFAULT_MAX_LINE_BYTES,
            carry: String::new(),
        }
    }

    /// Override the per-line size cap (bytes). Chiefly for tests exercising the
    /// fail-closed overflow path without allocating a 64 KiB line.
    pub fn with_max_line_bytes(mut self, max_line_bytes: usize) -> Self {
        self.max_line_bytes = max_line_bytes;
        self
    }

    /// Add a rotated secret at runtime. Rebuilds the automaton over the FULL secret
    /// set so both the new value AND every prior value stay masked.
    pub fn add_secret(&mut self, label: &str, value: &str) {
        self.secrets.push((label.to_string(), value.to_string()));
        let (automaton, pattern_labels) = build_automaton(&self.secrets);
        self.automaton = automaton;
        self.pattern_labels = pattern_labels;
    }

    /// Redact a single line, fail-closed: an over-cap line, or any internal panic,
    /// returns [`OVERFLOW_MASK`] rather than risk emitting raw content.
    pub fn redact_line(&self, line: &str) -> String {
        if line.len() > self.max_line_bytes {
            return OVERFLOW_MASK.to_string();
        }
        // Defence-in-depth: even if a layer were to panic on some pathological input,
        // the redactor must never return the unredacted line.
        match catch_unwind(AssertUnwindSafe(|| self.redact_text(line))) {
            Ok(out) => out,
            Err(_) => OVERFLOW_MASK.to_string(),
        }
    }

    /// Redact a stream CHUNK. Frames on `\n`: every complete line is redacted and
    /// emitted (with its newline); the unterminated tail is held internally so a
    /// secret split across a chunk boundary is never emitted before it can be scanned
    /// whole. Call [`flush`](Self::flush) at end-of-stream to redact the final tail.
    pub fn redact_chunk(&mut self, chunk: &str) -> String {
        self.carry.push_str(chunk);
        let mut out = String::new();
        while let Some(idx) = self.carry.find('\n') {
            let line: String = self.carry.drain(..=idx).collect();
            let line = line.strip_suffix('\n').unwrap_or(&line);
            out.push_str(&self.redact_line(line));
            out.push('\n');
        }
        // Bound the held tail: an unterminated line past the cap is emitted as
        // overflow (never leak, never buffer unboundedly) and the carry reset.
        if self.carry.len() > self.max_line_bytes {
            out.push_str(OVERFLOW_MASK);
            self.carry.clear();
        }
        out
    }

    /// Redact and return whatever unterminated tail remains after the last chunk.
    pub fn flush(&mut self) -> String {
        if self.carry.is_empty() {
            return String::new();
        }
        let tail = std::mem::take(&mut self.carry);
        self.redact_line(&tail)
    }

    /// The three-layer redaction pipeline over one (already size-checked) line.
    fn redact_text(&self, input: &str) -> String {
        let after_secrets = self.apply_secrets(input);
        let after_denylist = apply_denylist(&after_secrets);
        apply_entropy(&after_denylist)
    }

    /// Layer 1 — mask every known-secret exact match (and its derived forms).
    fn apply_secrets(&self, input: &str) -> String {
        let Some(automaton) = &self.automaton else {
            return input.to_string();
        };
        let mut out = String::with_capacity(input.len());
        automaton.replace_all_with(input, &mut out, |m, _matched, dst| {
            dst.push_str(&mask(&self.pattern_labels[m.pattern().as_usize()]));
            true
        });
        out
    }
}

/// Build the Layer-1 automaton (and its parallel label table) over every secret's
/// derived forms. Returns `None` when there is nothing to match.
fn build_automaton(secrets: &[(String, String)]) -> (Option<AhoCorasick>, Vec<String>) {
    let mut patterns: Vec<String> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for (label, value) in secrets {
        // Dedup a single secret's forms (base64/percent forms often coincide with the
        // raw value for alphanumeric tokens) so the automaton stays lean.
        let forms: BTreeSet<String> = derived_forms(value).into_iter().collect();
        for form in forms {
            patterns.push(form);
            labels.push(label.clone());
        }
    }
    if patterns.is_empty() {
        return (None, labels);
    }
    let automaton = AhoCorasick::builder()
        .match_kind(MatchKind::LeftmostLongest)
        .build(&patterns)
        .expect("aho-corasick automaton builds from non-empty literal patterns");
    (Some(automaton), labels)
}

/// Every masked form of one secret value: the raw value, its base64 / base64url
/// (padded + unpadded) and percent encodings, and the two URL-composed credential
/// shapes. An empty value yields nothing (it must never seed a match-everywhere
/// pattern).
fn derived_forms(value: &str) -> Vec<String> {
    if value.is_empty() {
        return Vec::new();
    }
    let mut forms = vec![
        value.to_string(),
        STANDARD.encode(value),
        STANDARD_NO_PAD.encode(value),
        URL_SAFE.encode(value),
        URL_SAFE_NO_PAD.encode(value),
        percent_encode(value),
        // `https://x-access-token:<v>@host` (GitHub App token in a clone URL) and the
        // generic `<user>:<v>@` shape — registering `:<v>@` catches ANY username.
        format!("x-access-token:{value}@"),
        format!(":{value}@"),
    ];
    forms.retain(|form| !form.is_empty());
    forms
}

/// Percent-encode every byte outside the RFC 3986 unreserved set. Hand-rolled (rather
/// than a new dependency) because the redactor needs the exact "encode everything
/// else" behaviour to reconstruct how a secret would appear once URL-escaped.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Layer 2 — mask each denylist shape present in the line. The [`RegexSet`] gives the
/// membership test in one pass; only the matched patterns are then replaced.
fn apply_denylist(input: &str) -> String {
    let matched = denylist_set().matches(input);
    if !matched.matched_any() {
        return input.to_string();
    }
    let regexes = denylist_regexes();
    let mut out = input.to_string();
    for id in matched.iter() {
        let replacement = mask(DENYLIST[id].0);
        out = regexes[id]
            .replace_all(&out, |_: &Captures| replacement.clone())
            .into_owned();
    }
    out
}

/// Layer 3 — mask high-entropy base64/hex runs, allow-listing git SHAs and UUIDs.
fn apply_entropy(input: &str) -> String {
    entropy_run_re()
        .replace_all(input, |caps: &Captures| {
            let run = &caps[0];
            if is_allowlisted(run) || !is_high_entropy(run) {
                run.to_string()
            } else {
                mask(ENTROPY_LABEL)
            }
        })
        .into_owned()
}

/// A run that is a legitimate high-entropy identifier we must NOT mask: a 40-hex git
/// SHA or a canonical UUID.
fn is_allowlisted(run: &str) -> bool {
    git_sha_re().is_match(run) || uuid_re().is_match(run)
}

/// Whether a candidate run's Shannon entropy clears the per-alphabet threshold.
fn is_high_entropy(run: &str) -> bool {
    let pure_hex = run.bytes().all(|byte| byte.is_ascii_hexdigit());
    let threshold = if pure_hex {
        HEX_ENTROPY_THRESHOLD
    } else {
        BASE64_ENTROPY_THRESHOLD
    };
    shannon_entropy(run) >= threshold
}

/// Shannon entropy in bits per character. Runs are drawn from an ASCII class, so a
/// per-byte histogram is exact.
fn shannon_entropy(run: &str) -> f64 {
    let mut counts = [0usize; 256];
    for &byte in run.as_bytes() {
        counts[byte as usize] += 1;
    }
    let len = run.len() as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}

/// The `«REDACTED:<label>»` mask token.
fn mask(label: &str) -> String {
    format!("«REDACTED:{label}»")
}

fn denylist_set() -> &'static RegexSet {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    SET.get_or_init(|| {
        RegexSet::new(DENYLIST.iter().map(|(_, p)| *p)).expect("static denylist patterns compile")
    })
}

fn denylist_regexes() -> &'static [Regex] {
    static RES: OnceLock<Vec<Regex>> = OnceLock::new();
    RES.get_or_init(|| {
        DENYLIST
            .iter()
            .map(|(_, p)| Regex::new(p).expect("static denylist pattern compiles"))
            .collect()
    })
}

fn entropy_run_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(&format!(r"[A-Za-z0-9+/=_-]{{{MIN_ENTROPY_RUN},}}"))
            .expect("static entropy-run regex compiles")
    })
}

fn git_sha_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[0-9a-fA-F]{40}$").expect("static git-sha regex compiles"))
}

fn uuid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")
            .expect("static uuid regex compiles")
    })
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;
