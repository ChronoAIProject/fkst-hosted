import { describe, it, expect, vi } from 'vitest';
import { getHealthReport, getSessionHealth, HealthError } from './health';
import type { ApiFetch } from './canvas';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

const listing = {
  session_id: 'sess-1',
  reports: [
    {
      id: 'report-2',
      generated_at: '2026-07-30T14:15:00Z',
      status: 'stalled',
      status_raw: 'stalled',
      headline: 'no movement',
      producer: 'fkst-health@0.1.0',
    },
  ],
  latest: {
    id: 'report-2',
    generated_at: '2026-07-30T14:15:00Z',
    status: 'stalled',
    status_raw: 'stalled',
    headline: 'no movement',
    producer: 'fkst-health@0.1.0',
  },
  staleness: { state: 'fresh', expected_interval_secs: 600, age_secs: 120 },
};

describe('getSessionHealth', () => {
  it('GETs the encoded health path and returns the payload', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(listing)) as ApiFetch;
    const body = await getSessionHealth(apiFetch, 'sess 1');
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/sessions/sess%201/health');
    expect(body.reports).toHaveLength(1);
    expect(body.staleness.state).toBe('fresh');
  });

  it('treats an empty listing as a normal result, not an error', async () => {
    const empty = { session_id: 'sess-1', reports: [], staleness: { state: 'never_reported' } };
    const apiFetch = vi.fn(async () => jsonResponse(empty)) as ApiFetch;
    const body = await getSessionHealth(apiFetch, 'sess-1');
    expect(body.reports).toEqual([]);
    expect(body.staleness.state).toBe('never_reported');
  });

  it('throws a typed error carrying the status so 503 is distinguishable', async () => {
    const apiFetch = vi.fn(async () => jsonResponse({}, 503)) as ApiFetch;
    await expect(getSessionHealth(apiFetch, 'sess-1')).rejects.toMatchObject({
      name: 'HealthError',
      status: 503,
    });
  });

  it.each([403, 404, 502])('surfaces status %i on the typed error', async (status) => {
    const apiFetch = vi.fn(async () => jsonResponse({}, status)) as ApiFetch;
    await expect(getSessionHealth(apiFetch, 'sess-1')).rejects.toBeInstanceOf(HealthError);
  });

  it('rejects a malformed payload rather than handing it to the UI', async () => {
    const apiFetch = vi.fn(async () => jsonResponse({ session_id: 'x' })) as ApiFetch;
    await expect(getSessionHealth(apiFetch, 'sess-1')).rejects.toThrow();
  });
});

describe('getHealthReport', () => {
  it('encodes both path segments', async () => {
    const apiFetch = vi.fn(async () =>
      jsonResponse({
        session_id: 'sess 1',
        id: 'r 2',
        generated_at: '2026-07-30T14:15:00Z',
        status: 'working',
        status_raw: 'working',
        headline: 'h',
        producer: 'p@1',
        expected_interval_secs: 600,
        evidence: [],
        work_items: [],
        body_markdown: '## body',
      })
    ) as ApiFetch;
    const report = await getHealthReport(apiFetch, 'sess 1', 'r 2');
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/sessions/sess%201/health/r%202');
    expect(report.body_markdown).toBe('## body');
  });

  it('throws a typed error on a non-2xx', async () => {
    const apiFetch = vi.fn(async () => jsonResponse({}, 404)) as ApiFetch;
    await expect(getHealthReport(apiFetch, 'sess-1', 'nope')).rejects.toMatchObject({
      name: 'HealthError',
      status: 404,
    });
  });
});
