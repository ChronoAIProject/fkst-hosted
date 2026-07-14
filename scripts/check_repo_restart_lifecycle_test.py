#!/usr/bin/env python3
from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import check_repo_restart_lifecycle as ratchet


class RestartLifecycleRatchetTest(unittest.TestCase):
    def write_fixture(self, root: Path, mutate=None, recompute_sha: bool = True) -> None:
        target = root / 'packages/github-devloop/departments/loop'
        target.mkdir(parents=True, exist_ok=True)
        (target / 'main.lua').write_text('local function pipeline() return "pipeline" end\nreturn { pipeline = pipeline }\n', encoding='utf-8')
        inventory = {
            'schema': ratchet.SCHEMA,
            'version': 1,
            'source_tree': ['packages/github-devloop/departments/loop/main.lua'],
            'old_behavior_observations': [
                {
                    'schema': ratchet.OBS_SCHEMA,
                    'observation_id': 'obs-1',
                    'owner': 'github-devloop',
                    'site': {'path': 'packages/github-devloop/departments/loop/main.lua', 'symbol': 'pipeline', 'ordinal': 'obs'},
                    'boundary': 'writer',
                    'typed_intent': {
                        'kind': 'state-transition',
                        'source_state': 'thinking',
                        'source_boundary': 'writer',
                        'target': 'blocked',
                        'cause_schema_id': 'state-marker.v1',
                        'generation_epoch': {'generation': '1', 'epoch': '1'},
                        'lineage': {'proposal_id': 'proposal-1'},
                    },
                    'old_inputs': {
                        'current_fact': {'state': 'thinking'},
                        'caller_from_states': ['thinking'],
                        'incoming_version': 'v1',
                        'target_version': 'v2',
                        'handoff_reference': None,
                    },
                    'old_outcome': {
                        'status': 'ok',
                        'reason_code': 'ok',
                        'cas_outcome': 'applied',
                        'emitted_effects': [{
                            'effect_id': 'eff-1',
                            'sink_kind': 'comment',
                            'authority_class': 'lifecycle-authoritative',
                            'ordinal': 1,
                        }],
                        'observable_writes': [{'kind': 'comment'}],
                        'handoff_direct_lookup_count': 0,
                        'timeout_evidence_source': None,
                    },
                    'evidence_refs': [{'kind': 'fixture', 'ref': 'old-execution:obs-1'}],
                }
            ],
            'old_pending_projection': [],
            'production_writer_sites': [
                {'site_id': 'writer:1', 'path': 'packages/github-devloop/departments/loop/main.lua', 'symbol': 'pipeline', 'ordinal': 'writer'}
            ],
            'effect_sink_sites': [],
            'row_replay_sites': [],
            'published_intent_sites': [],
            'receiver_activation_acceptors': [],
            'consumer_entry_acceptors': [],
            'direct_constructor_sites': [],
            'shared_issue_row_exports': [],
            'ops_issue_row_reader_sites': [],
            'owner_observation_fact_sites': [],
            'grantless_sink_sites': [],
            'unobserved_sites': [
                {'site_id': 'writer:1', 'category': 'production_writer_sites', 'path': 'packages/github-devloop/departments/loop/main.lua', 'symbol': 'pipeline', 'ordinal': 'writer', 'why': 'base'}
            ],
            'watched_files': ['packages/github-devloop/departments/loop/main.lua'],
        }
        if mutate:
            mutate(inventory)
        if recompute_sha:
            inventory['artifact_sha256'] = ratchet.artifact_sha256_for_document(inventory)
        else:
            inventory.setdefault('artifact_sha256', ratchet.artifact_sha256_for_document(inventory))
        (root / 'migration').mkdir(parents=True, exist_ok=True)
        (root / ratchet.INVENTORY).write_text(json.dumps(inventory, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
        (root / ratchet.ALLOWLIST).write_text('writer:1\n', encoding='utf-8')

    def messages_for(
        self,
        mutate=None,
        allowlist_lines=None,
        recompute_sha: bool = True,
        base_allowlist_lines=None,
    ):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_fixture(root, mutate=mutate, recompute_sha=recompute_sha)
            if allowlist_lines is not None:
                (root / ratchet.ALLOWLIST).write_text(''.join(f'{line}\n' for line in allowlist_lines), encoding='utf-8')
            if base_allowlist_lines is None:
                return ratchet.repository_messages(root, enforce_base=False)
            base_text = ''.join(f'{line}\n' for line in base_allowlist_lines)
            with mock.patch.object(ratchet.ratchet_base, 'file_at_base', return_value=('present', base_text)):
                return ratchet.repository_messages(root, enforce_base=True)

    def test_valid_fixture_passes(self):
        self.assertEqual(self.messages_for(), [])

    def test_null_target_version_passes(self):
        def mutate(inventory):
            inventory['old_behavior_observations'][0]['old_inputs']['target_version'] = None
        self.assertEqual(self.messages_for(mutate=mutate), [])

    def test_missing_target_version_key_fails(self):
        def mutate(inventory):
            del inventory['old_behavior_observations'][0]['old_inputs']['target_version']
        messages = self.messages_for(mutate=mutate)
        self.assertTrue(any('old_inputs: missing key target_version' in message for message in messages), messages)

    def test_empty_target_version_fails(self):
        def mutate(inventory):
            inventory['old_behavior_observations'][0]['old_inputs']['target_version'] = ''
        messages = self.messages_for(mutate=mutate)
        self.assertTrue(any(
            '.old_inputs.target_version: must be a non-empty string or null' in message
            for message in messages
        ), messages)

    def test_wrong_type_target_version_fails(self):
        def mutate(inventory):
            inventory['old_behavior_observations'][0]['old_inputs']['target_version'] = 123
        messages = self.messages_for(mutate=mutate)
        self.assertTrue(any(
            '.old_inputs.target_version: must be a non-empty string or null' in message
            for message in messages
        ), messages)

    def test_missing_spec_required_fields_fail(self):
        cases = (
            (('evidence_refs',), 'missing key evidence_refs'),
            (('typed_intent', 'source_state'), 'typed_intent: missing key source_state'),
            (('typed_intent', 'source_boundary'), 'typed_intent: missing key source_boundary'),
            (('typed_intent', 'target'), 'typed_intent: missing key target'),
            (('typed_intent', 'cause_schema_id'), 'typed_intent: missing key cause_schema_id'),
            (('typed_intent', 'generation_epoch'), 'typed_intent: missing key generation_epoch'),
            (('typed_intent', 'lineage'), 'typed_intent: missing key lineage'),
            (('old_inputs', 'current_fact'), 'old_inputs: missing key current_fact'),
            (('old_inputs', 'caller_from_states'), 'old_inputs: missing key caller_from_states'),
            (('old_inputs', 'incoming_version'), 'old_inputs: missing key incoming_version'),
            (('old_inputs', 'target_version'), 'old_inputs: missing key target_version'),
            (('old_inputs', 'handoff_reference'), 'old_inputs: missing key handoff_reference'),
            (('old_outcome', 'observable_writes'), 'old_outcome: missing key observable_writes'),
            (('old_outcome', 'handoff_direct_lookup_count'), 'old_outcome: missing key handoff_direct_lookup_count'),
            (('old_outcome', 'timeout_evidence_source'), 'old_outcome: missing key timeout_evidence_source'),
        )
        for field_path, expected in cases:
            with self.subTest(field_path=field_path):
                def mutate(inventory, field_path=field_path):
                    value = inventory['old_behavior_observations'][0]
                    for key in field_path[:-1]:
                        value = value[key]
                    del value[field_path[-1]]
                messages = self.messages_for(mutate=mutate)
                self.assertTrue(any(expected in m for m in messages), messages)

    def test_unknown_nested_observation_key_fails(self):
        def mutate(inventory):
            inventory['old_behavior_observations'][0]['typed_intent']['surprise'] = True
        messages = self.messages_for(mutate=mutate)
        self.assertTrue(any('typed_intent: unknown key surprise' in m for m in messages), messages)

    def test_observation_identifier_fields_must_be_non_empty_strings(self):
        for key, value in (('observation_id', ''), ('owner', 7)):
            with self.subTest(key=key):
                def mutate(inventory, key=key, value=value):
                    inventory['old_behavior_observations'][0][key] = value
                messages = self.messages_for(mutate=mutate)
                self.assertTrue(any(f'.{key}: must be a non-empty string' in m for m in messages), messages)

    def test_evidence_refs_must_be_a_non_empty_collection(self):
        expected = '.evidence_refs: must be a non-empty array or object'
        for value in ([], {}, 'x'):
            with self.subTest(value=value):
                def mutate(inventory, value=value):
                    inventory['old_behavior_observations'][0]['evidence_refs'] = value
                messages = self.messages_for(mutate=mutate)
                self.assertTrue(any(expected in m for m in messages), messages)

    def test_emitted_effect_value_constraints_fail(self):
        cases = (
            ('sink_kind', 'invalid', '.sink_kind: must be one of '),
            ('authority_class', 'invalid', '.authority_class: must be one of '),
            ('ordinal', 0, '.ordinal: must be a positive integer'),
        )
        for key, value, expected in cases:
            with self.subTest(key=key):
                def mutate(inventory, key=key, value=value):
                    effect = inventory['old_behavior_observations'][0]['old_outcome']['emitted_effects'][0]
                    effect[key] = value
                messages = self.messages_for(mutate=mutate)
                self.assertTrue(any(expected in m for m in messages), messages)

    def test_old_outcome_value_constraints_fail(self):
        cases = (
            ('observable_writes', 'invalid', '.observable_writes: must be an array or object'),
            ('handoff_direct_lookup_count', -1, '.handoff_direct_lookup_count: must be a non-negative integer'),
            ('timeout_evidence_source', [], '.timeout_evidence_source: must be a string or null'),
        )
        for key, value, expected in cases:
            with self.subTest(key=key):
                def mutate(inventory, key=key, value=value):
                    inventory['old_behavior_observations'][0]['old_outcome'][key] = value
                messages = self.messages_for(mutate=mutate)
                self.assertTrue(any(expected in m for m in messages), messages)

    def test_nested_string_and_nullable_value_constraints_fail(self):
        cases = (
            (('typed_intent', 'kind'), '', '.typed_intent.kind: must be a non-empty string'),
            (('typed_intent', 'source_state'), [], '.typed_intent.source_state: must be a string or null'),
            (('old_inputs', 'incoming_version'), '', '.old_inputs.incoming_version: must be a non-empty string'),
            (('old_inputs', 'handoff_reference'), 'invalid', '.old_inputs.handoff_reference: must be an object or null'),
            (('old_outcome', 'status'), '', '.old_outcome.status: must be a non-empty string'),
            (
                ('old_outcome', 'emitted_effects', 0, 'effect_id'),
                '',
                '.old_outcome.emitted_effects[0].effect_id: must be a non-empty string',
            ),
        )
        for field_path, value, expected in cases:
            with self.subTest(field_path=field_path):
                def mutate(inventory, field_path=field_path, value=value):
                    target = inventory['old_behavior_observations'][0]
                    for key in field_path[:-1]:
                        target = target[key]
                    target[field_path[-1]] = value
                messages = self.messages_for(mutate=mutate)
                self.assertTrue(any(expected in m for m in messages), messages)

    def test_missing_inventory_skips(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.assertEqual(ratchet.repository_messages(root, enforce_base=False), [])

    def test_tampered_artifact_sha256_fails(self):
        def mutate(inventory):
            inventory['artifact_sha256'] = 'tampered'
        messages = self.messages_for(mutate=mutate, recompute_sha=False)
        self.assertTrue(any('artifact_sha256 mismatch' in m for m in messages), messages)

    def test_normalize_artifact_sha256_rewrites_tampered_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)

            def mutate(inventory):
                inventory['artifact_sha256'] = 'tampered'

            self.write_fixture(root, mutate=mutate, recompute_sha=False)
            expected = ratchet.normalize_artifact_sha256(root)
            inventory = json.loads((root / ratchet.INVENTORY).read_text(encoding='utf-8'))

            self.assertEqual(inventory['artifact_sha256'], expected)
            self.assertEqual(ratchet.repository_messages(root, enforce_base=False), [])

    def test_allowlist_growth_fails(self):
        messages = self.messages_for(allowlist_lines=['writer:1', 'writer:2'])
        self.assertTrue(any('unknown site id writer:2' in m for m in messages), messages)

    def test_allowlist_missing_id_fails(self):
        messages = self.messages_for(allowlist_lines=[])
        self.assertTrue(any('missing unobserved site id writer:1' in m for m in messages), messages)

    def test_allowlist_shrink_without_matching_observation_fails(self):
        def mutate(inventory):
            inventory['unobserved_sites'] = []
        messages = self.messages_for(
            mutate=mutate,
            allowlist_lines=[],
            base_allowlist_lines=['writer:1'],
        )
        self.assertTrue(any(
            'writer:1' in m
            and 'packages/github-devloop/departments/loop/main.lua' in m
            and 'pipeline' in m
            and 'writer' in m
            and 'missing matching old behavior observation evidence' in m
            for m in messages
        ), messages)

    def test_allowlist_shrink_with_matching_observation_passes(self):
        def mutate(inventory):
            inventory['unobserved_sites'] = []
            inventory['old_behavior_observations'][0]['site'] = {
                'path': 'packages/github-devloop/departments/loop/main.lua',
                'symbol': 'pipeline',
                'ordinal': 'writer',
            }
        self.assertEqual(self.messages_for(
            mutate=mutate,
            allowlist_lines=[],
            base_allowlist_lines=['writer:1'],
        ), [])

    def test_allowlist_shrink_with_malformed_site_identity_fails_closed(self):
        def mutate(inventory):
            inventory['unobserved_sites'] = []
            inventory['production_writer_sites'][0]['ordinal'] = []
        messages = self.messages_for(
            mutate=mutate,
            allowlist_lines=[],
            base_allowlist_lines=['writer:1'],
        )
        self.assertTrue(any(
            'writer:1' in m and 'no unique enumerated (path, symbol, ordinal) identity' in m
            for m in messages
        ), messages)

    def test_allowlist_shrink_with_ambiguous_site_identity_fails_closed(self):
        def mutate(inventory):
            inventory['unobserved_sites'] = []
            inventory['effect_sink_sites'].append({
                'site_id': 'writer:1',
                'path': 'packages/github-devloop/departments/loop/main.lua',
                'symbol': 'pipeline',
                'ordinal': 'second-writer',
            })
        messages = self.messages_for(
            mutate=mutate,
            allowlist_lines=[],
            base_allowlist_lines=['writer:1'],
        )
        self.assertTrue(any(
            'writer:1' in m and 'no unique enumerated (path, symbol, ordinal) identity' in m
            for m in messages
        ), messages)

    def test_edge_id_key_fails(self):
        def mutate(inventory):
            inventory['old_behavior_observations'][0]['edge_id'] = 'edge-1'
        messages = self.messages_for(mutate=mutate)
        self.assertTrue(any('edge_id is forbidden' in m for m in messages), messages)

    def test_new_module_token_fails(self):
        def mutate(inventory):
            inventory['old_behavior_observations'][0]['typed_intent'] = {'kind': 'restart_edges.bad'}
        messages = self.messages_for(mutate=mutate)
        self.assertTrue(any("NEW module token 'restart_edges'" in m for m in messages), messages)

    def test_unknown_top_level_key_fails(self):
        def mutate(inventory):
            inventory['surprise'] = True
        messages = self.messages_for(mutate=mutate)
        self.assertTrue(any('unknown top-level key surprise' in m for m in messages), messages)

    def test_fabricated_site_path_fails(self):
        def mutate(inventory):
            inventory['production_writer_sites'][0]['path'] = 'packages/missing.lua'
            inventory['unobserved_sites'][0]['path'] = 'packages/missing.lua'
        messages = self.messages_for(mutate=mutate)
        self.assertTrue(any('site_id writer:1: path does not exist' in m for m in messages), messages)


if __name__ == '__main__':
    unittest.main()
