// Stateful fixtures for the named-environment REST API
// (`/api/v1/users/me/environment-profiles`, backend `routes/environments.rs`).
// Kept out of fixtures.ts so BOTH files stay under the 500-line ceiling, and so
// the CRUD-parity spec can drive a real create→list→delete round-trip against an
// in-memory store whose mutations (PUT/DELETE) are reflected in the next list.

import type { Route } from '@playwright/test';

const BASE = '/api/v1/users/me/environment-profiles';

const iso = (d: string) => new Date(d).toISOString();

/** The full view of one stored environment. Secret VALUES are never held here —
 *  only `secret_keys` — mirroring the backend contract (values are write-only). */
export interface StoredProfile {
  name: string;
  status: string;
  validated_at: string;
  install: string[];
  variables: Record<string, string>;
  secret_keys: string[];
}

/** The install-validation `422` body a failing PUT returns (nothing persisted).
 *  The parity spec asserts every field the inline ValidationReport renders. */
export const VALIDATION_ERROR = {
  error: 'install_validation_failed',
  message: 'Install command failed inside the validation pod.',
  failed_command_index: 1,
  failed_command: 'pip install nonexistent-pkg==9.9.9',
  exit_code: 2,
  timed_out: false,
  stderr_tail:
    'ERROR: Could not find a version that satisfies the requirement nonexistent-pkg==9.9.9\nERROR: No matching distribution found',
};

/** Any PUT whose env name is in this set fails validation (422) instead of
 *  persisting — drives the inline error path without a bespoke route. */
const FAIL_NAMES = new Set(['bad-env']);

function summaryOf(p: StoredProfile) {
  return {
    name: p.name,
    status: p.status,
    validated_at: p.validated_at,
    install_command_count: p.install.length,
    variable_count: Object.keys(p.variables).length,
    secret_count: p.secret_keys.length,
  };
}

const json = (route: Route, body: unknown, status = 200) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

/**
 * A per-test in-memory environment store + the route handler that serves it.
 * `handle` returns true when it consumed the request (an env-profiles path),
 * false otherwise so the caller's dispatcher can fall through to other routes.
 */
export interface EnvStore {
  profiles: Map<string, StoredProfile>;
  handle: (route: Route, url: URL) => Promise<boolean>;
}

/** Seed the store with a starter environment so the list is non-empty on open
 *  and the create-trigger picker has a profile to show even before a create. */
export function createEnvStore(): EnvStore {
  const profiles = new Map<string, StoredProfile>();
  profiles.set('video-studio', {
    name: 'video-studio',
    status: 'valid',
    validated_at: iso('2026-07-18T08:00:00Z'),
    install: ['apt-get install -y ffmpeg'],
    variables: { OUTPUT_DIR: '/out' },
    // Only KEY names — the value 'topsecret' below is never stored/echoed.
    secret_keys: ['RENDER_TOKEN'],
  });

  const handle = async (route: Route, url: URL): Promise<boolean> => {
    const p = url.pathname;
    if (!p.includes('/environment-profiles')) return false;

    // GET list
    if (p.endsWith(BASE) && route.request().method() === 'GET') {
      await json(route, {
        environment_profiles: [...profiles.values()].map(summaryOf),
      });
      return true;
    }

    const nameMatch = p.match(/\/environment-profiles\/([^/]+)$/);
    const name = nameMatch ? decodeURIComponent(nameMatch[1]!) : null;
    const method = route.request().method();

    if (name && method === 'GET') {
      const found = profiles.get(name);
      if (!found) return json(route, { error: 'not_found', message: name }, 404).then(() => true);
      await json(route, found);
      return true;
    }

    if (name && method === 'PUT') {
      // A designated name fails install validation (422) with the detailed
      // report; nothing is persisted, matching the backend.
      if (FAIL_NAMES.has(name)) {
        await json(route, VALIDATION_ERROR, 422);
        return true;
      }
      const spec = JSON.parse(route.request().postData() ?? '{}') as {
        install?: string[];
        variables?: Record<string, string>;
        secrets?: Record<string, string>;
      };
      const stored: StoredProfile = {
        name,
        status: 'valid',
        validated_at: iso('2026-07-19T10:00:00Z'),
        install: spec.install ?? [],
        variables: spec.variables ?? {},
        // Contract: only KEY names are ever returned; values stay write-only.
        secret_keys: Object.keys(spec.secrets ?? {}),
      };
      profiles.set(name, stored);
      await json(route, stored);
      return true;
    }

    if (name && method === 'DELETE') {
      profiles.delete(name);
      await route.fulfill({ status: 204, body: '' });
      return true;
    }

    return false;
  };

  return { profiles, handle };
}
