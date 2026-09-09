//! Tests for the knowledge base (sibling `#[path]` module).
//!
//! The drift guards are the point of this file. They import the backend's OWN constants —
//! never inlined copies, which would defeat the guard entirely — and assert the manual
//! mentions each one. Renaming a label or a trigger heading in the code then fails the
//! build until the manual is updated with it, so the concierge cannot quietly start
//! quoting a name the platform no longer uses.

use super::*;

use crate::goals::trigger_parse::{
    HEADING_AUTO_MERGE, HEADING_ENGINE_CONFIG, HEADING_ENVIRONMENT, HEADING_FKST_CONTRIBUTORS,
    HEADING_LOG_ACCESS, HEADING_MANIFEST, HEADING_OUTPUT_LANGUAGE, HEADING_PACKAGES,
    HEADING_SESSION_COLLABORATORS, HEADING_SESSION_NAME, HEADING_SOURCE_BRANCH,
    HEADING_TARGET_BRANCH, HEADING_WORK_LABEL, PACKAGE_REF_FORM,
};
use crate::reconcile::{
    SUBSTRATE_ANNOUNCED_LABEL, SUBSTRATE_CONFIG_REJECTED_LABEL, SUBSTRATE_DEGRADED_LABEL,
    SUBSTRATE_INVALID_LABEL, SUBSTRATE_RETIRED_LABEL, TRIGGER_UNAUTHORIZED_LABEL,
    WORK_PICKED_UP_LABEL, WORK_UNAUTHORIZED_LABEL, WORK_UNROUTED_LABEL,
};

/// Assert the manual mentions `needle`, naming the constant when it does not.
fn assert_documents(needle: &str, what: &str) {
    assert!(
        MANUAL.contains(needle),
        "{what} ({needle:?}) is missing from manual.md — the concierge would answer from \
         model priors instead of the real platform contract. Update \
         backend/src/chat/knowledge/manual.md."
    );
}

// ---- drift guards ---------------------------------------------------------

#[test]
fn the_manual_documents_every_status_label() {
    for (label, what) in [
        (SUBSTRATE_INVALID_LABEL, "the invalid-trigger label"),
        (TRIGGER_UNAUTHORIZED_LABEL, "the unauthorized-trigger label"),
        (SUBSTRATE_ANNOUNCED_LABEL, "the registered-session label"),
        (WORK_PICKED_UP_LABEL, "the picked-up label"),
        (WORK_UNAUTHORIZED_LABEL, "the unauthorized-work label"),
        (WORK_UNROUTED_LABEL, "the unrouted-work label"),
        (SUBSTRATE_RETIRED_LABEL, "the retired-session label"),
        (SUBSTRATE_DEGRADED_LABEL, "the degraded-health label"),
        (SUBSTRATE_CONFIG_REJECTED_LABEL, "the config-rejected label"),
    ] {
        assert_documents(label, what);
    }
}

#[test]
fn the_manual_documents_every_trigger_body_heading() {
    for heading in [
        HEADING_SESSION_NAME,
        HEADING_PACKAGES,
        HEADING_MANIFEST,
        HEADING_WORK_LABEL,
        HEADING_ENVIRONMENT,
        HEADING_AUTO_MERGE,
        HEADING_LOG_ACCESS,
        HEADING_FKST_CONTRIBUTORS,
        HEADING_SESSION_COLLABORATORS,
        HEADING_OUTPUT_LANGUAGE,
        HEADING_ENGINE_CONFIG,
        HEADING_SOURCE_BRANCH,
        HEADING_TARGET_BRANCH,
    ] {
        assert_documents(heading, "a trigger-body heading");
    }
}

#[test]
fn the_manual_documents_the_package_reference_grammar() {
    assert_documents(PACKAGE_REF_FORM, "the package-reference form");
}

#[test]
fn the_manual_documents_the_default_trigger_label() {
    // The label a user actually applies. Taken from the reconciler's configured default
    // rather than restated, so a rename is caught here.
    let default_trigger_label =
        crate::reconcile_config::ReconcileConfig::default().substrate_trigger_label;
    assert_documents(&default_trigger_label, "the trigger label");
}

#[test]
fn the_manual_documents_the_engine_config_model_knobs() {
    // These two are the ones users most often want and were missing from the older
    // documentation, so they are guarded explicitly.
    assert_documents("FKST_LLM_MODEL", "the per-session model knob");
    assert_documents(
        "FKST_LLM_REASONING_EFFORT",
        "the per-session reasoning-effort knob",
    );
}

#[test]
fn the_manual_states_that_work_authority_is_always_enforced() {
    // The older manual said the work-author check applies "only when the deployment
    // enforces work-issue authority". That conditional is outdated — there is no
    // enforcement toggle — and repeating it would tell users an unauthorized author
    // might still be worked.
    assert!(
        MANUAL.contains("always enforced"),
        "the manual must state that work-issue authority is always enforced"
    );
    assert!(
        !MANUAL.contains("only when the deployment enforces"),
        "the manual must not repeat the retired enforcement-toggle conditional"
    );
}

#[test]
fn the_manual_stays_deployment_agnostic() {
    // A hostname or cluster name in here would be wrong for every other deployment and
    // would leak operator detail into a user-facing answer.
    for forbidden in [
        "chronoai-fkst.local",
        "kind-opensandbox-local",
        "kubectl",
        "opensandbox-system",
    ] {
        assert!(
            !MANUAL.contains(forbidden),
            "manual.md must stay deployment-agnostic but mentions {forbidden:?}"
        );
    }
}

// ---- parsing --------------------------------------------------------------

