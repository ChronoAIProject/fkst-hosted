// Realistic API fixtures for the dashboard E2E — every object mirrors the
// snake_case wire shapes in src/lib/api/types.ts field for field. A single
// route handler (installApiRoutes) dispatches all /api/v1/* calls to these.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import type { Page, Route } from '@playwright/test';

const asset = (name: string): Buffer =>
  readFileSync(fileURLToPath(new URL(`./assets/${name}`, import.meta.url)));

// Real bytes so the <img>/<video> tags actually render (ffmpeg-generated).
const PNG_BYTES = asset('sample.png');
const MP4_BYTES = asset('sample.mp4');

export const PERSONAL = 'octo-dev';
export const ORG = 'acme-corp';
export const REPO = 'web-app';

// Session A: healthy + live. Its session_id drives the logs/observe routes.
export const LIVE_SESSION_ID = 'a1b2c3d4e5f6a7b8';
// Session B: degraded — its observe route returns an error on purpose.
export const DEGRADED_SESSION_ID = 'degraded99887766';

const iso = (d: string) => new Date(d).toISOString();

function issue(
  number: number,
  title: string,
  state: 'open' | 'closed',
  labels: string[],
  closedAt: string | null = null
) {
  return {
    number,
    title,
    state,
    author: PERSONAL,
    labels,
    html_url: `https://github.com/${PERSONAL}/${REPO}/issues/${number}`,
    created_at: iso('2026-07-15T02:00:00Z'),
    updated_at: iso('2026-07-18T09:30:00Z'),
    closed_at: closedAt,
  };
}

// ---- GET /api/v1/overview ---------------------------------------------------

export const overview = {
  app_slug: 'fkst-hosted-app',
  viewer: { login: PERSONAL },
  accounts: [
    {
      login: PERSONAL,
      kind: 'personal',
      owner: true,
      installed: true,
      installation_id: 1001,
      repository_selection: 'all',
      counts_complete: true,
      repos: [
        {
          id: 1,
          owner: PERSONAL,
          name: REPO,
          private: false,
          admin: true,
          installed: true,
          active_sessions: 2,
          packages: [
            'ChronoAIProject/fkst-packages@fkst-hosted:workflow-dev',
            'ChronoAIProject/fkst-packages@fkst-hosted:github-devloop',
          ],
        },
        {
          id: 2,
          owner: PERSONAL,
          name: 'api-service',
          private: true,
          admin: true,
          installed: true,
          active_sessions: 1,
          packages: ['ChronoAIProject/fkst-packages@fkst-hosted:github-devloop'],
        },
        {
          id: 3,
          owner: PERSONAL,
          name: 'docs-site',
          private: false,
          admin: true,
          installed: false,
          active_sessions: 0,
          packages: [],
        },
      ],
    },
    {
      login: ORG,
      kind: 'org',
      owner: true,
      installed: true,
      installation_id: 2002,
      repository_selection: 'selected',
      // deliberately incomplete → renders the "±" counts marker
      counts_complete: false,
      repos: [
        {
          id: 10,
          owner: ORG,
          name: 'platform',
          private: true,
          admin: true,
          installed: true,
          active_sessions: 3,
          packages: [
            'ChronoAIProject/fkst-packages@fkst-hosted:consensus-triage',
            'ChronoAIProject/fkst-packages@fkst-hosted:code-review',
          ],
        },
        {
          id: 11,
          owner: ORG,
          name: 'infra',
          private: true,
          admin: false,
          installed: true,
          active_sessions: 0,
          packages: [],
        },
      ],
    },
  ],
  totals: {
    sessions: 6,
    packages: [
      { package: 'ChronoAIProject/fkst-packages@fkst-hosted:github-devloop', count: 2 },
      { package: 'ChronoAIProject/fkst-packages@fkst-hosted:workflow-dev', count: 1 },
      { package: 'ChronoAIProject/fkst-packages@fkst-hosted:consensus-triage', count: 1 },
      { package: 'ChronoAIProject/fkst-packages@fkst-hosted:code-review', count: 1 },
    ],
  },
};

// ---- GET /api/v1/repos/{owner}/{name}/sessions ------------------------------

