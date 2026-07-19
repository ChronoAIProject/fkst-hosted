import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import { ToastProvider, Toaster } from '@/components/ui/toast';
import { en } from '@/i18n/en';
import { CreateRepoModal, type UserRepo } from './create-repo-modal';

const rc = en.dashboard.repos;

/** Minimal Response double for the one endpoint the modal touches. */
function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

const CREATED_REPO: UserRepo = {
  id: 7,
  owner: 'alice',
  name: 'widgets',
  private: true,
  org: false,
  admin: true,
  installed: false,
};

function renderModal(over: { onCreated?: (r: UserRepo) => void } = {}) {
  const onCreated = over.onCreated ?? vi.fn();
  const onClose = vi.fn();
  render(
    <AuthProvider>
      <ToastProvider>
        <CreateRepoModal
          viewerLogin="alice"
          orgs={['acme']}
          rc={rc}
          onClose={onClose}
          onCreated={onCreated}
        />
        <Toaster />
      </ToastProvider>
    </AuthProvider>
  );
  return { onCreated, onClose };
}

describe('CreateRepoModal', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });
  afterEach(() => vi.unstubAllGlobals());

  it('submits from the sticky-footer button, toasts success and hands back the repo', async () => {
    const user = userEvent.setup();
    vi.stubGlobal('fetch', vi.fn(async () => jsonResponse(CREATED_REPO)));
    const { onCreated } = renderModal();

    await user.type(screen.getByLabelText(rc.nameLabel), 'widgets');
    // The Create button lives in ModalShell's footer slot (outside the <form>),
    // wired back via `form=`; clicking it must still submit the form.
    await user.click(screen.getByRole('button', { name: rc.submit }));

    expect(await screen.findByText(rc.createdToast)).toBeInTheDocument();
    await waitFor(() => expect(onCreated).toHaveBeenCalledWith(CREATED_REPO));
  });

  it('surfaces the server error and raises no toast when create fails', async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => jsonResponse({ error: 'name_taken', message: 'name already exists' }, 422))
    );
    const { onCreated } = renderModal();

    await user.type(screen.getByLabelText(rc.nameLabel), 'widgets');
    await user.click(screen.getByRole('button', { name: rc.submit }));

    expect(await screen.findByText('name already exists')).toBeInTheDocument();
    expect(onCreated).not.toHaveBeenCalled();
    expect(screen.queryByText(rc.createdToast)).not.toBeInTheDocument();
  });

  it('keeps the submit button disabled until the name is valid', async () => {
    const user = userEvent.setup();
    vi.stubGlobal('fetch', vi.fn(async () => jsonResponse(CREATED_REPO)));
    renderModal();

    // Empty name → disabled.
    expect(screen.getByRole('button', { name: rc.submit })).toBeDisabled();

    // Illegal characters (spaces) fail the client-side name check → still disabled.
    await user.type(screen.getByLabelText(rc.nameLabel), 'bad name');
    expect(screen.getByRole('button', { name: rc.submit })).toBeDisabled();

    // A valid name enables it.
    await user.clear(screen.getByLabelText(rc.nameLabel));
    await user.type(screen.getByLabelText(rc.nameLabel), 'ok-name');
    expect(screen.getByRole('button', { name: rc.submit })).toBeEnabled();
  });
});
