import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { AuthProvider } from '@/lib/auth/github-auth';
import { ToastProvider, Toaster } from '@/components/ui/toast';
import { CreateTriggerModal, buildCreateRequest } from './create-trigger-modal';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

/** URL discriminators for the two endpoints the modal touches. */
const ENV_PATH = '/api/v1/users/me/environment-profiles';
const SESSIONS_PATH = '/sessions';

function renderModal(
  over: {
    onCreated?: (r: { issue_number: number; html_url: string }) => void;
    inUseWorkLabels?: readonly string[];
  } = {}
) {
  const onCreated = over.onCreated ?? vi.fn();
  const onClose = vi.fn();
  render(
    <MemoryRouter>
      <AuthProvider>
        <ToastProvider>
          <CreateTriggerModal
            owner="acme"
            name="app"
            inUseWorkLabels={over.inUseWorkLabels}
            onClose={onClose}
            onCreated={onCreated}
          />
          <Toaster />
        </ToastProvider>
      </AuthProvider>
    </MemoryRouter>
  );
  return { onCreated, onClose };
}

describe('buildCreateRequest', () => {
  it('trims name/packages and omits every blank optional section', () => {
    const req = buildCreateRequest({
      name: '  nightly  ',
      packages: ['  a/b@main:pkg  ', ''],
      manifests: '   \n  ',
      workLabel: '   ',
      environment: '   ',
      sourceBranch: '   ',
      targetBranch: '   ',
      autoMerge: false,
      logAccess: '   ',
      collaborators: '   ',
      outputLang: '  ',
    });
    expect(req).toEqual({ name: 'nightly', packages: ['a/b@main:pkg'] });
  });

  it('includes every optional section once populated', () => {
    const req = buildCreateRequest({
      name: 'n',
      packages: ['p'],
      manifests: 'o/m@main:bundle-a\n  o/m@main:bundle-b  ',
      workLabel: 'ready',
      environment: 'staging',
      sourceBranch: ' release/v1.2 ',
      targetBranch: ' feature/site ',
      autoMerge: true,
      logAccess: 'alice, bob',
      collaborators: 'worker helper',
      outputLang: 'English',
    });
    expect(req).toEqual({
      name: 'n',
      packages: ['p'],
      manifests: ['o/m@main:bundle-a', 'o/m@main:bundle-b'],
      work_label: 'ready',
      environment: 'staging',
      source_branch: 'release/v1.2',
      target_branch: 'feature/site',
      auto_merge: true,
      log_access: ['alice', 'bob'],
      collaborators: ['worker', 'helper'],
      output_lang: 'English',
    });
  });

  it('sends a disposable environment instead of a saved profile', () => {
    const req = buildCreateRequest({
      name: 'n',
      packages: ['p'],
      manifests: '',
      workLabel: '',
      environment: 'must-not-be-sent',
      disposableEnvironment: {
        install: ['apt-get install -y jq'],
        variables: { APP_MODE: 'test' },
        secrets: { DEPLOY_TOKEN: 'secret' },
      },
      sourceBranch: '',
      targetBranch: '',
      autoMerge: false,
      logAccess: '',
      collaborators: '',
      outputLang: '',
    });
    expect(req.disposable_environment).toEqual({
      install: ['apt-get install -y jq'],
      variables: { APP_MODE: 'test' },
      secrets: { DEPLOY_TOKEN: 'secret' },
    });
    expect(req.environment).toBeUndefined();
  });

  it('parses the manifest textarea one reference per line, dropping blanks', () => {
    const req = buildCreateRequest({
      name: 'n',
      packages: [],
      manifests: 'o/m@main:a\n\n  o/m@main:b  \n',
      workLabel: '',
      environment: '',
      sourceBranch: '',
      targetBranch: '',
      autoMerge: false,
      logAccess: '',
      collaborators: '',
      outputLang: '',
    });
    // A manifest-only request omits packages entirely (empty array) and carries
    // exactly the two non-blank lines.
    expect(req).toEqual({
      name: 'n',
      packages: [],
      manifests: ['o/m@main:a', 'o/m@main:b'],
    });
  });
});

