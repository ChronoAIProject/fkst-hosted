import { describe, it, expect, vi } from 'vitest';
import { fetchBlob, getSessionOutcomes, saveBlob } from './outcomes';
import type { ApiFetch } from './canvas';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

function blobResponse(blob: Blob, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, blob: async () => blob } as Response;
}

const outcomesBody = {
  owner: 'shining',
  name: 'lab',
  trigger_issue: 7,
  prs: [
    {
      number: 12,
      title: 'feat: thing',
      html_url: 'https://github.com/shining/lab/pull/12',
      state: 'open',
      merged: false,
      work_issue: 9,
      files_error: false,
      files: [
        {
          filename: 'src/a.ts',
          status: 'added',
          additions: 10,
          deletions: 0,
          sha: 'deadbeef',
          previous_filename: null,
          kind: 'text',
          size_hint: 10,
        },
      ],
    },
  ],
};

describe('getSessionOutcomes', () => {
  it('GETs the encoded outcomes path and returns the payload', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(outcomesBody)) as ApiFetch;
    const body = await getSessionOutcomes(apiFetch, 'a b', 'lab', 7);
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/repos/a%20b/lab/sessions/7/outcomes');
    expect(body.prs[0]!.files[0]!.filename).toBe('src/a.ts');
  });

  it('throws on a non-2xx and on a malformed body', async () => {
    await expect(
      getSessionOutcomes((async () => jsonResponse(null, 404)) as ApiFetch, 'o', 'r', 1)
    ).rejects.toThrow('404');
    await expect(
      getSessionOutcomes((async () => jsonResponse({ owner: 'o' })) as ApiFetch, 'o', 'r', 1)
    ).rejects.toThrow('malformed session outcomes');
  });
});

describe('fetchBlob', () => {
  it('GETs the blob with the name query for preview (no download flag)', async () => {
    const blob = new Blob(['hi'], { type: 'text/plain' });
    const apiFetch = vi.fn(async () => blobResponse(blob)) as ApiFetch;
    const res = await fetchBlob(apiFetch, 'shining', 'lab', 'sha1', 'src/a.ts');
    expect(apiFetch).toHaveBeenCalledWith(
      '/api/v1/repos/shining/lab/blob/sha1?name=src%2Fa.ts'
    );
    expect(res).toEqual({ ok: true, blob });
  });

  it('adds download=1 when saving', async () => {
    const blob = new Blob(['x']);
    const apiFetch = vi.fn(async () => blobResponse(blob)) as ApiFetch;
    await fetchBlob(apiFetch, 'o', 'r', 'sha', 'f.png', true);
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/repos/o/r/blob/sha?name=f.png&download=1');
  });

  it('reports a typed failure, flagging 413 as tooLarge', async () => {
    expect(await fetchBlob((async () => blobResponse(new Blob(), 413)) as ApiFetch, 'o', 'r', 's', 'big.mp4')).toEqual({
      ok: false,
      tooLarge: true,
      status: 413,
    });
    expect(await fetchBlob((async () => blobResponse(new Blob(), 500)) as ApiFetch, 'o', 'r', 's', 'f')).toEqual({
      ok: false,
      tooLarge: false,
      status: 500,
    });
  });
});

describe('saveBlob', () => {
  it('creates an anchor, clicks it, and revokes the object URL', () => {
    // Fake timers so the deferred revoke runs while URL is still stubbed.
    vi.useFakeTimers();
    const createURL = vi.fn(() => 'blob:mock');
    const revokeURL = vi.fn();
    vi.stubGlobal('URL', { createObjectURL: createURL, revokeObjectURL: revokeURL });
    const clicked: string[] = [];
    const clickSpy = vi
      .spyOn(HTMLAnchorElement.prototype, 'click')
      .mockImplementation(function (this: HTMLAnchorElement) {
        clicked.push(this.download);
      });

    saveBlob(new Blob(['data']), 'report.txt');

    expect(createURL).toHaveBeenCalledTimes(1);
    expect(clicked).toEqual(['report.txt']);

    vi.runAllTimers();
    expect(revokeURL).toHaveBeenCalledWith('blob:mock');

    clickSpy.mockRestore();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });
});
