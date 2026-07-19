import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import { ToastProvider, Toaster } from '@/components/ui/toast';
import { CreateWorkItemModal, buildWorkItemRequest } from './create-work-item-modal';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

/** URL discriminator for the work-items endpoint. */
const WORK_ITEMS_PATH = '/work-items';

function renderModal(
  over: { onCreated?: (r: { issue_number: number; html_url: string }) => void } = {}
) {
  const onCreated = over.onCreated ?? vi.fn();
  const onClose = vi.fn();
  render(
    <AuthProvider>
      <ToastProvider>
        <CreateWorkItemModal
          owner="acme"
          name="site"
          triggerIssue={21}
          workLabel="site-build"
          onClose={onClose}
          onCreated={onCreated}
        />
        <Toaster />
      </ToastProvider>
    </AuthProvider>
  );
  return { onCreated, onClose };
}

describe('buildWorkItemRequest', () => {
  it('trims the title and omits a blank body', () => {
    expect(buildWorkItemRequest({ title: '  do the thing  ', body: '   ' })).toEqual({
      title: 'do the thing',
    });
  });

  it('includes the body once populated', () => {
    expect(buildWorkItemRequest({ title: 't', body: '  details  ' })).toEqual({
      title: 't',
      body: 'details',
    });
  });
});

describe('CreateWorkItemModal', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });
  afterEach(() => vi.unstubAllGlobals());

  it('names the session work label the issue joins', () => {
    vi.stubGlobal('fetch', vi.fn());
    renderModal();
    expect(screen.getByText(/site-build/)).toBeInTheDocument();
  });

  it('posts the right request, shows a success toast, and hands back the issue', async () => {
    const user = userEvent.setup();
    const seen: { url: string; init?: RequestInit }[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        seen.push({ url, init });
        if (url.includes(WORK_ITEMS_PATH)) {
          return jsonResponse({
            issue_number: 77,
            html_url: 'https://github.com/acme/site/issues/77',
          });
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    const { onCreated } = renderModal();

    await user.type(screen.getByLabelText('Title'), 'build the landing page');
    await user.type(screen.getByLabelText('Details (optional)'), 'do it well');
    await user.click(screen.getByRole('button', { name: 'Queue work item' }));

    expect(await screen.findByText('Work item queued')).toBeInTheDocument();
    await waitFor(() =>
      expect(onCreated).toHaveBeenCalledWith({
        issue_number: 77,
        html_url: 'https://github.com/acme/site/issues/77',
      })
    );

    const call = seen.find((s) => s.url.includes(WORK_ITEMS_PATH));
    expect(call?.url).toContain('/repos/acme/site/sessions/21/work-items');
    expect(call?.init?.method).toBe('POST');
    expect(JSON.parse(String(call?.init?.body))).toEqual({
      title: 'build the landing page',
      body: 'do it well',
    });
  });

  it('surfaces a server error and raises no toast when the queue fails', async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes(WORK_ITEMS_PATH)) {
          return jsonResponse({ error: 'invalid', message: 'no work label on this session' }, 422);
        }
        throw new Error(`unexpected fetch: ${url}`);
      })
    );
    const { onCreated } = renderModal();

    await user.type(screen.getByLabelText('Title'), 'a task');
    await user.click(screen.getByRole('button', { name: 'Queue work item' }));

    expect(await screen.findByText('no work label on this session')).toBeInTheDocument();
    expect(onCreated).not.toHaveBeenCalled();
    expect(screen.queryByText('Work item queued')).not.toBeInTheDocument();
  });
});
