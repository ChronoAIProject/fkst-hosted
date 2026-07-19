// Realistic API fixtures for the dashboard E2E — every object mirrors the
// snake_case wire shapes in src/lib/api/types.ts field for field. A single
// route handler (installApiRoutes) dispatches all /api/v1/* calls to these.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import type { Page, Route } from '@playwright/test';
import type { EnvStore } from './env-fixtures';
import { LOG_CONTENT, observeSnapshot, TEXT_BLOBS } from './payloads';

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
  // Frozen config the ConfigPanel (Packages tab) surfaces: the log-access
  // allowlist (extra GitHub logins) and the output locale.
  log_access: [PERSONAL, 'collab-bob'],
  output_lang: 'zh',
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

/** A repo with many sessions so the level-2 sidebar panel overflows its fixed
 *  height and its INTERNAL ScrollArea (not the page) scrolls. The first two are
 *  the real live/degraded sessions so drawer flows still work off this payload. */
export function manyRepoSessions(count = 24) {
  const extra = Array.from({ length: count }, (_, i) => ({
    ...degradedSession,
    session_id: `bulk${String(i).padStart(12, '0')}`,
    name: `bulk-session-${i}`,
    status_labels: ['fkst-substrate-active'],
    trigger: issue(500 + i, `bulk session ${i}`, 'open', ['fkst-substrate-trigger']),
    work_issues: [],
  }));
  return { owner: PERSONAL, name: REPO, installed: true, sessions: [liveSession, ...extra] };
}

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

// ---- Router -----------------------------------------------------------------

const json = (route: Route, body: unknown, status = 200) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

export interface RouteOptions {
  /** Force GET /overview to 500 (drives the load-failed error screen). */
  failOverview?: boolean;
  /** Force GET /overview to a given status (401 → reactive refresh → with no
   *  refresh token, an involuntary session expiry / re-auth prompt). */
  overviewStatus?: number;
  /** Return an overview whose account carries a malformed (null) `repos`, so a
   *  render-time throw escapes into the ErrorBoundary/route error fallback. */
  malformedOverview?: boolean;
  /** Serve a large session list at level 2 so the sidebar panel overflows. */
  manySessions?: boolean;
  /** A stateful environment-profile store; when present its handler serves every
   *  `/environment-profiles` path (list/get/put/delete) with real mutations. */
  envStore?: EnvStore;
}

/** An overview whose first account has `repos: null` — passes the client's
 *  array/login shape checks but throws when a level renders its repos. */
const malformedOverviewBody = {
  ...overview,
  accounts: [{ ...overview.accounts[0], repos: null }],
};

/** Register one handler for every /api/v1/* call the SPA makes. */
export async function installApiRoutes(page: Page, opts: RouteOptions = {}) {
  await page.route('**/api/v1/**', async (route) => {
    const url = new URL(route.request().url());
    const p = url.pathname;

    // Stateful env-profile store owns its own paths when supplied.
    if (opts.envStore && (await opts.envStore.handle(route, url))) return;

    if (p.endsWith('/api/v1/overview')) {
      if (opts.failOverview) return json(route, { error: 'internal', message: 'boom' }, 500);
      if (opts.overviewStatus) {
        return json(route, { error: 'unauthorized', message: 'token expired' }, opts.overviewStatus);
      }
      if (opts.malformedOverview) return json(route, malformedOverviewBody);
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

    // stop a session: DELETE /repos/{o}/{n}/sessions/{issueNumber}
    if (/\/repos\/[^/]+\/[^/]+\/sessions\/\d+$/.test(p)) {
      return json(route, { ok: true });
    }

    // create/list sessions: POST/GET /repos/{o}/{n}/sessions
    if (/\/repos\/[^/]+\/[^/]+\/sessions$/.test(p)) {
      if (route.request().method() === 'POST') {
        // CreateSessionResponse: the created trigger issue.
        return json(route, { issue_number: 999, html_url: 'https://github.com/x/y/issues/999' });
      }
      return json(route, opts.manySessions ? manyRepoSessions() : repoSessions);
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

/** The guided-tour per-login "seen" key for the fixture viewer (PERSONAL is
 *  overview.viewer.login). seedAuth marks it seen so the first-run auto-prompt
 *  doesn't overlay unrelated dashboard tests; the onboarding spec clears it. */
export const TOUR_SEEN_KEY = `fkst-tour-seen-v1:${PERSONAL}`;

/** Seed a fake access token so useAuth() renders as an authenticated user
 *  BEFORE any page script runs (isAuthenticated reads localStorage on init).
 *  Also marks the guided tour as already seen (the default is a returning,
 *  onboarded user); pass {firstRun: true} to leave it unseen so the tour
 *  auto-prompts. */
export async function seedAuth(page: Page, opts: { firstRun?: boolean } = {}) {
  const seenKey = opts.firstRun ? null : TOUR_SEEN_KEY;
  await page.addInitScript((k) => {
    window.localStorage.setItem('fkst-gh-access', 'e2e-fake-access-token');
    // No expiry key → treated as non-expiring, so getToken() never refreshes.
    if (k) window.localStorage.setItem(k, String(1));
  }, seenKey);
}
