import { describe, expect, it, vi } from 'vitest';
import { runProposal, runProposalAsMutation } from './proposal-exec';
import type { ActionProposal } from './action-types';

/**
 * These tests pin the security-relevant property of this module: each `kind` reaches
 * ONE endpoint, and `target` — which is model-influenced text — never drives a request.
 */

const target = { method: 'X', path: '/never/read/this' };

/** A fetch stub recording every call, answering with `body` at `status`. */
function stubFetch(status: number, body: unknown = {}) {
  const calls: { url: string; init?: RequestInit }[] = [];
  const apiFetch = vi.fn(async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    return {
      ok: status >= 200 && status < 300,
      status,
      json: async () => body,
      text: async () => JSON.stringify(body),
      headers: new Headers({ 'content-type': 'application/json' }),
    } as unknown as Response;
  });
  return { apiFetch: apiFetch as never, calls };
}

const sessionProposal: ActionProposal = {
  kind: 'create_session',
  owner: 'acme',
  name: 'site',
  request: {
    name: 'sitebuilder',
    packages: ['acme/pkgs@main:packages/site'],
    manifests: [],
    log_access: [],
    collaborators: [],
  },
  rendered_issue_body: '### Session Name\nsitebuilder',
  summary: 'Start a session',
  target,
};

const envProposal: ActionProposal = {
  kind: 'save_environment_profile',
  profile_name: 'node-ci',
  replaces_existing: false,
  install: ['npm ci'],
  variables: [{ key: 'NODE_ENV', value: 'production' }],
  secret_keys: ['NPM_TOKEN'],
  summary: 'Create node-ci',
  target,
};

