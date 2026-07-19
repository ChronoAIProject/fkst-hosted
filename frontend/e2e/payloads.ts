// Bulky, self-contained response payloads split out of fixtures.ts so both
// files stay under the 500-line ceiling. These are pure data (no dependency on
// fixtures' own constants), imported back by the router in fixtures.ts.

/** Blob bodies keyed by SHA — the text previews the Outcomes tab expands. */
export const TEXT_BLOBS: Record<string, string> = {
  'sha-text-login': [
    "import { useState } from 'react';",
    '',
    'export function LoginForm() {',
    '  const [email, setEmail] = useState("");',
    '  // renders the OAuth entry point',
    '  return <form aria-label="Sign in">…</form>;',
    '}',
  ].join('\n'),
  'sha-text-session':
    'export const SESSION_COOKIE = "fkst.sid";\n// 8h sliding TTL, refreshed on activity\n',
  'sha-text-index': 'export * from "./login";\nexport * from "./session";\n',
};

// Each log file's text. The driver log carries the searchable token "reconcile"
// several times so the in-file search highlights matches.
export const LOG_CONTENT: Record<string, string> = {
  'fkst-substrate/driver/driver.log': [
    '2026-07-18T09:20:01Z INFO  driver: startup, watching installation 1001',
    '2026-07-18T09:20:03Z DEBUG driver: reconcile tick begin (repo octo-dev/web-app)',
    '2026-07-18T09:20:03Z DEBUG driver: reconcile discovered 3 open work issues',
    '2026-07-18T09:20:04Z INFO  driver: claimed work issue #111 (thinking)',
    '2026-07-18T09:20:31Z DEBUG driver: reconcile tick begin',
    '2026-07-18T09:20:31Z WARN  driver: pod liveness probe slow (1.8s)',
    '2026-07-18T09:20:59Z DEBUG driver: reconcile tick begin',
    '2026-07-18T09:21:00Z ERROR driver: transient GitHub 502 on comment, retrying',
    '2026-07-18T09:21:02Z INFO  driver: reconcile settled, 2 in flight',
  ].join('\n'),
  'fkst-substrate/supervise/supervise.log':
    '2026-07-18T09:20:02Z INFO supervise: launching codex\n2026-07-18T09:20:05Z INFO supervise: codex healthy\n',
  'fkst-substrate/codex/codex.log':
    '2026-07-18T09:20:06Z codex: session started\n2026-07-18T09:20:40Z codex: proposed patch for #112\n',
  'fkst-substrate/misc/notes.log': 'misc: nothing notable\n',
  'README.md': '# Session log bundle\n\nRedacted logs for session feature-auth.\n',
  'meta.json': '{\n  "session": "feature-auth",\n  "schema": 1\n}\n',
};

/** GET /api/v1/sessions/{id}/observe — the engine read-model, returned verbatim. */
export const observeSnapshot = {
  schema_version: 1,
  generated_at_ms: 1_752_830_000_000,
  source: 'engine',
  limits: { max_queues: 64 },
  truncated: false,
  queues: [
    {
      queue: 'workflow-writer.workflow_writer_materialization_tick',
      depth: 3,
      pending: 2,
      in_flight: 1,
      retrying: 0,
      oldest_pending_age_ms: 12_000,
    },
    {
      queue: 'github-devloop.reconcile_tick',
      depth: 1,
      pending: 1,
      in_flight: 0,
      retrying: 0,
      oldest_pending_age_ms: null,
    },
    {
      queue: 'codex.candidate_poll',
      depth: 0,
      pending: 0,
      in_flight: 0,
      retrying: 1,
      oldest_pending_age_ms: 400,
    },
  ],
  deliveries: [{ id: 'd1' }, { id: 'd2' }],
  dead_letters: [],
};
