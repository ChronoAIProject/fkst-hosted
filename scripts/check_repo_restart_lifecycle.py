#!/usr/bin/env python3
"""Ratchet for independent OLD restart lifecycle inventory."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

import ratchet_base

INVENTORY = "migration/restart-lifecycle.inventory.json"
ALLOWLIST = "migration/restart-lifecycle.allowlist"
SCHEMA = "restart-lifecycle.inventory.v1"
OBS_SCHEMA = "restart-old-behavior-observation.v2"
NEW_MODULE_TOKENS = (
    "restart_edges",
    "restart_cas_catalog",
    "restart_authority",
    "restart_effect_entitlements",
    "restart_owner_pending_projection",
)
TOP_LEVEL_KEYS = (
    "schema",
    "version",
    "source_tree",
    "old_behavior_observations",
    "old_pending_projection",
    "production_writer_sites",
    "effect_sink_sites",
    "row_replay_sites",
    "published_intent_sites",
    "receiver_activation_acceptors",
    "consumer_entry_acceptors",
    "direct_constructor_sites",
    "shared_issue_row_exports",
    "ops_issue_row_reader_sites",
    "owner_observation_fact_sites",
    "grantless_sink_sites",
    "unobserved_sites",
    "watched_files",
    "artifact_sha256",
)
OBSERVATION_KEYS = (
    "schema",
    "observation_id",
    "owner",
    "site",
    "boundary",
    "typed_intent",
    "old_inputs",
    "old_outcome",
    "evidence_refs",
)
SITE_KEYS = ("path", "symbol", "ordinal")
TYPED_INTENT_KEYS = (
    "kind",
    "source_state",
    "source_boundary",
    "target",
    "cause_schema_id",
    "generation_epoch",
    "lineage",
)
OLD_INPUT_KEYS = (
    "current_fact",
    "caller_from_states",
    "incoming_version",
    "target_version",
    "handoff_reference",
)
OLD_OUTCOME_KEYS = (
    "status",
    "reason_code",
    "cas_outcome",
    "emitted_effects",
    "observable_writes",
    "handoff_direct_lookup_count",
    "timeout_evidence_source",
)
EMITTED_EFFECT_KEYS = ("effect_id", "sink_kind", "authority_class", "ordinal")
UNOBSERVED_SITE_KEYS = ("site_id", "category", "path", "symbol", "ordinal", "why")
ENUMERATED_SITE_LIST_KEYS = (
    "production_writer_sites",
    "effect_sink_sites",
    "row_replay_sites",
    "published_intent_sites",
    "receiver_activation_acceptors",
    "consumer_entry_acceptors",
    "direct_constructor_sites",
    "shared_issue_row_exports",
    "ops_issue_row_reader_sites",
    "owner_observation_fact_sites",
    "grantless_sink_sites",
)

BOUNDARIES = {
    "writer",
    "effect_sink",
    "receiver_activation",
    "entry_acceptor",
    "published_intent_producer",
    "row_replay",
    "shared_row_export",
    "owner_observation_fact",
    "observation_fact_reader",
}
SINK_KINDS = {"queue", "comment", "label", "adapter", "codex", "git", "merge"}
AUTHORITY_CLASSES = {
    "lifecycle-authoritative",
    "grantless-published-intent",
    "grantless-telemetry",
    "grantless-non-lifecycle",
}


def canonical_json_bytes(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def artifact_sha256_for_document(document: dict[str, Any]) -> str:
    body = dict(document)
    body.pop("artifact_sha256", None)
    return hashlib.sha256(canonical_json_bytes(body)).hexdigest()


def load_inventory(root: Path) -> dict[str, Any]:
    path = root / INVENTORY
    return json.loads(path.read_text(encoding="utf-8"))


def validate_top_level(inventory: dict[str, Any]) -> list[str]:
    messages: list[str] = []
    for key in TOP_LEVEL_KEYS:
        if key not in inventory:
            messages.append(f"{INVENTORY}: missing top-level key {key}")
    unknown = sorted(set(inventory.keys()) - set(TOP_LEVEL_KEYS))
    for key in unknown:
        messages.append(f"{INVENTORY}: unknown top-level key {key}")
    return messages


def validate_closed_object(value: Any, keys: tuple[str, ...], label: str) -> list[str]:
    if not isinstance(value, dict):
        return [f"{label}: must be an object"]
    messages: list[str] = []
    for key in keys:
        if key not in value:
            messages.append(f"{label}: missing key {key}")
    for key in sorted(set(value.keys()) - set(keys)):
        messages.append(f"{label}: unknown key {key}")
    return messages


def validate_observation(record: dict[str, Any], index: int) -> list[str]:
    label = f"{INVENTORY}: old_behavior_observations[{index}]"
    messages: list[str] = []
    for key in OBSERVATION_KEYS:
        if key not in record:
            messages.append(f"{label}: missing key {key}")
    unknown = sorted(set(record.keys()) - set(OBSERVATION_KEYS) - {"edge_id"})
    for key in unknown:
        messages.append(f"{label}: unknown key {key}")
    if messages:
        return messages
    if record.get("schema") != OBS_SCHEMA:
        messages.append(f"{label}: schema must be {OBS_SCHEMA}")
    for key in ("observation_id", "owner"):
        if not isinstance(record.get(key), str) or not record.get(key):
            messages.append(f"{label}.{key}: must be a non-empty string")
    if "edge_id" in record:
        messages.append(f"{label}: edge_id is forbidden; OLD observation must stay independent of NEW edge identities")
    site = record.get("site")
    if not isinstance(site, dict):
        messages.append(f"{label}: site must be an object")
    else:
        unknown_site_keys = sorted(set(site.keys()) - set(SITE_KEYS))
        for key in unknown_site_keys:
            messages.append(f"{label}.site: unknown key {key}")
        for key in SITE_KEYS:
            if key not in site:
                messages.append(f"{label}.site: missing key {key}")
            elif not isinstance(site[key], str) or not site[key]:
                messages.append(f"{label}.site.{key}: must be a non-empty string")
    boundary = record.get("boundary")
    if boundary not in BOUNDARIES:
        messages.append(f"{label}: boundary must be one of {sorted(BOUNDARIES)}")

    typed_intent = record.get("typed_intent")
    messages.extend(validate_closed_object(typed_intent, TYPED_INTENT_KEYS, f"{label}.typed_intent"))
    if isinstance(typed_intent, dict):
        for key in ("kind", "cause_schema_id"):
            if not isinstance(typed_intent.get(key), str) or not typed_intent.get(key):
                messages.append(f"{label}.typed_intent.{key}: must be a non-empty string")
        for key in ("source_state", "source_boundary", "target"):
            if typed_intent.get(key) is not None and not isinstance(typed_intent.get(key), str):
                messages.append(f"{label}.typed_intent.{key}: must be a string or null")
        for key in ("generation_epoch", "lineage"):
            if not isinstance(typed_intent.get(key), dict):
                messages.append(f"{label}.typed_intent.{key}: must be an object")

    old_inputs = record.get("old_inputs")
    messages.extend(validate_closed_object(old_inputs, OLD_INPUT_KEYS, f"{label}.old_inputs"))
    if isinstance(old_inputs, dict):
        if not isinstance(old_inputs.get("current_fact"), dict):
            messages.append(f"{label}.old_inputs.current_fact: must be an object")
        if not isinstance(old_inputs.get("caller_from_states"), (dict, list)):
            messages.append(f"{label}.old_inputs.caller_from_states: must be an array or object")
        for key in ("incoming_version", "target_version"):
            if not isinstance(old_inputs.get(key), str) or not old_inputs.get(key):
                messages.append(f"{label}.old_inputs.{key}: must be a non-empty string")
        handoff_reference = old_inputs.get("handoff_reference")
        if handoff_reference is not None and not isinstance(handoff_reference, dict):
            messages.append(f"{label}.old_inputs.handoff_reference: must be an object or null")

    outcome = record.get("old_outcome")
    messages.extend(validate_closed_object(outcome, OLD_OUTCOME_KEYS, f"{label}.old_outcome"))
    if isinstance(outcome, dict):
        for key in ("status", "reason_code", "cas_outcome"):
            if not isinstance(outcome.get(key), str) or not outcome.get(key):
                messages.append(f"{label}.old_outcome.{key}: must be a non-empty string")
        emitted = outcome.get("emitted_effects")
        if not isinstance(emitted, list):
            messages.append(f"{label}.old_outcome.emitted_effects: must be an array")
        else:
            for effect_index, effect in enumerate(emitted):
                if not isinstance(effect, dict):
                    messages.append(f"{label}.old_outcome.emitted_effects[{effect_index}]: must be an object")
                    continue
                effect_label = f"{label}.old_outcome.emitted_effects[{effect_index}]"
                messages.extend(validate_closed_object(effect, EMITTED_EFFECT_KEYS, effect_label))
                if not isinstance(effect.get("effect_id"), str) or not effect.get("effect_id"):
                    messages.append(f"{effect_label}.effect_id: must be a non-empty string")
                if effect.get("sink_kind") not in SINK_KINDS:
                    messages.append(f"{effect_label}.sink_kind: must be one of {sorted(SINK_KINDS)}")
                if effect.get("authority_class") not in AUTHORITY_CLASSES:
                    messages.append(f"{effect_label}.authority_class: must be one of {sorted(AUTHORITY_CLASSES)}")
                if type(effect.get("ordinal")) is not int or effect["ordinal"] < 1:
                    messages.append(f"{effect_label}.ordinal: must be a positive integer")
        if not isinstance(outcome.get("observable_writes"), (dict, list)):
            messages.append(f"{label}.old_outcome.observable_writes: must be an array or object")
        lookup_count = outcome.get("handoff_direct_lookup_count")
        if type(lookup_count) is not int or lookup_count < 0:
            messages.append(f"{label}.old_outcome.handoff_direct_lookup_count: must be a non-negative integer")
        timeout_source = outcome.get("timeout_evidence_source")
        if timeout_source is not None and not isinstance(timeout_source, str):
            messages.append(f"{label}.old_outcome.timeout_evidence_source: must be a string or null")

    evidence_refs = record.get("evidence_refs")
    if not isinstance(evidence_refs, (dict, list)) or not evidence_refs:
        messages.append(f"{label}.evidence_refs: must be a non-empty array or object")
    return messages


def inventory_references_new_modules(inventory: Any) -> list[str]:
    messages: list[str] = []

    def walk(value: Any, path: str) -> None:
        if isinstance(value, dict):
            for key, nested in value.items():
                walk(nested, f"{path}.{key}" if path else str(key))
            return
        if isinstance(value, list):
            for idx, nested in enumerate(value):
                walk(nested, f"{path}[{idx}]")
            return
        if isinstance(value, str):
            for token in NEW_MODULE_TOKENS:
                if token in value:
                    messages.append(f"{INVENTORY}: NEW module token '{token}' is forbidden in inventory source data ({path})")

    walk(inventory, "")
    return messages


def load_allowlist(path: Path) -> list[str]:
    if not path.exists():
        return []
    return [line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def validate_site_provenance(site: dict[str, Any], label: str, root: Path) -> list[str]:
    messages: list[str] = []
    path_value = site.get("path")
    symbol_value = site.get("symbol")
    site_id = site.get("site_id", label)
    if not isinstance(path_value, str) or not path_value:
        return messages
    path = root / path_value
    if not path.exists():
        messages.append(f"{INVENTORY}: site_id {site_id}: path does not exist: {path_value}")
        return messages
    if not isinstance(symbol_value, str) or not symbol_value:
        messages.append(f"{INVENTORY}: site_id {site_id}: symbol must be a non-empty string")
        return messages
    content = path.read_text(encoding="utf-8")
    if symbol_value not in content:
        messages.append(f"{INVENTORY}: site_id {site_id}: symbol not found in {path_value}: {symbol_value}")
    return messages


def validate_enumerated_site_lists(inventory: dict[str, Any], root: Path) -> list[str]:
    messages: list[str] = []
    for key in ENUMERATED_SITE_LIST_KEYS:
        value = inventory.get(key)
        if not isinstance(value, list):
            messages.append(f"{INVENTORY}: {key} must be an array")
            continue
        for index, site in enumerate(value):
            label = f"{INVENTORY}: {key}[{index}]"
            if not isinstance(site, dict):
                messages.append(f"{label} must be an object")
                continue
            messages.extend(validate_site_provenance(site, label, root))
    return messages


def validate_allowlist_alignment(inventory: dict[str, Any], allowlist_lines: list[str], root: Path) -> list[str]:
    messages: list[str] = []
    unobserved = inventory.get("unobserved_sites")
    if not isinstance(unobserved, list):
        return [f"{INVENTORY}: unobserved_sites must be an array"]
    inventory_ids: list[str] = []
    for index, site in enumerate(unobserved):
        if not isinstance(site, dict):
            messages.append(f"{INVENTORY}: unobserved_sites[{index}] must be an object")
            continue
        unknown = sorted(set(site.keys()) - set(UNOBSERVED_SITE_KEYS))
        for key in unknown:
            messages.append(f"{INVENTORY}: unobserved_sites[{index}] unknown key {key}")
        for key in UNOBSERVED_SITE_KEYS:
            if key not in site:
                messages.append(f"{INVENTORY}: unobserved_sites[{index}] missing key {key}")
        site_id = site.get("site_id")
        if isinstance(site_id, str) and site_id:
            inventory_ids.append(site_id)
        messages.extend(validate_site_provenance(site, f"{INVENTORY}: unobserved_sites[{index}]", root))
    allowlist_set = set(allowlist_lines)
    inventory_set = set(inventory_ids)
    missing = sorted(inventory_set - allowlist_set)
    extra = sorted(allowlist_set - inventory_set)
    for item in missing:
        messages.append(f"{ALLOWLIST}: missing unobserved site id {item}")
    for item in extra:
        messages.append(f"{ALLOWLIST}: unknown site id {item}")
    return messages


def site_identity(site: dict[str, Any]) -> tuple[str, str, str] | None:
    values = tuple(site.get(key) for key in SITE_KEYS)
    if not all(isinstance(value, str) and value for value in values):
        return None
    path, symbol, ordinal = values
    return path, symbol, ordinal


def enumerated_site_identities(inventory: dict[str, Any]) -> dict[str, set[tuple[str, str, str]]]:
    identities: dict[str, set[tuple[str, str, str]]] = {}
    for key in ENUMERATED_SITE_LIST_KEYS:
        sites = inventory.get(key)
        if not isinstance(sites, list):
            continue
        for site in sites:
            if not isinstance(site, dict):
                continue
            site_id = site.get("site_id")
            if isinstance(site_id, str) and site_id:
                identity = site_identity(site)
                if identity is not None:
                    identities.setdefault(site_id, set()).add(identity)
    return identities


def observed_site_identities(inventory: dict[str, Any]) -> set[tuple[str, str, str]]:
    identities: set[tuple[str, str, str]] = set()
    observations = inventory.get("old_behavior_observations")
    if not isinstance(observations, list):
        return identities
    for record in observations:
        if isinstance(record, dict) and isinstance(record.get("site"), dict):
            identity = site_identity(record["site"])
            if identity is not None:
                identities.add(identity)
    return identities


def shrink_only_messages(
    root: Path,
    current_lines: list[str],
    inventory: dict[str, Any],
    enforce_base: bool = True,
) -> list[str]:
    if not enforce_base:
        return []
    status, base_text = ratchet_base.file_at_base(root, ALLOWLIST)
    if status == "absent":
        return []
    if status == "unresolved":
        return [f"{ALLOWLIST}: cannot resolve dev base allowlist to enforce shrink-only ratchet; ensure CI provides the dev ref"]
    base_lines = {line.strip() for line in (base_text or "").splitlines() if line.strip()}
    current_set = set(current_lines)
    additions = sorted(current_set - base_lines)
    messages = [f"{ALLOWLIST}: shrink-only violation; new unobserved site id {item}" for item in additions]
    enumerated = enumerated_site_identities(inventory)
    observed = observed_site_identities(inventory)
    for item in sorted(base_lines - current_set):
        identities = enumerated.get(item, set())
        if len(identities) != 1:
            messages.append(
                f"{ALLOWLIST}: removed site {item} has no unique enumerated (path, symbol, ordinal) identity; "
                "missing matching old behavior observation evidence"
            )
            continue
        identity = next(iter(identities))
        if identity not in observed:
            path, symbol, ordinal = identity
            messages.append(
                f"{ALLOWLIST}: removed site {item} (path={path}, symbol={symbol}, ordinal={ordinal}) "
                "is missing matching old behavior observation evidence in old_behavior_observations"
            )
    return messages


def repository_messages(root: Path, enforce_base: bool = True) -> list[str]:
    if not (root / INVENTORY).exists():
        return []
    inventory = load_inventory(root)
    messages = validate_top_level(inventory)
    if inventory.get("schema") != SCHEMA:
        messages.append(f"{INVENTORY}: schema must be {SCHEMA}")
    for index, record in enumerate(inventory.get("old_behavior_observations", [])):
        if not isinstance(record, dict):
            messages.append(f"{INVENTORY}: old_behavior_observations[{index}] must be an object")
            continue
        messages.extend(validate_observation(record, index))
    messages.extend(inventory_references_new_modules(inventory))
    allowlist_lines = load_allowlist(root / ALLOWLIST)
    messages.extend(validate_enumerated_site_lists(inventory, root))
    messages.extend(validate_allowlist_alignment(inventory, allowlist_lines, root))
    messages.extend(shrink_only_messages(root, allowlist_lines, inventory, enforce_base))
    expected_sha = artifact_sha256_for_document(inventory)
    actual_sha = inventory.get("artifact_sha256")
    if actual_sha != expected_sha:
        messages.append(f"{INVENTORY}: artifact_sha256 mismatch (expected {expected_sha}, found {actual_sha})")
    return messages


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    messages = repository_messages(root)
    if messages:
        for message in messages:
            print(message)
        return 1
    print("OK: restart lifecycle inventory is schema-valid, shrink-only, independent, and self-hash-matched")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
