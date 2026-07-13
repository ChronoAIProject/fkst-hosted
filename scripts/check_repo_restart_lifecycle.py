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
)
SITE_KEYS = ("path", "symbol", "ordinal")
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
    boundary = record.get("boundary")
    if boundary not in BOUNDARIES:
        messages.append(f"{label}: boundary must be one of {sorted(BOUNDARIES)}")
    outcome = record.get("old_outcome")
    if not isinstance(outcome, dict):
        messages.append(f"{label}: old_outcome must be an object")
    else:
        for key in ("status", "reason_code", "cas_outcome", "emitted_effects"):
            if key not in outcome:
                messages.append(f"{label}.old_outcome: missing key {key}")
        emitted = outcome.get("emitted_effects")
        if not isinstance(emitted, list):
            messages.append(f"{label}.old_outcome.emitted_effects: must be an array")
        else:
            for effect_index, effect in enumerate(emitted):
                if not isinstance(effect, dict):
                    messages.append(f"{label}.old_outcome.emitted_effects[{effect_index}]: must be an object")
                    continue
                for key in ("effect_id", "sink_kind", "authority_class", "ordinal"):
                    if key not in effect:
                        messages.append(f"{label}.old_outcome.emitted_effects[{effect_index}]: missing key {key}")
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


def shrink_only_messages(root: Path, current_lines: list[str], enforce_base: bool = True) -> list[str]:
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
    return [f"{ALLOWLIST}: shrink-only violation; new unobserved site id {item}" for item in additions]


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
    messages.extend(shrink_only_messages(root, allowlist_lines, enforce_base))
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
