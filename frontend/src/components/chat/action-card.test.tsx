import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { AuthProvider } from '@/lib/auth/github-auth';
import { ToastProvider } from '@/components/ui/toast';
import { ChatProvider, useChat } from './chat-context';
import { MessageList } from './message-list';
import { scriptedTransport } from './chat-test-kit';

/** Expose the context so a test can drive a turn without typing. */
const captured: { current: ReturnType<typeof useChat> | null } = { current: null };
function Host() {
  captured.current = useChat();
  return <MessageList onPickStarter={() => {}} />;
}
const chat = () => captured.current!;

const target = { method: 'POST', path: '/api/v1/repos/acme/site/sessions' };

const sessionProposal = {
  kind: 'create_session',
  owner: 'acme',
  name: 'site',
  request: {
    name: 'sitebuilder',
    packages: ['acme/pkgs@main:packages/site'],
    manifests: [],
    work_label: 'site-build',
    environment: null,
    source_branch: null,
    target_branch: null,
    auto_merge: true,
    log_access: [],
    collaborators: [],
    output_lang: null,
  },
  rendered_issue_body: '### Session Name\n\nsitebuilder\n\n### Work Label\n\nsite-build\n',
  summary: 'Start session `sitebuilder` on acme/site',
  target,
};

const workItemProposal = {
  kind: 'create_work_item',
  owner: 'acme',
  name: 'site',
  trigger_issue_number: 7,
  title: 'Add the footer',
  label: 'site-build',
  body: 'Edit `src/footer.tsx`',
  summary: 'Queue a work item on acme/site #7',
  target: { method: 'POST', path: '/api/v1/repos/acme/site/sessions/7/work-items' },
};

const stopProposal = {
  kind: 'stop_session',
  owner: 'acme',
  name: 'site',
  trigger_issue_number: 7,
  reason: 'the work is finished',
  summary: 'Stop the session on acme/site',
  target: { method: 'DELETE', path: '/api/v1/repos/acme/site/sessions/7' },
};

/** Record every mutation the card triggers. */
function stubMutations(response: { status: number; body?: unknown } = { status: 201 }) {
  const calls: { url: string; method: string; body: unknown }[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({
      url: String(input),
      method: init?.method ?? 'GET',
      body: init?.body ? JSON.parse(String(init.body)) : null,
    });
    return {
      ok: response.status >= 200 && response.status < 300,
      status: response.status,
      json: async () =>
        response.body ?? { issue_number: 42, html_url: 'https://github.com/acme/site/issues/42' },
    } as Response;
  });
  vi.stubGlobal('fetch', fetchMock);
  return calls;
}

function renderWithProposal(proposal: unknown) {
  const script = scriptedTransport();
  window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  render(
    <ToastProvider>
      <AuthProvider>
        <MemoryRouter>
          <ChatProvider transport={script.transport}>
            <Host />
          </ChatProvider>
        </MemoryRouter>
      </AuthProvider>
    </ToastProvider>
  );
  act(() => chat().sendMessage('do the thing'));
  act(() => script.handlers().onActionProposal(proposal));
  act(() => script.handlers().onDone({ finishReason: 'stop', sessionRefs: [] }));
  return script;
}

