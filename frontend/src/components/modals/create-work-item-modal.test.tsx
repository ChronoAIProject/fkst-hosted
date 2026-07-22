import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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
  over: {
    onCreated?: (r: { issue_number: number; html_url: string }) => void;
    workLabels?: string[];
    creator?: string;
  } = {}
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
          creator={over.creator ?? 'session-owner'}
          workLabels={over.workLabels ?? ['site-build', 'fkst-security']}
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
    expect(
      buildWorkItemRequest({ title: '  do the thing  ', body: '   ', workLabel: 'site-build' })
    ).toEqual({
      title: 'do the thing',
      label: 'site-build',
    });
  });

  it('includes populated Markdown without changing its whitespace', () => {
    expect(
      buildWorkItemRequest({ title: 't', body: '  details\n', workLabel: 'fkst-security' })
    ).toEqual({
      title: 't',
      label: 'fkst-security',
      body: '  details\n',
    });
  });
});

describe('CreateWorkItemModal', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });
  afterEach(() => vi.unstubAllGlobals());

  it('offers every applicable session work label', () => {
    vi.stubGlobal('fetch', vi.fn());
    renderModal();
    expect(screen.getByRole('combobox', { name: 'Work label' })).toHaveValue('site-build');
    expect(screen.getByText(/Opens an issue labeled `site-build`/)).toBeInTheDocument();
    expect(screen.getByText(/assigned to `@session-owner`/)).toBeInTheDocument();
    expect(screen.getByRole('option', { name: 'fkst-security' })).toBeInTheDocument();
  });

  it('renders one applicable label as a fixed value instead of a picker', () => {
    vi.stubGlobal('fetch', vi.fn());
    renderModal({ workLabels: ['site-build'] });

    expect(screen.queryByRole('combobox', { name: 'Work label' })).not.toBeInTheDocument();
    expect(screen.getByText('site-build')).toBeInTheDocument();
    expect(screen.getByText(/Opens an issue labeled `site-build`/)).toBeInTheDocument();
  });

  it('previews Markdown safely and preserves the raw body when returning to Write', async () => {
    const user = userEvent.setup();
    vi.stubGlobal('fetch', vi.fn());
    renderModal();
    const raw = '# Plan\n\n- verify **routing**\n\nRead [the docs](https://example.com/docs).';

    fireEvent.change(screen.getByLabelText('Details (optional)'), { target: { value: raw } });
    await user.click(screen.getByRole('button', { name: 'Preview' }));

    expect(screen.getByRole('button', { name: 'Preview' })).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    const preview = screen.getByRole('region', { name: 'Markdown preview' });
    expect(within(preview).getByRole('heading', { level: 1, name: 'Plan' })).toBeInTheDocument();
    expect(within(preview).getByText('routing').tagName).toBe('STRONG');
    expect(within(preview).getByRole('link', { name: 'the docs' })).toHaveAttribute(
      'href',
      'https://example.com/docs'
    );

    await user.click(screen.getByRole('button', { name: 'Write' }));
    expect(screen.getByLabelText('Details (optional)')).toHaveValue(raw);
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
    await user.selectOptions(screen.getByRole('combobox', { name: 'Work label' }), 'fkst-security');
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
      label: 'fkst-security',
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