#[test]
fn the_manual_parses_into_titled_slugged_sections() {
    let parsed = sections();
    assert!(
        parsed.len() >= 15,
        "the manual should split into its documented sections, got {}",
        parsed.len()
    );
    for section in parsed {
        assert!(!section.id.is_empty(), "{:?} needs a slug", section.title);
        assert!(
            !section.title.is_empty() && !section.body.is_empty(),
            "{:?} must have a title and a body",
            section.id
        );
        assert!(
            !section.body.starts_with("## "),
            "{:?} swallowed the next heading",
            section.id
        );
    }
}

#[test]
fn slugs_are_kebab_case_and_stable() {
    assert_eq!(slug("Session logs"), "session-logs");
    assert_eq!(
        slug("`### Engine Config` — allowlisted tunables"),
        "engine-config-allowlisted-tunables"
    );
    assert_eq!(
        slug("Package and manifest references — the `owner/repo@ref:path` grammar"),
        "package-and-manifest-references-the-owner-repo-ref-path-grammar"
    );
}

#[test]
fn the_preamble_is_not_a_section() {
    // The title and framing before the first `## ` are not an answer to anything.
    assert!(
        !toc()
            .iter()
            .any(|(_, title)| title.contains("concierge knowledge base")),
        "the document title must not become a section"
    );
}

#[test]
fn the_toc_lists_every_section_in_document_order() {
    let toc = toc();
    let parsed = sections();
    assert_eq!(toc.len(), parsed.len());
    for (entry, section) in toc.iter().zip(parsed.iter()) {
        assert_eq!(&entry.0, &section.id);
        assert_eq!(&entry.1, &section.title);
    }
}

// ---- lookup ---------------------------------------------------------------

/// The ids a query returns, in rank order.
fn ids(query: &str) -> Vec<String> {
    lookup(query, DEFAULT_MAX_SECTIONS, DEFAULT_MAX_BYTES)
        .into_iter()
        .map(|section| section.id.clone())
        .collect()
}

#[test]
fn an_unrouted_question_returns_the_sections_that_answer_it() {
    // Three sections carry this answer — the routing rule, the label table, and the
    // troubleshooting entry — and a default lookup returns all three, so the model sees
    // the whole answer regardless of their internal order. What must hold is that the
    // TOP hit already contains the actual rule, so a one-section lookup still answers.
    for query in [
        "unrouted",
        "why is my issue labeled fkst-unrouted",
        "fkst-unrouted",
    ] {
        let ranked = ids(query);
        assert!(
            ranked.iter().any(|id| id.contains("status-labels")),
            "{query:?} must surface the status-label table, got {ranked:?}"
        );
        let top = lookup(query, 1, DEFAULT_MAX_BYTES)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("{query:?} must match something"));
        assert!(
            top.body.contains("exactly one assignee"),
            "{query:?} ranked {:?} first, which does not carry the exactly-one-assignee \
             rule that answers it",
            top.id
        );
    }
}

#[test]
fn a_label_meaning_question_ranks_the_status_table_first() {
    // "what does fkst-degraded mean" — the label table IS the answer, and it must not be
    // outranked by a section that merely mentions health.
    let ranked = ids("what does fkst-degraded mean");
    assert!(
        ranked.first().expect("a match").contains("status-labels"),
        "got {ranked:?}"
    );
}

#[test]
fn an_environment_secrets_question_returns_the_environments_section() {
    let ranked = ids("environment secrets write-only");
    assert!(
        ranked.iter().any(|id| id.contains("environments")),
        "got {ranked:?}"
    );
}

#[test]
fn a_grammar_question_returns_the_reference_grammar_section() {
    let ranked = ids("package reference owner repo ref path grammar");
    assert!(
        ranked
            .iter()
            .any(|id| id.contains("package-and-manifest-references")),
        "got {ranked:?}"
    );
}

#[test]
fn a_heading_match_outranks_scattered_body_mentions() {
    // "logs" appears throughout the manual; the section titled for it must still win.
    let ranked = ids("session logs");
    assert!(
        ranked.first().expect("a match").contains("logs"),
        "got {ranked:?}"
    );
}

#[test]
fn the_section_cap_is_honored() {
    assert!(lookup("session", 1, DEFAULT_MAX_BYTES).len() <= 1);
    assert!(lookup("session", 2, DEFAULT_MAX_BYTES).len() <= 2);
}

#[test]
fn the_byte_budget_is_honored_but_never_returns_nothing() {
    // A budget smaller than any section still yields ONE section: half an answer is
    // worse than a long one, and zero results would read as "not documented".
    let sections = lookup("session logs", DEFAULT_MAX_SECTIONS, 10);
    assert_eq!(sections.len(), 1, "the best match is always returned");

    let generous = lookup("session logs", DEFAULT_MAX_SECTIONS, DEFAULT_MAX_BYTES);
    let total: usize = generous.iter().map(|s| s.body.len() + s.title.len()).sum();
    assert!(total <= DEFAULT_MAX_BYTES, "budget exceeded: {total}");
}

#[test]
fn an_unmatched_query_returns_nothing() {
    // The tool turns this into the table-of-contents fallback.
    assert!(ids("zzzznotatopic").is_empty());
}

#[test]
fn a_query_with_no_usable_terms_returns_nothing() {
    // Single characters and punctuation are not search terms.
    assert!(ids("a ? !").is_empty());
    assert!(ids("").is_empty());
}

#[test]
fn lookup_is_case_insensitive() {
    assert_eq!(ids("FKST-UNROUTED"), ids("fkst-unrouted"));
}
