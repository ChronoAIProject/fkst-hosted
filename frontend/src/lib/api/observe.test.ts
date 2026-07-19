import { describe, it, expect, vi } from 'vitest';
import { getObserve, ObserveError } from './observe';
import type { ApiFetch } from './canvas';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

describe('getObserve', () => {
  it('GETs the encoded session observe path and returns the snapshot', async () => {
    const snap = { queues: [{ queue: 'q', depth: 3 }], deliveries: [] };
    const apiFetch = vi.fn(async () => jsonResponse(snap)) as ApiFetch;
    const body = await getObserve(apiFetch, 'abc 123');
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/sessions/abc%20123/observe');
    expect(body).toEqual(snap);
  });

  it('degrades a non-object body to an empty snapshot (never assumes shape)', async () => {
    expect(await getObserve((async () => jsonResponse(null)) as ApiFetch, 's')).toEqual({});
    expect(await getObserve((async () => jsonResponse(42)) as ApiFetch, 's')).toEqual({});
    // An array is not the object envelope either.
    expect(await getObserve((async () => jsonResponse([1, 2])) as ApiFetch, 's')).toEqual({});
  });

  it('throws a status-carrying ObserveError on a non-2xx (409 = no durable store)', async () => {
    const apiFetch = (async () => jsonResponse({ error: 'no_store' }, 409)) as ApiFetch;
    const err = await getObserve(apiFetch, 's').catch((e) => e);
    expect(err).toBeInstanceOf(ObserveError);
    expect((err as ObserveError).status).toBe(409);
  });
});