const liveSession = {
  session_id: LIVE_SESSION_ID,
  name: 'feature-auth',
  work_label: 'fkst-work',
  auto_merge: true,
  environment: 'video-studio',
  packages: [
    'ChronoAIProject/fkst-packages@fkst-hosted:workflow-dev',
    'ChronoAIProject/fkst-packages@fkst-hosted:github-devloop',
  ],
  invalid_reason: null,
  status_labels: ['fkst-substrate-active'],
  trigger: issue(101, 'feature-auth session', 'open', ['fkst-substrate-trigger']),
  work_issues: [
    issue(110, 'Scaffold auth module', 'closed', ['fkst-work'], iso('2026-07-17T10:00:00Z')),
    issue(111, 'Add login form', 'open', ['fkst-work', 'fkst-dev:thinking']),
    issue(112, 'Wire OAuth callback', 'open', ['fkst-work', 'fkst-dev:implementing']),
    issue(113, 'Persist session cookie', 'open', ['fkst-work', 'fkst-dev:ready']),
  ],
  log_url: 'https://api.chronoai-fkst.local/api/v1/logs/' + LIVE_SESSION_ID,
  liveness: 'live',
  prs: [
    {
      number: 300,
      title: 'feat: auth scaffold',
      html_url: `https://github.com/${PERSONAL}/${REPO}/pull/300`,
      state: 'closed',
      merged: true,
      work_issue: 110,
    },
    {
      number: 301,
      title: 'feat: login form',
      html_url: `https://github.com/${PERSONAL}/${REPO}/pull/301`,
      state: 'open',
      merged: false,
      work_issue: 111,
    },
  ],
};

const degradedSession = {
  session_id: DEGRADED_SESSION_ID,
  name: 'refactor-core',
  work_label: 'fkst-refactor',
  auto_merge: false,
  environment: null,
  packages: ['ChronoAIProject/fkst-packages@fkst-hosted:code-review'],
  invalid_reason: null,
  status_labels: ['fkst-degraded'],
  trigger: issue(202, 'refactor-core session', 'open', ['fkst-substrate-trigger']),
  work_issues: [issue(211, 'Split the monolith module', 'open', ['fkst-refactor', 'fkst-dev:impl-failed'])],
  log_url: null,
  liveness: null,
  prs: [],
};

export const TRIGGER_LIVE = liveSession.trigger.number; // 101
export const TRIGGER_DEGRADED = degradedSession.trigger.number; // 202

export const repoSessions = {
  owner: PERSONAL,
  name: REPO,
  installed: true,
  sessions: [liveSession, degradedSession],
};

// ---- GET /api/v1/repos/{o}/{n}/sessions/{issue}/outcomes --------------------

export const outcomes = {
  owner: PERSONAL,
  name: REPO,
  trigger_issue: TRIGGER_LIVE,
  prs: [
    {
      number: 301,
      title: 'feat: login form',
      html_url: `https://github.com/${PERSONAL}/${REPO}/pull/301`,
      state: 'open',
      merged: false,
      work_issue: 111,
      files_error: false,
      files: [
        {
          filename: 'src/auth/login.tsx',
          status: 'added',
          additions: 128,
          deletions: 0,
          sha: 'sha-text-login',
          previous_filename: null,
          kind: 'text',
          size_hint: 128,
        },
        {
          filename: 'docs/login-screenshot.png',
          status: 'added',
          additions: 0,
          deletions: 0,
          sha: 'sha-image-shot',
          previous_filename: null,
          kind: 'image',
          size_hint: null,
        },
        {
          filename: 'media/login-demo.mp4',
          status: 'added',
          additions: 0,
          deletions: 0,
          sha: 'sha-video-demo',
          previous_filename: null,
          kind: 'video',
          size_hint: null,
        },
        {
          filename: 'src/auth/session.ts',
          status: 'renamed',
          additions: 4,
          deletions: 2,
          sha: 'sha-text-session',
          previous_filename: 'src/auth/old-session.ts',
          kind: 'text',
          size_hint: 6,
        },
      ],
    },
    {
      number: 300,
      title: 'feat: auth scaffold',
      html_url: `https://github.com/${PERSONAL}/${REPO}/pull/300`,
      state: 'closed',
      merged: true,
      work_issue: 110,
      files_error: false,
      files: [
        {
          filename: 'src/auth/index.ts',
          status: 'modified',
          additions: 8,
          deletions: 3,
          sha: 'sha-text-index',
          previous_filename: null,
          kind: 'text',
          size_hint: 11,
        },
      ],
    },
  ],
};

