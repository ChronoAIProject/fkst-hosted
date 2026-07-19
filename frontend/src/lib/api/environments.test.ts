import { describe, it, expect, vi } from 'vitest';
import {
  deleteEnvironmentProfile,
  getEnvironmentProfile,
  listEnvironmentProfiles,
  putEnvironmentProfile,
} from './environments';
import type { ApiFetch } from './canvas';
import type { EnvironmentProfileSpec, InstallValidationError } from './types';

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as Response;
}

const summary = {
  name: 'video-studio',
  status: 'ready',
  validated_at: '2026-07-19T00:00:00Z',
  install_command_count: 2,
  variable_count: 1,
  secret_count: 1,
};

const view = {
  name: 'video-studio',
  status: 'ready',
  validated_at: '2026-07-19T00:00:00Z',
  install: ['pip install ffmpeg-python'],
  variables: { REGION: 'us' },
  secret_keys: ['OPENAI_API_KEY'],
};

const spec: EnvironmentProfileSpec = {
  install: ['pip install ffmpeg-python'],
  variables: { REGION: 'us' },
  secrets: { OPENAI_API_KEY: 'sk-secret' },
};

describe('listEnvironmentProfiles', () => {
  it('GETs the collection and unwraps environment_profiles', async () => {
    const apiFetch = vi.fn(async () =>
      jsonResponse({ environment_profiles: [summary] })
    ) as ApiFetch;
    const result = await listEnvironmentProfiles(apiFetch);
    expect(apiFetch).toHaveBeenCalledWith('/api/v1/users/me/environment-profiles');
    expect(result).toEqual([summary]);
  });

  it('throws on a non-2xx status', async () => {
    const apiFetch = (async () => jsonResponse(null, 503)) as ApiFetch;
    await expect(listEnvironmentProfiles(apiFetch)).rejects.toThrow('503');
  });

  it('throws loudly on a malformed payload', async () => {
    const apiFetch = (async () => jsonResponse({ nope: true })) as ApiFetch;
    await expect(listEnvironmentProfiles(apiFetch)).rejects.toThrow('malformed environment profiles');
  });
});

describe('getEnvironmentProfile', () => {
  it('GETs the encoded name path and returns the view', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(view)) as ApiFetch;
    const result = await getEnvironmentProfile(apiFetch, 'video studio');
    expect(apiFetch).toHaveBeenCalledWith(
      '/api/v1/users/me/environment-profiles/video%20studio'
    );
    expect(result.secret_keys).toEqual(['OPENAI_API_KEY']);
  });

  it('throws on a 404 status', async () => {
    const apiFetch = (async () => jsonResponse(null, 404)) as ApiFetch;
    await expect(getEnvironmentProfile(apiFetch, 'missing')).rejects.toThrow('404');
  });

  it('throws loudly when the body is missing required fields', async () => {
    const apiFetch = (async () => jsonResponse({ name: 'x' })) as ApiFetch;
    await expect(getEnvironmentProfile(apiFetch, 'x')).rejects.toThrow(
      'malformed environment profile'
    );
  });
});

describe('putEnvironmentProfile', () => {
  it('PUTs the spec and returns the created/updated view', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(view, 201)) as ApiFetch;
    const result = await putEnvironmentProfile(apiFetch, 'video-studio', spec);
    expect(result).toEqual({ ok: true, data: view });
    const [path, init] = (apiFetch as ReturnType<typeof vi.fn>).mock.calls[0]! as [
      string,
      RequestInit,
    ];
    expect(path).toBe('/api/v1/users/me/environment-profiles/video-studio');
    expect(init.method).toBe('PUT');
    expect(JSON.parse(String(init.body))).toEqual(spec);
  });

  it('surfaces the install-validation 422 body verbatim as a typed result', async () => {
    const validation: InstallValidationError = {
      error: 'install_validation_failed',
      message: 'install command 1 of 1 failed',
      failed_command_index: 1,
      failed_command: 'pip install ffmpeg-python',
      exit_code: 1,
      timed_out: false,
      stderr_tail: 'ERROR: could not find a version',
    };
    const apiFetch = (async () => jsonResponse(validation, 422)) as ApiFetch;
    const result = await putEnvironmentProfile(apiFetch, 'video-studio', spec);
    expect(result).toEqual({ ok: false, validation });
    // The report rides back as a structured object, never a thrown string.
    if (!result.ok && 'validation' in result) {
      expect(result.validation.exit_code).toBe(1);
    } else {
      throw new Error('expected a typed validation failure');
    }
  });

  it('treats a plain-envelope 422 (pre-validation) as a message failure', async () => {
    const message = 'invalid environment name "BAD": must match ...';
    const apiFetch = (async () =>
      jsonResponse({ error: 'unprocessable', message }, 422)) as ApiFetch;
    const result = await putEnvironmentProfile(apiFetch, 'BAD', spec);
    expect(result).toEqual({ ok: false, message });
  });

  it('carries the envelope message for other failures (e.g. 429)', async () => {
    const message = 'validation capacity busy';
    const apiFetch = (async () =>
      jsonResponse({ error: 'too_many_requests', message }, 429)) as ApiFetch;
    expect(await putEnvironmentProfile(apiFetch, 'video-studio', spec)).toEqual({
      ok: false,
      message,
    });
  });

  it('falls back to a null message when the failure body is not JSON', async () => {
    const apiFetch = (async () =>
      ({
        ok: false,
        status: 500,
        json: async () => {
          throw new Error('not json');
        },
      }) as unknown as Response) as ApiFetch;
    expect(await putEnvironmentProfile(apiFetch, 'video-studio', spec)).toEqual({
      ok: false,
      message: null,
    });
  });
});

describe('deleteEnvironmentProfile', () => {
  it('DELETEs the encoded name path', async () => {
    const apiFetch = vi.fn(async () => jsonResponse(null, 204)) as ApiFetch;
    expect(await deleteEnvironmentProfile(apiFetch, 'video studio')).toEqual({
      ok: true,
      data: null,
    });
    expect(apiFetch).toHaveBeenCalledWith(
      '/api/v1/users/me/environment-profiles/video%20studio',
      { method: 'DELETE' }
    );
  });

  it('returns the envelope message on failure', async () => {
    const apiFetch = (async () =>
      jsonResponse({ error: 'unavailable', message: 'environment store backend unavailable' }, 503)) as ApiFetch;
    expect(await deleteEnvironmentProfile(apiFetch, 'x')).toEqual({
      ok: false,
      message: 'environment store backend unavailable',
    });
  });
});
