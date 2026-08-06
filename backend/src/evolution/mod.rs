//! FKST Evolution — continuous, repository-native product-artifact synchronisation.
//!
//! Evolution keeps a repository's product-facing materials aligned with the
//! latest trusted state of its source: documentation, product-operation skills,
//! executable journeys, screenshots, demo video, release narratives and slide
//! decks, all derived from one structured product observation.
//!
//! Two properties shape every module here.
//!
//! **GitHub is the only durable store.** There is no Evolution database. The
//! manifest, the product model, the coordination issues and pull requests, and
//! the artifact Releases are all repository resources. A complete loss of this
//! process's state must not lose work or change the outcome.
//!
//! **Reconciliation is level-triggered.** A webhook is a best-effort hint (see
//! [`crate::routes::github_app_webhook`]); correctness comes from re-reading
//! GitHub and comparing fingerprints. Startup and periodic full resynchronisation
//! repair missed, duplicated, delayed or out-of-order deliveries.
//!
//! The generator itself — the thing that produces artifact bytes — is not here.
//! It runs in a session sandbox from packaged producer roles. This module is the
//! control plane's half: enrollment, the singleton coordination lane, the merge
//! gate, and the convergence decision.

pub mod config;
pub mod lane;
