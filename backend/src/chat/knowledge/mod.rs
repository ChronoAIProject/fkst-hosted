//! The concierge's grounded knowledge base: a compiled-in operator manual, split into
//! searchable sections.
//!
//! Why compiled in rather than fetched or embedded in a vector store: the platform's
//! rules are precise and small. Their failure modes are exact (a second assignee
//! produces `fkst-unrouted`, not "something like that"), so the concierge must quote
//! them rather than paraphrase model priors — and a few hundred lines of prose needs no
//! retrieval infrastructure to search well. Refreshing the manual is a normal pull
//! request, which also means it is reviewed like any other content.
//!
//! The drift guard is the load-bearing part: `knowledge_tests.rs` imports the backend's
//! own label and trigger-heading constants and asserts the manual mentions every one of
//! them. So renaming a label or a heading in the code fails the build until the manual
//! is updated with it — the manual cannot quietly go stale.

use std::sync::OnceLock;

/// The curated manual. An embedded DATA asset, not source code: it is reviewed as
/// content, and the 500-line source-file rule does not apply to it. If it grows past
/// roughly a thousand lines, split it into part files concatenated here rather than
/// letting this module grow.
const MANUAL: &str = include_str!("manual.md");

/// Default number of sections [`lookup`] returns. Three is enough to cover a question
/// that spans two topics without crowding out the live tool results in the same turn.
pub const DEFAULT_MAX_SECTIONS: usize = 3;

/// Default byte budget for one lookup's combined sections.
pub const DEFAULT_MAX_BYTES: usize = 24 * 1024;

/// One `## ` section of the manual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// A kebab-case slug of the heading — stable enough to cite in an answer.
    pub id: String,
    pub title: String,
    /// The section body, heading excluded.
    pub body: String,
}

/// Parse the manual once, on first use.
fn sections() -> &'static Vec<Section> {
    static SECTIONS: OnceLock<Vec<Section>> = OnceLock::new();
    SECTIONS.get_or_init(|| parse(MANUAL))
}

/// Split the manual on `## ` headings. Content before the first one (the title and
/// preamble) is deliberately dropped: it is framing, not an answer.
fn parse(manual: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    // A leading marker so the first section is found by the same split as the rest.
    let normalized = format!("\n{manual}");
    for chunk in normalized.split("\n## ").skip(1) {
        let (heading, body) = match chunk.split_once('\n') {
            Some((heading, body)) => (heading.trim(), body),
            None => (chunk.trim(), ""),
        };
        if heading.is_empty() {
            continue;
        }
        sections.push(Section {
            id: slug(heading),
            title: heading.to_string(),
            body: body.trim_end().to_string(),
        });
    }
    sections
}

/// Kebab-case a heading: lowercase alphanumerics, every run of anything else a single
/// dash. Punctuation-heavy headings ("`### Engine Config` — allowlisted tunables") still
/// produce a readable slug.
fn slug(heading: &str) -> String {
    let mut out = String::with_capacity(heading.len());
    let mut pending_dash = false;
    for ch in heading.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    out
}

/// Every section's `(id, title)`, in document order.
///
/// Embedded in the system prompt so the model knows what the manual can answer before
/// it searches — which stops it guessing at a topic the manual covers.
pub fn toc() -> Vec<(String, String)> {
    sections()
        .iter()
        .map(|section| (section.id.clone(), section.title.clone()))
        .collect()
}

/// A section's searchable text, lowercased once at startup rather than per query.
struct Index {
    title: String,
    body: String,
}

fn index() -> &'static Vec<Index> {
    static INDEX: OnceLock<Vec<Index>> = OnceLock::new();
    INDEX.get_or_init(|| {
        sections()
            .iter()
            .map(|section| Index {
                title: section.title.to_lowercase(),
                body: section.body.to_lowercase(),
            })
            .collect()
    })
}

/// Words carrying no search signal, dropped before scoring.
///
/// Users ask questions ("what does fkst-degraded mean?", "how do I start a session?"), and
/// question words are common enough to survive the rarity weighting while still steering
/// the ranking — "what" in a section heading would otherwise outrank the label the user
/// actually named. A fixed list is the right tool here because the corpus is one small,
/// fixed English document.
const STOPWORDS: &[&str] = &[
    "about", "all", "an", "and", "any", "are", "as", "at", "be", "been", "but", "by", "can", "did",
    "do", "does", "for", "from", "get", "has", "have", "how", "if", "in", "into", "is", "it",
    "its", "me", "mean", "means", "my", "no", "not", "of", "on", "or", "should", "so", "that",
    "the", "their", "them", "then", "there", "these", "they", "this", "to", "use", "was", "were",
    "what", "when", "where", "which", "who", "why", "will", "with", "would", "you", "your",
];