const TEXT_BLOBS: Record<string, string> = {
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

// ---- GET /api/v1/logs/{session_id}/manifest & /file -------------------------

export const logManifest = {
  session_id: LIVE_SESSION_ID,
  generated_at: iso('2026-07-18T09:31:00Z'),
  files: [
    { path: 'fkst-substrate/driver/driver.log', size: 20480, label: 'Driver' },
    { path: 'fkst-substrate/supervise/supervise.log', size: 8192, label: 'Supervise' },
    { path: 'fkst-substrate/codex/codex.log', size: 51200, label: 'Codex' },
    { path: 'fkst-substrate/misc/notes.log', size: 1024, label: 'Misc' },
    { path: 'README.md', size: 512, label: 'README' },
    { path: 'meta.json', size: 256, label: 'Meta' },
  ],
};

// Each log file's text. The driver log carries the searchable token
// "reconcile" several times so the in-file search highlights matches.
const LOG_CONTENT: Record<string, string> = {
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

function logFileBody(path: string) {
  const content = LOG_CONTENT[path] ?? '';
  const total = 20480; // pretend the on-disk file is larger than the tail
  const truncated = path.endsWith('driver.log');
  return {
    session_id: LIVE_SESSION_ID,
    path,
    content,
    total_bytes: truncated ? total : content.length,
    returned_bytes: content.length,
    truncated,
  };
}

// ---- GET /api/v1/sessions/{session_id}/observe (raw engine JSON) ------------

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

// ---- Router -----------------------------------------------------------------

const json = (route: Route, body: unknown, status = 200) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

export interface RouteOptions {
  /** Force GET /overview to 500 (drives the load-failed error screen). */
  failOverview?: boolean;
}

/** Register one handler for every /api/v1/* call the SPA makes. */
export async function installApiRoutes(page: Page, opts: RouteOptions = {}) {
  await page.route('**/api/v1/**', async (route) => {
    const url = new URL(route.request().url());
    const p = url.pathname;

    if (p.endsWith('/api/v1/overview')) {
      if (opts.failOverview) return json(route, { error: 'internal', message: 'boom' }, 500);
      return json(route, overview);
    }

    // auth refresh (never hit in the happy path, but answer defensively)
    if (p.endsWith('/auth/github/refresh')) {
      return json(route, { access_token: 'e2e-refreshed-token' });
    }

    // outcomes: /repos/{o}/{n}/sessions/{issue}/outcomes
    if (/\/sessions\/\d+\/outcomes$/.test(p)) {
      const m = p.match(/\/sessions\/(\d+)\/outcomes$/)!;
      const trigger = Number(m[1]);
      if (trigger === TRIGGER_LIVE) return json(route, outcomes);
      // degraded session has no PRs → drives the "no PRs yet" empty state
      return json(route, { owner: PERSONAL, name: REPO, trigger_issue: trigger, prs: [] });
    }

    // repo sessions: /repos/{o}/{n}/sessions
    if (/\/repos\/[^/]+\/[^/]+\/sessions$/.test(p)) {
      return json(route, repoSessions);
    }

    // blob: /repos/{o}/{n}/blob/{sha}
    if (/\/blob\//.test(p)) {
      const sha = decodeURIComponent(p.split('/blob/')[1] ?? '');
      if (sha.startsWith('sha-image')) {
        return route.fulfill({ status: 200, contentType: 'image/png', body: PNG_BYTES });
      }
      if (sha.startsWith('sha-video')) {
        return route.fulfill({ status: 200, contentType: 'video/mp4', body: MP4_BYTES });
      }
      return route.fulfill({
        status: 200,
        contentType: 'text/plain; charset=utf-8',
        body: TEXT_BLOBS[sha] ?? '(empty)',
      });
    }

    // logs manifest: /logs/{id}/manifest
    if (/\/logs\/[^/]+\/manifest$/.test(p)) {
      return json(route, logManifest);
    }

    // logs file: /logs/{id}/file?path=...
    if (/\/logs\/[^/]+\/file$/.test(p)) {
      const path = url.searchParams.get('path') ?? '';
      return json(route, logFileBody(path));
    }

    // observe: /sessions/{id}/observe
    if (/\/sessions\/[^/]+\/observe$/.test(p)) {
      const id = decodeURIComponent(p.split('/sessions/')[1]?.split('/observe')[0] ?? '');
      if (id === DEGRADED_SESSION_ID) {
        // no durable store to observe / pod exec failed
        return json(route, { error: 'observe_failed', message: 'pod exec failed' }, 500);
      }
      return json(route, observeSnapshot);
    }

    // Anything unmatched: a JSON 404 so a stray call is visible, not silent.
    return json(route, { error: 'not_found', message: `no fixture for ${p}` }, 404);
  });
}

/** Seed a fake access token so useAuth() renders as an authenticated user
 *  BEFORE any page script runs (isAuthenticated reads localStorage on init). */
export async function seedAuth(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem('fkst-gh-access', 'e2e-fake-access-token');
    // No expiry key → treated as non-expiring, so getToken() never refreshes.
  });
}
