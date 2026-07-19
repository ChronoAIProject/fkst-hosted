import { describe, it, expect, vi } from 'vitest';
import { getLogFile, getLogManifest, DEFAULT_LOG_TAIL_BYTES } from './logs';
import type { ApiFetch } from './canvas';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

const manifestBody = {
  session_id: 'sess-1',
  generated_at: '2026-07-19T00:00:00Z',
  files: [{ path: 'fkst-substrate/codex/codex.log', size: 4096, label: 'Codex' }],
};

describe('getLogManifest', () => {
  it('GETs the encoded manifest path and returns the payload', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(manifestBody)) as ApiFetch;
    const body = await getLogManifest(apiFetch, 'sess 1');
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/logs/sess%201/manifest');
    expect(body.files).toHaveLength(1);
  });

  it('throws on a non-2xx and on a malformed body', async () => {
    await expect(
      getLogManifest((async () => jsonResponse(null, 403)) as ApiFetch, 's')
    ).rejects.toThrow('403');
    await expect(
      getLogManifest((async () => jsonResponse({ session_id: 's' })) as ApiFetch, 's')
    ).rejects.toThrow('malformed log manifest');
  });
});

describe('getLogFile', () => {
  const fileBody = {
    session_id: 'sess-1',
    path: 'a/b.log',
    content: 'line 1\nline 2',
    total_bytes: 13,
    returned_bytes: 13,
    truncated: false,
  };

  it('GETs the file with the path query and no tail by default', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(fileBody)) as ApiFetch;
    const body = await getLogFile(apiFetch, 'sess-1', 'a/b.log');
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/logs/sess-1/file?path=a%2Fb.log');
    expect(body.content).toContain('line 1');
  });

  it('appends tail_bytes when a tail window is requested', async () => {
    const apiFetch = vi.fn(async () => jsonResponse({ ...fileBody, truncated: true })) as ApiFetch;
    await getLogFile(apiFetch, 'sess-1', 'a/b.log', DEFAULT_LOG_TAIL_BYTES);
    expect(apiFetch).toHaveBeenCalledWith(
      `/api/v1/logs/sess-1/file?path=a%2Fb.log&tail_bytes=${DEFAULT_LOG_TAIL_BYTES}`
    );
  });

  it('throws on a non-2xx (404 for an unknown path) and a malformed body', async () => {
    await expect(
      getLogFile((async () => jsonResponse(null, 404)) as ApiFetch, 's', 'nope')
    ).rejects.toThrow('404');
    await expect(
      getLogFile((async () => jsonResponse({ path: 'x' })) as ApiFetch, 's', 'x')
    ).rejects.toThrow('malformed log file');
  });
});
