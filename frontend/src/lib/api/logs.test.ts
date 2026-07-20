import { describe, it, expect, vi } from 'vitest';
import { getLogFile, getLogManifest, getLogRuns, LogError, DEFAULT_LOG_TAIL_BYTES } from './logs';
import type { ApiFetch } from './canvas';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

const manifestBody = {
  session_id: 'sess-1',
  generated_at: '2026-07-19T00:00:00Z',
  files: [{ path: 'fkst-substrate/codex/codex.log', size: 4096, label: 'Codex' }],
};

const runsBody = [
  { run_id: 'r-2', started_at: '2026-07-20T02:00:00Z' },
  { run_id: 'r-1', started_at: '2026-07-20T01:00:00Z', ended_at: '2026-07-20T01:30:00Z' },
];

describe('getLogRuns', () => {
  it('GETs the encoded runs path and returns the payload (newest first)', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(runsBody)) as ApiFetch;
    const body = await getLogRuns(apiFetch, 'sess 1');
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/logs/sess%201/runs');
    expect(body).toHaveLength(2);
    expect(body[0]!.run_id).toBe('r-2');
  });

  it('throws a typed LogError carrying the status on a non-2xx', async () => {
    // 503 == log storage not configured; the caller distinguishes it by status.
    await expect(getLogRuns((async () => jsonResponse(null, 503)) as ApiFetch, 's')).rejects.toThrow(
      LogError
    );
    await expect(
      getLogRuns((async () => jsonResponse(null, 503)) as ApiFetch, 's').catch((e) => (e as LogError).status)
    ).resolves.toBe(503);
  });

  it('throws on a malformed (non-array) body', async () => {
    await expect(
      getLogRuns((async () => jsonResponse({ runs: [] })) as ApiFetch, 's')
    ).rejects.toThrow('malformed log runs');
  });
});

describe('getLogManifest', () => {
  it('GETs the encoded manifest path and returns the payload', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(manifestBody)) as ApiFetch;
    const body = await getLogManifest(apiFetch, 'sess 1');
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/logs/sess%201/manifest');
    expect(body.files).toHaveLength(1);
  });

  it('appends a run query for a concrete run, but not for latest/empty', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(manifestBody)) as ApiFetch;
    await getLogManifest(apiFetch, 'sess-1', 'r-2');
    expect(apiFetch).toHaveBeenLastCalledWith('/api/v1/logs/sess-1/manifest?run=r-2');
    // "latest" and an empty id are the backend default — no run param is sent.
    await getLogManifest(apiFetch, 'sess-1', 'latest');
    expect(apiFetch).toHaveBeenLastCalledWith('/api/v1/logs/sess-1/manifest');
    await getLogManifest(apiFetch, 'sess-1', '');
    expect(apiFetch).toHaveBeenLastCalledWith('/api/v1/logs/sess-1/manifest');
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

  it('appends a run query after path/tail for a concrete run, but not for latest', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(fileBody)) as ApiFetch;
    await getLogFile(apiFetch, 'sess-1', 'a/b.log', DEFAULT_LOG_TAIL_BYTES, 'r-2');
    expect(apiFetch).toHaveBeenLastCalledWith(
      `/api/v1/logs/sess-1/file?path=a%2Fb.log&tail_bytes=${DEFAULT_LOG_TAIL_BYTES}&run=r-2`
    );
    // No tail, concrete run.
    await getLogFile(apiFetch, 'sess-1', 'a/b.log', undefined, 'r-2');
    expect(apiFetch).toHaveBeenLastCalledWith('/api/v1/logs/sess-1/file?path=a%2Fb.log&run=r-2');
    // "latest" carries no run param (backend default).
    await getLogFile(apiFetch, 'sess-1', 'a/b.log', undefined, 'latest');
    expect(apiFetch).toHaveBeenLastCalledWith('/api/v1/logs/sess-1/file?path=a%2Fb.log');
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