/// Fixed-point scale for scores, so ranking stays integer-ordered while term weights
/// are fractional.
const SCALE: u64 = 1000;
/// Weight of a term appearing in a section's HEADING — a heading match usually means the
/// whole section is the answer.
const TITLE_WEIGHT: u64 = 20;
/// Weight of the caller's whole phrase appearing verbatim in a body.
const PHRASE_WEIGHT: u64 = 12;
/// Body hits counted per term. A section mentioning a term ten times is not ten times
/// more relevant than one mentioning it three times.
const MAX_BODY_HITS: u64 = 5;

/// Search the manual for `query`, best match first.
///
/// Scoring is simple and explainable rather than clever, with one refinement that
/// matters: each term is weighted by how RARE it is across the manual. Without that, a
/// question like "why is my issue labeled fkst-unrouted" ranks on the ubiquitous word
/// "issue" and returns the trigger-grammar section instead of the label section that
/// actually answers it. Rare terms are the ones carrying the user's intent.
///
/// Ties keep document order, so related sections read in the order they were written.
pub fn lookup(query: &str, max_sections: usize, max_bytes: usize) -> Vec<&'static Section> {
    let needle = query.to_lowercase();
    let terms: Vec<&str> = needle
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1 && !STOPWORDS.contains(t))
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }

    let corpus = index();
    let total = corpus.len() as u64;
    // Per-term rarity: `total / document_frequency`, fixed-point. A term in every
    // section weighs 1×; a term in two of twenty weighs 10×. `discriminating` gates the
    // heading bonus to terms that actually narrow the search — without it, a heading
    // containing a ubiquitous word like "issue" outranks the section that holds the
    // rare term the user really asked about.
    let weights: Vec<TermWeight> = terms
        .iter()
        .map(|term| {
            let df = corpus
                .iter()
                .filter(|entry| entry.title.contains(term) || entry.body.contains(term))
                .count() as u64;
            TermWeight {
                // 0 for an absent term (df == 0), which `score` then skips.
                rarity: (total * SCALE).checked_div(df).unwrap_or(0),
                discriminating: df > 0 && df * 2 <= total,
            }
        })
        .collect();

    let mut scored: Vec<(u64, usize, &'static Section)> = sections()
        .iter()
        .zip(corpus.iter())
        .enumerate()
        .filter_map(|(position, (section, entry))| {
            let score = score(entry, &needle, &terms, &weights);
            (score > 0).then_some((score, position, section))
        })
        .collect();
    // Highest score first; document order breaks ties.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut chosen = Vec::new();
    let mut bytes = 0usize;
    for (_, _, section) in scored.into_iter().take(max_sections) {
        // Stop before exceeding the budget rather than returning a half section: a
        // truncated rule reads as a complete one, which is worse than a missing one.
        // The best match is always returned, budget or not — zero results would read as
        // "not documented".
        let cost = section.body.len() + section.title.len();
        if !chosen.is_empty() && bytes + cost > max_bytes {
            break;
        }
        bytes += cost;
        chosen.push(section);
    }
    chosen
}

/// How much one query term counts, and whether it may claim the heading bonus.
struct TermWeight {
    /// Fixed-point `total_sections / document_frequency`; 0 when the term is absent.
    rarity: u64,
    /// True when the term appears in at most half the sections.
    discriminating: bool,
}

/// Score one section against a query.
fn score(entry: &Index, phrase: &str, terms: &[&str], weights: &[TermWeight]) -> u64 {
    let mut score = 0u64;
    for (term, weight) in terms.iter().zip(weights.iter()) {
        if weight.rarity == 0 {
            continue;
        }
        if weight.discriminating && entry.title.contains(term) {
            score += TITLE_WEIGHT * weight.rarity;
        }
        let hits = (entry.body.matches(term).count() as u64).min(MAX_BODY_HITS);
        score += hits * weight.rarity;
    }
    if entry.body.contains(phrase) {
        score += PHRASE_WEIGHT * SCALE;
    }
    score
}

#[cfg(test)]
#[path = "knowledge_tests.rs"]
mod tests;