describe('ActionCard', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.sessionStorage.clear();
    captured.current = null;
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  // ---- rendering ----------------------------------------------------------

  it('renders a session draft with a collapsible exact-body preview', () => {
    stubMutations();
    renderWithProposal(sessionProposal);

    expect(screen.getByTestId('chat-action-card')).toHaveAttribute('data-state', 'idle');
    expect(screen.getByText('NEW SESSION')).toBeInTheDocument();
    expect(screen.getByText('Start session `sitebuilder` on acme/site')).toBeInTheDocument();
    // The exact body a confirmation will file is behind a disclosure.
    expect(screen.queryByTestId('chat-action-preview')).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId('chat-action-preview-toggle'));
    expect(screen.getByTestId('chat-action-preview')).toHaveTextContent('### Session Name');
    // Plus the at-a-glance field table.
    expect(screen.getByText('site-build')).toBeInTheDocument();
    // The target line is shown, and the note is honest about what runs on confirm.
    expect(screen.getByText(/POST \/api\/v1\/repos\/acme\/site\/sessions/)).toBeInTheDocument();
    expect(screen.getByText(/Final permission and collision checks/)).toBeInTheDocument();
  });

  it('renders a work-item draft with its title, label and body', () => {
    stubMutations();
    renderWithProposal(workItemProposal);
    expect(screen.getByText('WORK ITEM')).toBeInTheDocument();
    expect(screen.getByText('Add the footer')).toBeInTheDocument();
    expect(screen.getByLabelText('Work item body')).toHaveTextContent('src/footer.tsx');
  });

  it('renders a stop draft with warn styling and its reason', () => {
    stubMutations();
    renderWithProposal(stopProposal);
    expect(screen.getByText('STOP SESSION')).toBeInTheDocument();
    const line = screen.getByText(/Closes trigger #7/);
    // Warning means --warn, never the brand accent.
    expect(line.className).toContain('text-warn');
    expect(screen.getByText('the work is finished')).toBeInTheDocument();
  });

  // ---- execution ----------------------------------------------------------

  it('creates the trigger through the typed API on confirm', async () => {
    const calls = stubMutations();
    renderWithProposal(sessionProposal);
    fireEvent.click(screen.getByTestId('chat-action-confirm'));

    await waitFor(() =>
      expect(screen.getByTestId('chat-action-card')).toHaveAttribute('data-state', 'succeeded')
    );
    const call = calls.find((c) => c.method === 'POST')!;
    expect(call.url).toContain('/api/v1/repos/acme/site/sessions');
    // The draft mapped onto the real request body — with no field for secrets.
    expect(call.body).toMatchObject({
      name: 'sitebuilder',
      work_label: 'site-build',
      auto_merge: true,
    });
    expect(call.body).not.toHaveProperty('disposable_environment');
  });

  it('creates the work item on the work-items endpoint', async () => {
    const calls = stubMutations();
    renderWithProposal(workItemProposal);
    fireEvent.click(screen.getByTestId('chat-action-confirm'));

    await waitFor(() =>
      expect(screen.getByTestId('chat-action-card')).toHaveAttribute('data-state', 'succeeded')
    );
    const call = calls.find((c) => c.method === 'POST')!;
    expect(call.url).toContain('/api/v1/repos/acme/site/sessions/7/work-items');
    expect(call.body).toMatchObject({ title: 'Add the footer', label: 'site-build' });
  });

  it('shows the issue and dashboard links on success', async () => {
    stubMutations();
    renderWithProposal(sessionProposal);
    fireEvent.click(screen.getByTestId('chat-action-confirm'));

    await waitFor(() => expect(screen.getByTestId('chat-action-issue-link')).toBeInTheDocument());
    expect(screen.getByTestId('chat-action-issue-link')).toHaveAttribute(
      'href',
      'https://github.com/acme/site/issues/42'
    );
    // The trigger-<n> alias is what the workspace matches, so this link keeps
    // working once the session acquires a session_id.
    expect(screen.getByTestId('chat-action-dashboard-link')).toHaveAttribute(
      'href',
      '/dashboard?owner=acme&repo=site&session=trigger-42'
    );
    expect(screen.getByText('CREATED')).toBeInTheDocument();
    // The outcome is recorded in the thread, not only on the card.
    expect(screen.getByText(/Created trigger #42 in acme\/site/)).toBeInTheDocument();
  });

  it('shows the server message in-card on failure and allows a retry', async () => {
    stubMutations({
      status: 403,
      body: { error: 'forbidden', message: 'you lack maintain on acme/site' },
    });
    renderWithProposal(sessionProposal);
    fireEvent.click(screen.getByTestId('chat-action-confirm'));

    await waitFor(() =>
      expect(screen.getByText('you lack maintain on acme/site')).toBeInTheDocument()
    );
    expect(screen.getByTestId('chat-action-card')).toHaveAttribute('data-state', 'failed');
    // Retry is allowed: the failure may be fixable (a role granted).
    expect(screen.getByTestId('chat-action-confirm')).toBeEnabled();
  });

  it('fires exactly one request when confirm is double-clicked', async () => {
    const calls = stubMutations();
    renderWithProposal(sessionProposal);
    const confirm = screen.getByTestId('chat-action-confirm');
    fireEvent.click(confirm);
    fireEvent.click(confirm);

    await waitFor(() =>
      expect(screen.getByTestId('chat-action-card')).toHaveAttribute('data-state', 'succeeded')
    );
    expect(calls.filter((c) => c.method === 'POST')).toHaveLength(1);
  });

  it('never re-executes a succeeded proposal', async () => {
    const calls = stubMutations();
    renderWithProposal(sessionProposal);
    fireEvent.click(screen.getByTestId('chat-action-confirm'));
    await waitFor(() =>
      expect(screen.getByTestId('chat-action-card')).toHaveAttribute('data-state', 'succeeded')
    );
    // The success row has no confirm button at all — the state machine, not styling,
    // is what prevents a second run.
    expect(screen.queryByTestId('chat-action-confirm')).not.toBeInTheDocument();
    expect(calls.filter((c) => c.method === 'POST')).toHaveLength(1);
  });

  it('dismisses a proposal without executing anything', () => {
    const calls = stubMutations();
    renderWithProposal(sessionProposal);
    fireEvent.click(screen.getByTestId('chat-action-dismiss'));
    expect(screen.queryByTestId('chat-action-card')).not.toBeInTheDocument();
    expect(calls.filter((c) => c.method === 'POST')).toHaveLength(0);
  });

  // ---- the stop path ------------------------------------------------------

  it('routes a stop through ConfirmDialog and executes it exactly once', async () => {
    const calls = stubMutations({ status: 204, body: null });
    renderWithProposal(stopProposal);

    // Confirming on the card opens the dialog; it does NOT stop the session.
    fireEvent.click(screen.getByTestId('chat-action-confirm'));
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    expect(calls.filter((c) => c.method === 'DELETE')).toHaveLength(0);

    fireEvent.click(screen.getByRole('button', { name: 'Stop session' }));
    await waitFor(() =>
      expect(screen.getByTestId('chat-action-card')).toHaveAttribute('data-state', 'succeeded')
    );
    // The dialog owns the mutation; the card must not run it a second time.
    expect(calls.filter((c) => c.method === 'DELETE')).toHaveLength(1);
    expect(screen.getByText('STOPPED')).toBeInTheDocument();
    expect(screen.getByText(/Closed trigger #7 in acme\/site/)).toBeInTheDocument();
  });

  it('leaves a stop card untouched when the dialog is cancelled', async () => {
    const calls = stubMutations();
    renderWithProposal(stopProposal);
    fireEvent.click(screen.getByTestId('chat-action-confirm'));
    const dialog = await screen.findByRole('dialog');
    // Scoped to the dialog: the card behind it carries its own Dismiss button.
    fireEvent.click(within(dialog).getByRole('button', { name: 'DISMISS' }));

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(screen.getByTestId('chat-action-card')).toHaveAttribute('data-state', 'idle');
    expect(calls.filter((c) => c.method === 'DELETE')).toHaveLength(0);
  });
});
