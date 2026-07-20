//! The work-label SET ↔ single wire-value codec (epic #594 I4).
//!
//! A substrate session's EFFECTIVE work-label set (its explicit `### Work Label` ∪ the
//! labels its packages auto-declare) is carried through the pod as ONE value —
//! [`crate::k8s::SessionPodSpec::work_label`], which feeds the
//! `fkst.chrono-ai.fun/work-label` pod annotation, `FKST_GITHUB_PROXY_POLL_LABEL_PREFIX`,
//! and `FKST_SESSION_WORK_LABEL`. github-proxy's `github_proxy_poll_label_prefixes`
//! comma-splits `FKST_GITHUB_PROXY_POLL_LABEL_PREFIX` back into its (deduped) poll-label
//! list, so a **comma-joined** value makes the session wake on ANY of its labels.
//!
//! This module is the single source of truth for that encoding: [`join_work_labels`]
//! (write side — the reconcile executor builds the spec) and [`split_work_labels`]
//! (read side — the observe-side projections `pod_to_live` / `to_live_pod` recover the
//! set an orphaned session must retire its work issues across). Keeping the pair here —
//! pure, standalone, and exhaustively unit-tested — guarantees the two never drift and
//! that a single-label session round-trips byte-identically to the pre-multi-label value.

/// The separator joining an effective work-label SET into the single
/// [`crate::k8s::SessionPodSpec::work_label`] wire value. A comma because github-proxy
/// comma-splits `FKST_GITHUB_PROXY_POLL_LABEL_PREFIX` into its poll-label list.
const WORK_LABEL_SEP: &str = ",";

/// Join a session's effective work-label set into the single comma-separated value
/// carried on [`crate::k8s::SessionPodSpec::work_label`]. First-occurrence order is
/// preserved and blank / duplicate tokens are dropped, so a single-label set renders
/// byte-identically to the pre-multi-label value. Paired with [`split_work_labels`] (the
/// observe-side inverse) so the set round-trips through the annotation. An EMPTY set
/// yields an empty string — the reconciler rejects a label-less session upstream, so a
/// spawned pod never sees one (the in-pod `FKST_SESSION_WORK_LABEL` guard is the
/// defensive backstop).
pub(crate) fn join_work_labels(labels: &[String]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for label in labels {
        let trimmed = label.trim();
        if !trimmed.is_empty() && !seen.contains(&trimmed) {
            seen.push(trimmed);
        }
    }
    seen.join(WORK_LABEL_SEP)
}

/// Split a comma-joined work-label value (the pod annotation / correlation metadata)
/// back into its individual labels, dropping blank tokens. The inverse of
/// [`join_work_labels`]; used by the observe-side projections (`pod_to_live` /
/// `to_live_pod`) to recover the FULL set an orphaned session must retire its work
/// issues across.
pub(crate) fn split_work_labels(value: &str) -> Vec<String> {
    value
        .split(WORK_LABEL_SEP)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_work_labels_dedups_preserving_order_and_drops_blanks() {
        // First-occurrence order preserved; blanks + duplicates dropped; single = bare.
        assert_eq!(join_work_labels(&["a".to_string()]), "a");
        assert_eq!(
            join_work_labels(&["a".to_string(), "b".to_string(), "a".to_string()]),
            "a,b"
        );
        assert_eq!(
            join_work_labels(&["  x  ".to_string(), "".to_string(), "y".to_string()]),
            "x,y"
        );
        assert_eq!(
            join_work_labels(&[]),
            "",
            "an empty set joins to a blank string"
        );
    }

    #[test]
    fn split_work_labels_is_the_inverse_of_join() {
        assert_eq!(split_work_labels("a"), vec!["a".to_string()]);
        assert_eq!(
            split_work_labels("a,b"),
            vec!["a".to_string(), "b".to_string()]
        );
        // Blank tokens (a trailing/empty comma) are dropped; a blank value → empty set.
        assert_eq!(
            split_work_labels("a,,b, "),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(split_work_labels("").is_empty());
        // Round-trip: join then split recovers the deduped, ordered set.
        let set = vec!["one".to_string(), "two".to_string(), "three".to_string()];
        assert_eq!(split_work_labels(&join_work_labels(&set)), set);
    }
}
