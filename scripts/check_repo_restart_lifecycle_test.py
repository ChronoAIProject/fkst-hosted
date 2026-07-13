#!/usr/bin/env python3
from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

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
                    'typed_intent': {'kind': 'comment'},
                    'old_inputs': {'state': 'thinking'},
                    'old_outcome': {
                        'status': 'ok',
                        'reason_code': 'ok',
                        'cas_outcome': 'applied',
                        'emitted_effects': [{'effect_id': 'eff-1', 'sink_kind': 'comment', 'authority_class': 'bot', 'ordinal': '1'}],
                    },
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

    def messages_for(self, mutate=None, allowlist_lines=None, recompute_sha: bool = True):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_fixture(root, mutate=mutate, recompute_sha=recompute_sha)
            if allowlist_lines is not None:
                (root / ratchet.ALLOWLIST).write_text(''.join(f'{line}\n' for line in allowlist_lines), encoding='utf-8')
            return ratchet.repository_messages(root, enforce_base=False)

    def test_valid_fixture_passes(self):
        self.assertEqual(self.messages_for(), [])

    def test_missing_inventory_skips(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.assertEqual(ratchet.repository_messages(root, enforce_base=False), [])

    def test_tampered_artifact_sha256_fails(self):
        def mutate(inventory):
            inventory['artifact_sha256'] = 'tampered'
        messages = self.messages_for(mutate=mutate, recompute_sha=False)
        self.assertTrue(any('artifact_sha256 mismatch' in m for m in messages), messages)

    def test_allowlist_growth_fails(self):
        messages = self.messages_for(allowlist_lines=['writer:1', 'writer:2'])
        self.assertTrue(any('unknown site id writer:2' in m for m in messages), messages)

    def test_allowlist_missing_id_fails(self):
        messages = self.messages_for(allowlist_lines=[])
        self.assertTrue(any('missing unobserved site id writer:1' in m for m in messages), messages)

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
