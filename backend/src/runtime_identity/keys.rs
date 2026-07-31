//! The two metadata key sets, and the ONE renderer/parser pair they share.
//!
//! Kubernetes annotations and OpenSandbox metadata spell the same five facts
//! with different names — a dotted DNS-prefixed annotation key on one side, a
//! flat `fkst-` key on the other. Keeping the names in one file, behind one
//! [`IdentityKeys`] value each, is what makes a drift between the stamp and the
//! recovery impossible: both directions take the same [`IdentityKeys`] argument,
//! so a renamed key is renamed for both or for neither.
//!
//! ## Why annotations, never labels (Kubernetes)
//!
//! Attribution must not change selector or cardinality behaviour. Every existing
//! label on a session Pod is something the reconciler SELECTS on; a creator id
//! is something it DISPLAYS. Promoting attribution to a label would make every
//! distinct creator a new label value in the apiserver's index for no query that
//! anyone issues.
//!
//! ## Why metadata, never `extensions` (OpenSandbox)
//!
//! The upstream Sandbox response carries `metadata` but not `extensions` —
//! `extensions` is a create-REQUEST-only field, so anything put there can never
//! be read back (see [`crate::session_backend::opensandbox::backend::correlate`]).
//! Attribution that cannot be recovered is not attribution.

use std::collections::BTreeMap;

use super::{ObservedRuntimeIdentity, RuntimeIdentityMetadata};

/// The stamp contract version. Bump only alongside a new key layout; its
/// PRESENCE on a runtime is what separates a contract-stamped runtime from a
/// legacy one.
pub const IDENTITY_SCHEMA_VERSION: &str = "1";

/// The provenance value a LAUNCH stamp records.
///
/// It exists because the schema/attribution keys alone cannot answer the
/// question the epic insists be answered honestly: a backfill writes byte-for-
/// byte the same five keys a launch does, so without a durable marker a
/// legacy runtime patched from the trigger as it reads TODAY would later be
/// read back as "this is who launched it" — and, unlike the reconciler's
/// in-memory knowledge, that misreading survives a process restart.
pub const SOURCE_LAUNCH_METADATA: &str = "launch_metadata";

/// The provenance value a BACKFILL patch records. Deliberately not called
/// original/historical: a legacy runtime's trigger may have been edited or
/// re-assigned since launch, so evidence recovered now is honest only about
/// being current.
pub const SOURCE_BACKFILLED_CURRENT_TRIGGER: &str = "backfilled_current_trigger";

/// Which attribution fact disagreed, in a BACKEND-NEUTRAL spelling.
///
/// The conflict marker's VALUE cannot be the offending backend key string: a
/// Kubernetes annotation key (`fkst.chrono-ai.fun/creator-id`) contains a `/`,
/// which OpenSandbox metadata — bound by the Kubernetes label-VALUE contract —
/// rejects, and a per-backend spelling would make the same disagreement read
/// differently depending on where the session happened to run. These four names
/// are label-value-safe and identical on both backends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityField {
    Schema,
    CreatorId,
    CreatorLogin,
    TriggerAuthorId,
    TriggerAuthorLogin,
}

impl IdentityField {
    pub fn as_str(self) -> &'static str {
        match self {
            IdentityField::Schema => "identity-schema",
            IdentityField::CreatorId => "creator-id",
            IdentityField::CreatorLogin => "creator-login",
            IdentityField::TriggerAuthorId => "trigger-author-id",
            IdentityField::TriggerAuthorLogin => "trigger-author-login",
        }
    }
}

/// The metadata keys one backend spells its identity stamp with: the five
/// attribution keys plus the two markers that describe the stamp itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityKeys {
    pub schema: &'static str,
    pub creator_id: &'static str,
    pub creator_login: &'static str,
    pub trigger_author_id: &'static str,
    pub trigger_author_login: &'static str,
    /// Where this runtime's stamp came from — one of [`SOURCE_LAUNCH_METADATA`]
    /// / [`SOURCE_BACKFILLED_CURRENT_TRIGGER`]. Never an attribution value, so
    /// it never participates in the conflict comparison.
    pub source: &'static str,
    /// The DURABLE record that this runtime's stamp was observed to disagree
    /// with its trigger, holding the [`IdentityField`] that disagreed.
    ///
    /// It exists because a disagreement is otherwise knowable only to the process
    /// that compared the stamp against a freshly parsed registration — and the
    /// operations inventory ([`crate::session_backend::inventory`]) deliberately
    /// reads runtimes ALONE, with no registration to compare against. Without a
    /// durable marker a conflicted runtime would report `launch_metadata` to
    /// exactly the global admin the epic promises can identify it. Written only
    /// by the backfill sweep, only when absent, and never alongside an
    /// attribution value.
    pub conflict: &'static str,
}

