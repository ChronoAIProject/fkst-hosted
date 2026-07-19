import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import { ToastProvider, Toaster } from '@/components/ui/toast';
import { EnvironmentsDrawer } from './environments-drawer';
import type {
  EnvironmentProfileSummary,
  EnvironmentProfileView,
  InstallValidationError,
} from '@/lib/api/types';

/** A minimal fetch Response stand-in the API layer understands. */
function res(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

function summary(over: Partial<EnvironmentProfileSummary> = {}): EnvironmentProfileSummary {
  return {
    name: 'py',
    status: 'validated',
    validated_at: '2026-07-01T10:00:00Z',
    install_command_count: 2,
    variable_count: 1,
    secret_count: 3,
    ...over,
  };
}

function view(over: Partial<EnvironmentProfileView> = {}): EnvironmentProfileView {
  return {
    name: 'py',
    status: 'validated',
    validated_at: '2026-07-01T10:00:00Z',
    install: ['pip install -r requirements.txt'],
    variables: { FOO: 'bar' },
    secret_keys: ['API_KEY'],
    ...over,
  };
}

/** Mount the drawer under the auth + toast providers a real app supplies. The
 *  Toaster render surface is included so success toasts are assertable. */
function renderDrawer() {
  return render(
    <AuthProvider>
      <ToastProvider>
        <EnvironmentsDrawer open onClose={() => {}} />
        <Toaster />
      </ToastProvider>
    </AuthProvider>
  );
}

describe('EnvironmentsDrawer', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    // A non-expiring token so apiFetch attaches auth and never hits the 401 path.
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });
  afterEach(() => vi.unstubAllGlobals());

  it('lists profiles with status and content counts', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        res({ environment_profiles: [summary({ name: 'py' }), summary({ name: 'node' })] })
      )
    );
    renderDrawer();

    expect(await screen.findByText('py')).toBeInTheDocument();
    expect(screen.getByText('node')).toBeInTheDocument();
    // Counts render from the summary numbers.
    expect(screen.getAllByText('2 install').length).toBeGreaterThan(0);
    expect(screen.getAllByText('1 variable').length).toBeGreaterThan(0);
    expect(screen.getAllByText('3 secret').length).toBeGreaterThan(0);
  });

  it('shows the empty state and a New environment action when there are none', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => res({ environment_profiles: [] })));
    renderDrawer();

    expect(await screen.findByText('No environments yet.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /New environment/ })).toBeInTheDocument();
  });

  it('surfaces a load failure', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => res({ error: 'boom' }, 503)));
    renderDrawer();

    expect(await screen.findByText('Could not load your environments.')).toBeInTheDocument();
  });

  it('creates a profile: PUT ok raises a toast and returns to the refreshed list', async () => {
    let profiles: EnvironmentProfileSummary[] = [];
    const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
      const method = init?.method ?? 'GET';
      if (method === 'PUT') {
        profiles = [summary({ name: 'demo', install_command_count: 1, secret_count: 0 })];
        return res(view({ name: 'demo', secret_keys: [] }));
      }
      return res({ environment_profiles: profiles });
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    renderDrawer();

    await user.click(await screen.findByRole('button', { name: /New environment/ }));
    await user.type(screen.getByLabelText('Name'), 'demo');
    await user.click(screen.getByRole('button', { name: 'Save' }));

    // Success toast (verbatim, localized) + the refreshed list showing the row.
    expect(await screen.findByText('Environment “demo” saved.')).toBeInTheDocument();
    expect(await screen.findByText('demo')).toBeInTheDocument();

    // The slow PUT actually carried the name in the URL.
    const putCall = fetchMock.mock.calls.find((c) => (c[1] as RequestInit)?.method === 'PUT');
    expect(putCall?.[0]).toContain('/environment-profiles/demo');
  });

  it('renders the install-validation report inline on a 422', async () => {
    const validation: InstallValidationError = {
      error: 'install_validation_failed',
      message: 'a command failed',
      failed_command_index: 1,
      failed_command: 'pip install nonexistent-xyz',
      exit_code: 2,
      timed_out: false,
      stderr_tail: 'ERROR: could not find nonexistent-xyz',
    };
    const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
      if ((init?.method ?? 'GET') === 'PUT') return res(validation, 422);
      return res({ environment_profiles: [] });
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    renderDrawer();

    await user.click(await screen.findByRole('button', { name: /New environment/ }));
    await user.type(screen.getByLabelText('Name'), 'x');
    await user.click(screen.getByRole('button', { name: 'Save' }));

    expect(await screen.findByText('Install validation failed')).toBeInTheDocument();
    expect(screen.getByText('pip install nonexistent-xyz')).toBeInTheDocument();
    expect(screen.getByText('a command failed')).toBeInTheDocument();
    // exit code and stderr tail surface verbatim.
    expect(screen.getByText('2')).toBeInTheDocument();
    expect(screen.getByText('ERROR: could not find nonexistent-xyz')).toBeInTheDocument();
  });

  it('never shows secret values: detail lists key names only, and the editor secret input is empty + masked', async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url.endsWith('/environment-profiles')) {
        return res({ environment_profiles: [summary({ name: 'sec', secret_count: 1 })] });
      }
      return res(view({ name: 'sec', secret_keys: ['API_KEY'] }));
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    renderDrawer();

    await user.click(await screen.findByRole('button', { name: 'Open environment sec' }));

    // Detail exposes the secret KEY name, never a value.
    expect(await screen.findByText('API_KEY')).toBeInTheDocument();
    // The non-secret variable renders as plain text; the secret carries no value.
    expect(screen.getByText('FOO=bar')).toBeInTheDocument();
    expect(screen.getByText('Values are hidden and never returned.')).toBeInTheDocument();

    // Open the editor (edit mode) — the secret value input is a masked, empty
    // password field even though the key name is pre-filled.
    await user.click(screen.getByRole('button', { name: 'Edit' }));
    const secretValue = await screen.findByLabelText('Secrets 1 value (write-only)');
    expect(secretValue).toHaveAttribute('type', 'password');
    expect(secretValue).toHaveValue('');
    // The key name IS pre-filled (write-only value is not).
    expect(screen.getByLabelText('Secrets 1 NAME')).toHaveValue('API_KEY');
  });

  it('deletes a profile through the confirm dialog and toasts', async () => {
    let profiles: EnvironmentProfileSummary[] = [summary({ name: 'gone', secret_count: 0 })];
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      const method = init?.method ?? 'GET';
      if (method === 'DELETE') {
        profiles = [];
        return res(null, 204);
      }
      if (url.endsWith('/environment-profiles')) return res({ environment_profiles: profiles });
      return res(view({ name: 'gone', secret_keys: [] }));
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    renderDrawer();

    await user.click(await screen.findByRole('button', { name: 'Open environment gone' }));
    await user.click(await screen.findByRole('button', { name: 'Delete' }));

    // The confirm dialog is its own labelled dialog; confirm inside it.
    const dialog = await screen.findByRole('dialog', { name: 'Delete environment?' });
    await user.click(within(dialog).getByRole('button', { name: 'Delete' }));

    expect(await screen.findByText('Environment “gone” deleted.')).toBeInTheDocument();
    expect(await screen.findByText('No environments yet.')).toBeInTheDocument();

    const deleteCall = fetchMock.mock.calls.find((c) => (c[1] as RequestInit)?.method === 'DELETE');
    expect(deleteCall?.[0]).toContain('/environment-profiles/gone');
  });
});
