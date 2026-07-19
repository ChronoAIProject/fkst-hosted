import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import type { IssueDetail, SessionDetail } from '@/lib/api/types';
import { SessionCard } from '@/components/sidebar/session-card';
import { SessionDetailDrawer } from './session-detail-drawer';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

const trigger: IssueDetail = {
  number: 7,
  title: 'nightly session',
  state: 'open',
  author: 'shining',
  labels: [],
  html_url: 'https://github.com/shining/lab/issues/7',
  created_at: '2026-07-01T00:00:00Z',
  updated_at: '2026-07-02T00:00:00Z',
  closed_at: null,
};

const session = (over: Partial<SessionDetail> = {}): SessionDetail => ({
  session_id: 'sess-1',
  name: 'nightly',
  work_label: 'fkst-work',
  auto_merge: true,
  environment: null,
  packages: ['ChronoAIProject/fkst-packages@fkst-hosted:codex/base'],
  invalid_reason: null,
  status_labels: ['fkst-substrate-active'],
  trigger,
  work_issues: [],
  log_url: null,
  liveness: 'live',
  prs: [],
  ...over,
});

describe('SessionDetailDrawer', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });
  afterEach(() => vi.unstubAllGlobals());

  it('opens from the SessionCard Details action and shows the header + tabs', async () => {
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <SessionCard owner="shining" name="lab" session={session()} onStop={() => {}} />
      </AuthProvider>
    );

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Open details for session nightly' }));

    const dialog = await screen.findByRole('dialog');
    expect(dialog).toBeInTheDocument();
    // Header decoded pill + the four tabs.
    expect(screen.getByRole('tab', { name: 'Status' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Packages' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Logs' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Outcomes' })).toBeInTheDocument();
  });

  it('switches to the Packages tab and renders package roles', async () => {
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <SessionDetailDrawer owner="shining" name="lab" session={session()} onClose={() => {}} />
      </AuthProvider>
    );

    await user.click(screen.getByRole('tab', { name: 'Packages' }));
    expect(screen.getByText('Base')).toBeInTheDocument();
    expect(
      screen.getByText('ChronoAIProject/fkst-packages@fkst-hosted:codex/base')
    ).toBeInTheDocument();
  });

  it('loads observe on demand and shares it into the Packages tab', async () => {
    const user = userEvent.setup();
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => jsonResponse({ queues: [{ queue: 'events', depth: 4 }], deliveries: [] }))
    );
    render(
      <AuthProvider>
        <SessionDetailDrawer owner="shining" name="lab" session={session()} onClose={() => {}} />
      </AuthProvider>
    );

    await user.click(screen.getByRole('button', { name: 'Live engine details' }));
    // The shared snapshot renders on Status…
    expect(await screen.findByText('events')).toBeInTheDocument();
    // …and is reused on Packages without a second fetch.
    await user.click(screen.getByRole('tab', { name: 'Packages' }));
    expect(screen.getByText('Queue activity')).toBeInTheDocument();
  });

  it('closes via the Close button', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(
      <AuthProvider>
        <SessionDetailDrawer owner="shining" name="lab" session={session()} onClose={onClose} />
      </AuthProvider>
    );
    await user.click(screen.getByRole('button', { name: 'Close session details' }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });
});