impl IdentityKeys {
    /// Every key, in stamp order. Used by the round-trip tests and by the
    /// backfill planner, so a key added to the struct cannot be forgotten.
    pub fn all(&self) -> [&'static str; 7] {
        [
            self.schema,
            self.creator_id,
            self.creator_login,
            self.trigger_author_id,
            self.trigger_author_login,
            self.source,
            self.conflict,
        ]
    }
}

/// Kubernetes Pod ANNOTATION keys (never labels — see the module docs). The
/// `fkst.chrono-ai.fun/` prefix matches every other session-pod annotation.
pub const K8S_IDENTITY_KEYS: IdentityKeys = IdentityKeys {
    schema: "fkst.chrono-ai.fun/identity-schema-version",
    creator_id: "fkst.chrono-ai.fun/creator-id",
    creator_login: "fkst.chrono-ai.fun/creator-login",
    trigger_author_id: "fkst.chrono-ai.fun/trigger-author-id",
    trigger_author_login: "fkst.chrono-ai.fun/trigger-author-login",
    source: "fkst.chrono-ai.fun/identity-source",
    conflict: "fkst.chrono-ai.fun/identity-conflict",
};

/// OpenSandbox sandbox METADATA keys, sharing the flat `fkst-` convention of the
/// correlation keys already stamped there.
pub const OSB_IDENTITY_KEYS: IdentityKeys = IdentityKeys {
    schema: "fkst-identity-schema",
    creator_id: "fkst-creator-id",
    creator_login: "fkst-creator-login",
    trigger_author_id: "fkst-trigger-author-id",
    trigger_author_login: "fkst-trigger-author-login",
    source: "fkst-identity-source",
    conflict: "fkst-identity-conflict",
};

/// Render `identity` into `(key, value)` pairs for `keys`.
///
/// A value that cannot be stated is OMITTED rather than written as an empty or
/// placeholder string: an absent `creator-id` key is the explicit, recoverable
/// representation of an assignee-derived creator, and an empty string would be
/// both an invalid Kubernetes label value and a lie about what is known.
///
/// [`IdentityKeys::conflict`] is never rendered here: a launch stamp is the
/// first word on a runtime's attribution and therefore cannot disagree with
/// anything yet.
pub fn stamp_pairs(
    keys: &IdentityKeys,
    identity: &RuntimeIdentityMetadata,
) -> Vec<(&'static str, String)> {
    let mut pairs = vec![
        (keys.schema, IDENTITY_SCHEMA_VERSION.to_string()),
        // Written at launch, by the only writer that can truthfully claim it.
        (keys.source, SOURCE_LAUNCH_METADATA.to_string()),
    ];
    if let Some(creator_id) = identity.creator_id {
        pairs.push((keys.creator_id, creator_id.to_string()));
    }
    if !identity.creator_login.is_empty() {
        pairs.push((keys.creator_login, identity.creator_login.clone()));
    }
    pairs.push((
        keys.trigger_author_id,
        identity.trigger_author_id.to_string(),
    ));
    if !identity.trigger_author_login.is_empty() {
        pairs.push((
            keys.trigger_author_login,
            identity.trigger_author_login.clone(),
        ));
    }
    pairs
}

/// Recover the identity stamp from a runtime's metadata map. The exact inverse
/// of [`stamp_pairs`] for the same `keys`.
///
/// An id key holding a non-integer value sets
/// [`ObservedRuntimeIdentity::malformed`] instead of silently reading as absent:
/// a corrupted stamp must never be mistaken for the legitimate "assignee-derived
/// creator has no id" state, which the backfill planner treats very differently.
///
/// [`ObservedRuntimeIdentity::conflicting`] comes from the durable conflict
/// marker, which is what lets a reader holding ONLY the runtime — the operations
/// inventory — report a disagreement that a long-gone reconcile pass detected.
pub fn read(keys: &IdentityKeys, metadata: &BTreeMap<String, String>) -> ObservedRuntimeIdentity {
    let mut malformed = false;
    let mut id = |key: &str| -> Option<i64> {
        let raw = metadata.get(key)?;
        match raw.parse::<i64>() {
            Ok(value) => Some(value),
            Err(_) => {
                malformed = true;
                None
            }
        }
    };
    let creator_id = id(keys.creator_id);
    let trigger_author_id = id(keys.trigger_author_id);
    ObservedRuntimeIdentity {
        schema_version: metadata.get(keys.schema).cloned(),
        creator_id,
        creator_login: non_empty(metadata.get(keys.creator_login)),
        trigger_author_id,
        trigger_author_login: non_empty(metadata.get(keys.trigger_author_login)),
        source: non_empty(metadata.get(keys.source)),
        conflicting: non_empty(metadata.get(keys.conflict)).is_some(),
        malformed,
    }
}

/// A stamped value only counts when it says something; a blank annotation is
/// indistinguishable from an absent one for attribution purposes.
fn non_empty(value: Option<&String>) -> Option<String> {
    value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
