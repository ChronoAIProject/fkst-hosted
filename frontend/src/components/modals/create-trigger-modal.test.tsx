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

function renderModal(over: { onCreated?: (r: { issue_number: number; html_url: string }) => void } = {}) {
  const onCreated = over.onCreated ?? vi.fn();
  const onClose = vi.fn();
  render(
    <MemoryRouter>
      <AuthProvider>
        <ToastProvider>
          <CreateTriggerModal
            owner="acme"
            name="app"
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
      workLabel: '   ',
      environment: '   ',
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
      workLabel: 'ready',
      environment: 'staging',
      autoMerge: true,
      logAccess: 'alice, bob',
      collaborators: 'worker helper',
      outputLang: 'English',
    });
    expect(req).toEqual({
      name: 'n',
      packages: ['p'],
      work_label: 'ready',
      environment: 'staging',
      auto_merge: true,
      log_access: ['alice', 'bob'],
      collaborators: ['worker', 'helper'],
      output_lang: 'English',
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
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) {
          return jsonResponse({
            environment_profiles: [
              { name: 'staging', status: 'ready', validated_at: '', install_command_count: 0, variable_count: 0, secret_count: 0 },
              { name: 'prod', status: 'ready', validated_at: '', install_command_count: 0, variable_count: 0, secret_count: 0 },
            ],
          });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderModal();

    // The field is a <select> (not a free-text input) once the list loads.
    const select = (await screen.findByLabelText('Environment (optional)')) as HTMLSelectElement;
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
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) return jsonResponse({}, 503);
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    renderModal();

    // Degrades to an <input>, never blocking the dialog, plus the failure note.
    // Wait on the note (only rendered in the error branch) before reading the
    // field, so we don't sample the initial disabled <select>.
    expect(await screen.findByText(/Could not load your environments/)).toBeInTheDocument();
    const field = screen.getByLabelText('Environment (optional)') as HTMLElement;
    expect(field.tagName).toBe('INPUT');
  });

  it('shows a success toast and hands back the created session on submit', async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes(ENV_PATH)) return jsonResponse({ environment_profiles: [] });
        if (url.includes(SESSIONS_PATH)) {
          return jsonResponse({ issue_number: 42, html_url: 'https://github.com/acme/app/issues/42' });
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
          return jsonResponse({ issue_number: 7, html_url: 'https://github.com/acme/app/issues/7' });
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
});