describe('CreateTriggerModal', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });
  afterEach(() => vi.unstubAllGlobals());

  it('populates the environment <select> from the profile list', async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) {
          return jsonResponse({
            environment_profiles: [
              {
                name: 'staging',
                status: 'ready',
                validated_at: '',
                install_command_count: 0,
                variable_count: 0,
                secret_count: 0,
              },
              {
                name: 'prod',
                status: 'ready',
                validated_at: '',
                install_command_count: 0,
                variable_count: 0,
                secret_count: 0,
              },
            ],
          });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderModal();

    expect(screen.getByRole('button', { name: 'None' })).toHaveAttribute('aria-pressed', 'true');
    await user.click(screen.getByRole('button', { name: 'Saved profile' }));

    // The field is a <select> (not a free-text input) once the list loads.
    const select = (await screen.findByLabelText('Saved profile')) as HTMLSelectElement;
    expect(select.tagName).toBe('SELECT');
    // Blank "none" option + one per profile.
    expect(screen.getByRole('option', { name: 'None' })).toBeInTheDocument();
    expect(await screen.findByRole('option', { name: 'staging' })).toBeInTheDocument();
    expect(screen.getByRole('option', { name: 'prod' })).toBeInTheDocument();
    // The manager-hint note is present.
    expect(screen.getByText(/Environments manager in the top bar/)).toBeInTheDocument();
  });

  it('renders the one-label-per-trigger work-label hint with a Get-started link', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => jsonResponse({ environment_profiles: [] }))
    );
    renderModal();

    expect(await screen.findByText(/One work label per trigger/)).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Get started/ })).toHaveAttribute(
      'href',
      '/get-started'
    );
  });

  it('falls back to a free-text environment input when the profile fetch fails', async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) return jsonResponse({}, 503);
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderModal();
    await user.click(screen.getByRole('button', { name: 'Saved profile' }));

    // Degrades to an <input>, never blocking the dialog, plus the failure note.
    // Wait on the note (only rendered in the error branch) before reading the
    // field, so we don't sample the initial disabled <select>.
    expect(await screen.findByText(/Could not load your environments/)).toBeInTheDocument();
    const field = screen.getByLabelText('Saved profile') as HTMLElement;
    expect(field.tagName).toBe('INPUT');
  });

  it('shows the disposable editor with a masked secret field', async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => jsonResponse({ environment_profiles: [] }))
    );
    renderModal();

    expect(screen.queryByLabelText('Software installation commands 1')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Disposable' }));

    expect(screen.getByLabelText('Software installation commands 1')).toBeInTheDocument();
    expect(screen.getByLabelText('Secrets 1 secret value')).toHaveAttribute('type', 'password');
    expect(screen.getByText(/Add at least one command, variable, or secret/)).toBeInTheDocument();
  });

  it('opens a value-free confirmation and returns to the populated form on back', async () => {
    const user = userEvent.setup();
    let sessionWrites = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) return jsonResponse({ environment_profiles: [] });
        if (url.includes(SESSIONS_PATH)) {
          sessionWrites += 1;
          return jsonResponse({
            issue_number: 51,
            html_url: 'https://github.com/acme/app/issues/51',
          });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderModal();

    await user.type(await screen.findByLabelText('Session name'), 'private-run');
    await user.type(screen.getByLabelText('Packages 1'), 'a/b@main:pkg');
    await user.click(screen.getByRole('button', { name: 'Disposable' }));
    await user.type(
      screen.getByLabelText('Software installation commands 1'),
      'install-private-tool'
    );
    await user.type(screen.getByLabelText('Environment variables 1 NAME'), 'APP_MODE');
    await user.type(screen.getByLabelText('Environment variables 1 value'), 'private-mode');
    await user.type(screen.getByLabelText('Secrets 1 NAME'), 'DEPLOY_TOKEN');
    await user.type(screen.getByLabelText('Secrets 1 secret value'), 'super-secret-value');
    await user.click(screen.getByRole('button', { name: 'Create trigger issue' }));

    expect(await screen.findByText('Confirm disposable environment')).toBeInTheDocument();
    expect(sessionWrites).toBe(0);
    expect(screen.queryByText('install-private-tool')).not.toBeInTheDocument();
    expect(screen.queryByText('APP_MODE')).not.toBeInTheDocument();
    expect(screen.queryByText('private-mode')).not.toBeInTheDocument();
    expect(screen.queryByText('DEPLOY_TOKEN')).not.toBeInTheDocument();
    expect(screen.queryByText('super-secret-value')).not.toBeInTheDocument();
    expect(screen.getByText('Installation commands').parentElement).toHaveTextContent('1');
    expect(screen.getByText('Variables').parentElement).toHaveTextContent('1');
    expect(screen.getByText('Secrets').parentElement).toHaveTextContent('1');

    await user.click(screen.getByRole('button', { name: 'Back to edit' }));
    expect(await screen.findByLabelText('Session name')).toHaveValue('private-run');
    expect(screen.getByLabelText('Software installation commands 1')).toHaveValue(
      'install-private-tool'
    );
    expect(screen.getByLabelText('Secrets 1 secret value')).toHaveValue('super-secret-value');
    expect(sessionWrites).toBe(0);
  });

  it('confirms one disposable request with no saved environment field', async () => {
    const user = userEvent.setup();
    const sentBodies: Array<Record<string, unknown>> = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) return jsonResponse({ environment_profiles: [] });
        if (url.includes(SESSIONS_PATH)) {
          sentBodies.push(JSON.parse(String(init?.body)) as Record<string, unknown>);
          return jsonResponse({
            issue_number: 52,
            html_url: 'https://github.com/acme/app/issues/52',
          });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    const { onCreated } = renderModal();

    await user.type(await screen.findByLabelText('Session name'), 'private-run');
    await user.type(screen.getByLabelText('Packages 1'), 'a/b@main:pkg');
    await user.click(screen.getByRole('button', { name: 'Disposable' }));
    await user.type(screen.getByLabelText('Software installation commands 1'), 'npm ci');
    await user.type(screen.getByLabelText('Environment variables 1 NAME'), 'APP_MODE');
    await user.type(screen.getByLabelText('Environment variables 1 value'), 'test');
    await user.type(screen.getByLabelText('Secrets 1 NAME'), 'DEPLOY_TOKEN');
    await user.type(screen.getByLabelText('Secrets 1 secret value'), 'secret-value');
    await user.click(screen.getByRole('button', { name: 'Create trigger issue' }));

    expect(sentBodies).toHaveLength(0);
    await user.click(await screen.findByRole('button', { name: 'Confirm and create' }));
    await waitFor(() => expect(sentBodies).toHaveLength(1));
    const [sentBody] = sentBodies;
    expect(sentBody).toBeDefined();
    if (!sentBody) throw new Error('expected one session request');
    expect(sentBody).toMatchObject({
      name: 'private-run',
      packages: ['a/b@main:pkg'],
      disposable_environment: {
        install: ['npm ci'],
        variables: { APP_MODE: 'test' },
        secrets: { DEPLOY_TOKEN: 'secret-value' },
      },
    });
    expect(sentBody.environment).toBeUndefined();
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
  });

  it('shows a success toast and hands back the created session on submit', async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) return jsonResponse({ environment_profiles: [] });
        if (url.includes(SESSIONS_PATH)) {
          return jsonResponse({
            issue_number: 42,
            html_url: 'https://github.com/acme/app/issues/42',
          });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    const { onCreated } = renderModal();

    await user.type(await screen.findByLabelText('Session name'), 'nightly');
    await user.type(screen.getByLabelText('Packages 1'), 'a/b@main:pkg');
    await user.click(screen.getByRole('button', { name: 'Create trigger issue' }));

    expect(await screen.findByText('Session created')).toBeInTheDocument();
    await waitFor(() =>
      expect(onCreated).toHaveBeenCalledWith({
        issue_number: 42,
        html_url: 'https://github.com/acme/app/issues/42',
      })
    );
  });

  it('sends the collaborators input as its own request field, distinct from log access', async () => {
    const user = userEvent.setup();
    let sentBody: unknown;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) return jsonResponse({ environment_profiles: [] });
        if (url.includes(SESSIONS_PATH)) {
          sentBody = JSON.parse(String(init?.body));
          return jsonResponse({
            issue_number: 7,
            html_url: 'https://github.com/acme/app/issues/7',
          });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderModal();

    await user.type(await screen.findByLabelText('Session name'), 'nightly');
    await user.type(screen.getByLabelText('Packages 1'), 'a/b@main:pkg');
    // The collaborators field is present, labelled as its own input, and its
    // hint marks it as work-item authority distinct from log access.
    await user.type(screen.getByLabelText('Collaborators (optional)'), 'worker, helper');
    expect(screen.getByText(/granted work-item authority/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Create trigger issue' }));

    await waitFor(() => expect(sentBody).toBeDefined());
    expect(sentBody).toMatchObject({
      name: 'nightly',
      packages: ['a/b@main:pkg'],
      collaborators: ['worker', 'helper'],
    });
    // Nothing was typed into log access, so it must be absent (distinct field).
    expect((sentBody as Record<string, unknown>).log_access).toBeUndefined();
  });

  it('sends the manifest textarea as its own request field, one ref per line', async () => {
    const user = userEvent.setup();
    let sentBody: unknown;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) return jsonResponse({ environment_profiles: [] });
        if (url.includes(SESSIONS_PATH)) {
          sentBody = JSON.parse(String(init?.body));
          return jsonResponse({
            issue_number: 8,
            html_url: 'https://github.com/acme/app/issues/8',
          });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderModal();

    await user.type(await screen.findByLabelText('Session name'), 'nightly');
    await user.type(screen.getByLabelText('Packages 1'), 'a/b@main:pkg');
    // The manifest field is present and labelled as its own input; two lines
    // become two request entries.
    await user.type(
      screen.getByLabelText('Manifests (optional)'),
      'o/m@main:bundle-a\no/m@main:bundle-b'
    );
    await user.click(screen.getByRole('button', { name: 'Create trigger issue' }));

    await waitFor(() => expect(sentBody).toBeDefined());
    expect(sentBody).toMatchObject({
      name: 'nightly',
      packages: ['a/b@main:pkg'],
      manifests: ['o/m@main:bundle-a', 'o/m@main:bundle-b'],
    });
  });

  it('shows branch defaults and sends trimmed source and target branches', async () => {
    const user = userEvent.setup();
    let sentBody: Record<string, unknown> | undefined;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) return jsonResponse({ environment_profiles: [] });
        if (url.includes(SESSIONS_PATH)) {
          sentBody = JSON.parse(String(init?.body)) as Record<string, unknown>;
          return jsonResponse({
            issue_number: 10,
            html_url: 'https://github.com/acme/app/issues/10',
          });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderModal();

    expect(await screen.findByText('Advanced')).toBeInTheDocument();
    expect(screen.getByLabelText('Source branch (optional)')).toHaveAttribute(
      'placeholder',
      'Repository default branch'
    );
    expect(screen.getByLabelText('Target branch (optional)')).toHaveAttribute(
      'placeholder',
      expect.stringContaining('fkst-hosted-default')
    );
    await user.type(screen.getByLabelText('Session name'), 'nightly');
    await user.type(screen.getByLabelText('Packages 1'), 'a/b@main:pkg');
    await user.type(screen.getByLabelText('Source branch (optional)'), ' release/v1.2 ');
    await user.type(screen.getByLabelText('Target branch (optional)'), ' feature/site ');
    await user.click(screen.getByRole('button', { name: 'Create trigger issue' }));

    await waitFor(() => expect(sentBody).toBeDefined());
    expect(sentBody).toMatchObject({
      source_branch: 'release/v1.2',
      target_branch: 'feature/site',
    });
  });

  it('blocks an invalid branch name with localized client validation', async () => {
    const user = userEvent.setup();
    const fetch = vi.fn(async () => jsonResponse({ environment_profiles: [] }));
    vi.stubGlobal('fetch', fetch);
    renderModal();

    await user.type(await screen.findByLabelText('Session name'), 'nightly');
    await user.type(screen.getByLabelText('Packages 1'), 'a/b@main:pkg');
    await user.type(screen.getByLabelText('Target branch (optional)'), 'bad branch');

    expect(screen.getByRole('alert')).toHaveTextContent(/Use 1–200 letters/);
    expect(screen.getByRole('button', { name: 'Create trigger issue' })).toBeDisabled();
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it('allows a manifest-only submit with no packages and no work label', async () => {
    const user = userEvent.setup();
    let sentBody: unknown;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) return jsonResponse({ environment_profiles: [] });
        if (url.includes(SESSIONS_PATH)) {
          sentBody = JSON.parse(String(init?.body));
          return jsonResponse({
            issue_number: 9,
            html_url: 'https://github.com/acme/app/issues/9',
          });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderModal();

    await user.type(await screen.findByLabelText('Session name'), 'nightly');
    const submit = screen.getByRole('button', { name: 'Create trigger issue' });
    // With no package source yet, submit is blocked.
    expect(submit).toBeDisabled();

    // A manifest reference alone is a valid package source — submit unblocks.
    await user.type(screen.getByLabelText('Manifests (optional)'), 'o/m@main:bundle');
    expect(submit).toBeEnabled();
    await user.click(submit);

    await waitFor(() => expect(sentBody).toBeDefined());
    expect(sentBody).toMatchObject({ name: 'nightly', manifests: ['o/m@main:bundle'] });
    // No packages typed and none defaulted: the empty array is sent, work label absent.
    const body = sentBody as Record<string, unknown>;
    expect(body.packages).toEqual([]);
    expect(body.work_label).toBeUndefined();
  });

  it('surfaces a server error and raises no toast when create fails', async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) return jsonResponse({ environment_profiles: [] });
        if (url.includes(SESSIONS_PATH)) {
          return jsonResponse({ error: 'invalid', message: 'bad work label' }, 400);
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    const { onCreated } = renderModal();

    await user.type(await screen.findByLabelText('Session name'), 'nightly');
    await user.type(screen.getByLabelText('Packages 1'), 'a/b@main:pkg');
    await user.click(screen.getByRole('button', { name: 'Create trigger issue' }));

    expect(await screen.findByText('bad work label')).toBeInTheDocument();
    expect(onCreated).not.toHaveBeenCalled();
    expect(screen.queryByText('Session created')).not.toBeInTheDocument();
  });

  it.each([
    [403, '@shining must have admin or maintain permission on acme/app to create a session'],
    [409, 'work label "site-build" is already in use by the open session #19'],
  ])('renders the backend %s message verbatim', async (status, message) => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) return jsonResponse({ environment_profiles: [] });
        if (url.includes(SESSIONS_PATH))
          return jsonResponse({ error: 'rejected', message }, status);
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderModal();

    await user.type(await screen.findByLabelText('Session name'), 'nightly');
    await user.type(screen.getByLabelText('Packages 1'), 'a/b@main:pkg');
    await user.click(screen.getByRole('button', { name: 'Create trigger issue' }));

    expect(await screen.findByText(message)).toBeInTheDocument();
  });
});

