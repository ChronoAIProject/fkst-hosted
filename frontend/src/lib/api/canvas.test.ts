import { describe, it, expect, vi } from 'vitest';
import {
  createTrigger,
  getOverview,
  getRepoSessions,
  readErrorMessage,
  stopTrigger,
  uninstallApp,
} from './canvas';
import type { ApiFetch } from './canvas';

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as Response;
}

const overviewBody = {
  app_slug: 'chronoai-fkst',
  viewer: { login: 'shining' },
  accounts: [],
  totals: { sessions: 0, packages: [] },
  broader_oauth_available: false,
};

describe('getOverview', () => {
  it('GETs /api/v1/overview and returns the payload', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(overviewBody)) as ApiFetch;
    const body = await getOverview(apiFetch);
    // No broader token → single-arg call, byte-identical to before the header
    // was added (no init object, so no X-Github-Broader-Token).
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/overview');
    expect(body.viewer.login).toBe('shining');
  });

  it('omits the broader header when the token is null/undefined', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(overviewBody)) as ApiFetch;
    await getOverview(apiFetch, null);
    // Still the bare single-arg call — a null token adds no init object.
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/overview');
  });

  it('sends X-Github-Broader-Token when a broader token is present', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(overviewBody)) as ApiFetch;
    await getOverview(apiFetch, 'brd_tok');
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/overview', {
      headers: { 'X-Github-Broader-Token': 'brd_tok' },
    });
  });

  it('throws on a non-2xx status', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(null, 500)) as ApiFetch;
    await expect(getOverview(apiFetch)).rejects.toThrow('500');
  });

  it('throws loudly on a malformed payload', async () => {
    const apiFetch = vi.fn(async () => jsonResponse({ nope: true })) as ApiFetch;
    await expect(getOverview(apiFetch)).rejects.toThrow('malformed overview');
  });
});

describe('getRepoSessions', () => {
  it('GETs the encoded repo path', async () => {
    const apiFetch = vi.fn(async () =>
      jsonResponse({ owner: 'a b', name: 'x', installed: true, sessions: [] })
    ) as ApiFetch;
    await getRepoSessions(apiFetch, 'a b', 'x');
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/repos/a%20b/x/sessions');
  });

  it('throws on failure status and malformed body', async () => {
    await expect(
      getRepoSessions((async () => jsonResponse(null, 404)) as ApiFetch, 'o', 'r')
    ).rejects.toThrow('404');
    await expect(
      getRepoSessions((async () => jsonResponse({})) as ApiFetch, 'o', 'r')
    ).rejects.toThrow('malformed repo sessions');
  });
});

describe('createTrigger', () => {
  it('POSTs the request body to the sessions endpoint', async () => {
    const apiFetch = vi.fn(async () =>
      jsonResponse({ issue_number: 7, html_url: 'https://github.com/o/r/issues/7' }, 201)
    ) as ApiFetch;
    const result = await createTrigger(apiFetch, 'o', 'r', {
      name: 'sess',
      packages: ['o/p@main:pkg'],
      auto_merge: true,
    });
    expect(result).toEqual({
      ok: true,
      data: { issue_number: 7, html_url: 'https://github.com/o/r/issues/7' },
    });
    const [path, init] = (apiFetch as ReturnType<typeof vi.fn>).mock.calls[0]! as [
      string,
      RequestInit,
    ];
    expect(path).toBe('/api/v1/repos/o/r/sessions');
    expect(init.method).toBe('POST');
    expect(JSON.parse(String(init.body))).toEqual({
      name: 'sess',
      packages: ['o/p@main:pkg'],
      auto_merge: true,
    });
  });

  it('carries the 400 envelope message back verbatim', async () => {
    const message = 'Packages: line 1 is not a valid owner/repo@ref:path reference.';
    const apiFetch = (async () =>
      jsonResponse({ error: 'invalid_trigger', message }, 400)) as ApiFetch;
    expect(await createTrigger(apiFetch, 'o', 'r', { name: 'x', packages: ['bad'] })).toEqual({
      ok: false,
      message,
    });
  });
});

describe('stopTrigger', () => {
  it('DELETEs the trigger issue path', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(null, 204)) as ApiFetch;
    expect(await stopTrigger(apiFetch, 'o', 'r', 42)).toEqual({ ok: true, data: null });
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/repos/o/r/sessions/42', { method: 'DELETE' });
  });

  it('returns the envelope message on failure', async () => {
    const apiFetch = (async () =>
      jsonResponse({ error: 'not_found', message: 'No such trigger issue.' }, 404)) as ApiFetch;
    expect(await stopTrigger(apiFetch, 'o', 'r', 42)).toEqual({
      ok: false,
      message: 'No such trigger issue.',
    });
  });
});

describe('uninstallApp', () => {
  it('DELETEs the encoded installation path', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(null, 204)) as ApiFetch;
    expect(await uninstallApp(apiFetch, 'a b')).toEqual({ ok: true, data: null });
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/installations/a%20b', { method: 'DELETE' });
  });

  it('returns the envelope message on failure', async () => {
    const apiFetch = (async () =>
      jsonResponse({ error: 'not_found', message: 'No installation for this account.' }, 404)) as ApiFetch;
    expect(await uninstallApp(apiFetch, 'shining')).toEqual({
      ok: false,
      message: 'No installation for this account.',
    });
  });
});

describe('readErrorMessage', () => {
  it('returns null for non-JSON or message-less bodies', async () => {
    expect(
      await readErrorMessage({
        json: async () => {
          throw new Error('not json');
        },
      } as unknown as Response)
    ).toBeNull();
    expect(await readErrorMessage(jsonResponse({ error: 'x' }, 500))).toBeNull();
  });
});