describe('runProposal', () => {
  it('routes a session draft to the trigger endpoint and returns the created issue', async () => {
    const { apiFetch, calls } = stubFetch(201, {
      issue_number: 42,
      html_url: 'https://github.com/acme/site/issues/42',
    });
    const outcome = await runProposal(apiFetch, sessionProposal);
    expect(outcome).toEqual({
      ok: true,
      issueNumber: 42,
      issueUrl: 'https://github.com/acme/site/issues/42',
    });
    expect(calls[0]!.url).toBe('/api/v1/repos/acme/site/sessions');
    // The load-bearing assertion: `target.path` is display copy, never a request.
    expect(calls[0]!.url).not.toContain('never/read/this');
  });

  it('routes a work item to the work-items endpoint', async () => {
    const { apiFetch, calls } = stubFetch(201, { issue_number: 7, html_url: 'u' });
    const outcome = await runProposal(apiFetch, {
      kind: 'create_work_item',
      owner: 'acme',
      name: 'site',
      trigger_issue_number: 3,
      title: 'Do the thing',
      body: '',
      summary: 'queue',
      target,
    });
    expect(outcome.ok).toBe(true);
    expect(calls[0]!.url).toBe('/api/v1/repos/acme/site/sessions/3/work-items');
  });

  it('routes a stop to a DELETE on the trigger', async () => {
    const { apiFetch, calls } = stubFetch(204);
    const outcome = await runProposal(apiFetch, {
      kind: 'stop_session',
      owner: 'acme',
      name: 'site',
      trigger_issue_number: 3,
      reason: 'done',
      summary: 'stop',
      target,
    });
    expect(outcome).toEqual({ ok: true });
    expect(calls[0]!.url).toBe('/api/v1/repos/acme/site/sessions/3');
    expect(calls[0]!.init?.method).toBe('DELETE');
  });

  it('routes a repository draft to POST /repos with its visibility', async () => {
    const { apiFetch, calls } = stubFetch(201, { owner: 'me', name: 'x', private: true });
    const outcome = await runProposal(apiFetch, {
      kind: 'create_repository',
      name: 'site-builder',
      private: true,
      summary: 'create',
      target,
    });
    expect(outcome.ok).toBe(true);
    expect(calls[0]!.url).toBe('/api/v1/repos');
    const sent = JSON.parse(String(calls[0]!.init?.body));
    expect(sent).toEqual({ name: 'site-builder', private: true });
    // A personal-account draft must not send an `owner` at all.
    expect('owner' in sent).toBe(false);
  });

  it('sends the card-collected secrets with an environment save', async () => {
    const { apiFetch, calls } = stubFetch(200, { name: 'node-ci' });
    const outcome = await runProposal(apiFetch, envProposal, {
      secrets: { NPM_TOKEN: 'npm_live_value' },
    });
    expect(outcome.ok).toBe(true);
    expect(calls[0]!.url).toBe('/api/v1/users/me/environment-profiles/node-ci');
    expect(calls[0]!.init?.method).toBe('PUT');
    expect(JSON.parse(String(calls[0]!.init?.body))).toEqual({
      install: ['npm ci'],
      variables: { NODE_ENV: 'production' },
      secrets: { NPM_TOKEN: 'npm_live_value' },
    });
  });

  it('sends no secrets when the card collected none', async () => {
    const { apiFetch, calls } = stubFetch(200, { name: 'node-ci' });
    await runProposal(apiFetch, { ...envProposal, secret_keys: [] });
    expect(JSON.parse(String(calls[0]!.init?.body)).secrets).toEqual({});
  });

  it('turns a failed install validation into a message naming the command', async () => {
    // The bare envelope ("install validation failed") is useless; the failing command
    // and its stderr tail are the whole diagnosis.
    const { apiFetch } = stubFetch(422, {
      error: 'install_validation_failed',
      message: 'install validation failed',
      failed_command_index: 0,
      failed_command: 'npm ci',
      exit_code: 1,
      timed_out: false,
      stderr_tail: 'ENOENT: no package-lock.json',
    });
    const outcome = await runProposal(apiFetch, envProposal, { secrets: { NPM_TOKEN: 'x' } });
    expect(outcome.ok).toBe(false);
    expect(outcome.message).toContain('npm ci');
    expect(outcome.message).toContain('exit 1');
    expect(outcome.message).toContain('ENOENT');
  });

  it('reports a timed-out validation as a timeout rather than a failing command', async () => {
    const { apiFetch } = stubFetch(422, {
      error: 'install_validation_failed',
      message: 'install validation timed out',
      failed_command_index: 0,
      failed_command: '',
      exit_code: -1,
      timed_out: true,
      stderr_tail: '',
    });
    const outcome = await runProposal(apiFetch, envProposal, { secrets: { NPM_TOKEN: 'x' } });
    expect(outcome.message).toContain('deadline');
    expect(outcome.message).not.toContain('exit -1');
  });

  it('routes an environment delete to DELETE on that profile', async () => {
    const { apiFetch, calls } = stubFetch(204);
    const outcome = await runProposal(apiFetch, {
      kind: 'delete_environment_profile',
      profile_name: 'node-ci',
      summary: 'delete',
      target,
    });
    expect(outcome).toEqual({ ok: true });
    expect(calls[0]!.url).toBe('/api/v1/users/me/environment-profiles/node-ci');
    expect(calls[0]!.init?.method).toBe('DELETE');
  });

  it('routes an uninstall to DELETE on the installation', async () => {
    const { apiFetch, calls } = stubFetch(204);
    const outcome = await runProposal(apiFetch, {
      kind: 'uninstall_app',
      owner: 'acme',
      reason: 'consolidating',
      summary: 'uninstall',
      target,
    });
    expect(outcome).toEqual({ ok: true });
    expect(calls[0]!.url).toBe('/api/v1/installations/acme');
    expect(calls[0]!.init?.method).toBe('DELETE');
  });

  it('surfaces a server refusal as a failed outcome carrying its message', async () => {
    const { apiFetch } = stubFetch(403, { error: 'forbidden', message: 'not allowlisted' });
    const outcome = await runProposal(apiFetch, sessionProposal);
    expect(outcome.ok).toBe(false);
    expect(outcome.message).toBe('not allowlisted');
  });
});

describe('runProposalAsMutation', () => {
  it('adapts a success to the dialog result shape', async () => {
    const { apiFetch } = stubFetch(204);
    await expect(
      runProposalAsMutation(apiFetch, {
        kind: 'uninstall_app',
        owner: 'acme',
        reason: 'x',
        summary: 'y',
        target,
      })
    ).resolves.toEqual({ ok: true, data: null });
  });

  it('adapts a failure, keeping the server message', async () => {
    const { apiFetch } = stubFetch(409, { error: 'conflict', message: 'still running' });
    await expect(
      runProposalAsMutation(apiFetch, {
        kind: 'delete_environment_profile',
        profile_name: 'node-ci',
        summary: 'y',
        target,
      })
    ).resolves.toEqual({ ok: false, message: 'still running' });
  });
});