describe('CreateTriggerModal · work-label collision advisory', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => jsonResponse({ environment_profiles: [] }))
    );
  });
  afterEach(() => vi.unstubAllGlobals());

  it('warns and blocks submit once the typed label matches an in-use one', async () => {
    const user = userEvent.setup();
    renderModal({ inUseWorkLabels: ['fkst-work'] });

    await user.type(await screen.findByLabelText('Session name'), 'nightly');
    await user.type(screen.getByLabelText('Packages 1'), 'a/b@main:pkg');

    // A blank label is not a collision: the form is submittable and unwarned.
    const submit = screen.getByRole('button', { name: 'Create trigger issue' });
    expect(submit).toBeEnabled();
    expect(screen.queryByText(/already uses this work label/)).not.toBeInTheDocument();

    await user.type(screen.getByLabelText('Work label (optional)'), 'fkst-work');

    // Exact match with an open session's label: warning shown, submit disabled.
    expect(screen.getByRole('alert')).toHaveTextContent(/already uses this work label/);
    expect(submit).toBeDisabled();
  });

  it('leaves a label distinct from every in-use one unwarned and submittable', async () => {
    const user = userEvent.setup();
    renderModal({ inUseWorkLabels: ['fkst-work'] });

    await user.type(await screen.findByLabelText('Session name'), 'nightly');
    await user.type(screen.getByLabelText('Packages 1'), 'a/b@main:pkg');
    await user.type(screen.getByLabelText('Work label (optional)'), 'fkst-other');

    expect(screen.queryByText(/already uses this work label/)).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Create trigger issue' })).toBeEnabled();
  });

  it('does not warn on an empty label even when labels are in use', async () => {
    renderModal({ inUseWorkLabels: ['fkst-work'] });

    expect(await screen.findByLabelText('Work label (optional)')).toHaveValue('');
    expect(screen.queryByText(/already uses this work label/)).not.toBeInTheDocument();
  });
});
