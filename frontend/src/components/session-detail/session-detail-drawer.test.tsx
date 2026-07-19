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

  it('wires every tab to a single labelled tabpanel with a roving tabindex', () => {
    render(
      <AuthProvider>
        <SessionDetailDrawer owner="shining" name="lab" session={session()} onClose={() => {}} />
      </AuthProvider>
    );

    const panel = screen.getByRole('tabpanel');
    const statusTab = screen.getByRole('tab', { name: 'Status' });
    const packagesTab = screen.getByRole('tab', { name: 'Packages' });

    // Every tab controls the one stable panel; the panel is labelled back by
    // whichever tab is active (Status on first render).
    expect(statusTab).toHaveAttribute('aria-controls', panel.id);
    expect(packagesTab).toHaveAttribute('aria-controls', panel.id);
    expect(panel).toHaveAttribute('aria-labelledby', statusTab.id);

    // Roving tabindex: only the selected tab is Tab-reachable.
    expect(statusTab).toHaveAttribute('tabindex', '0');
    expect(packagesTab).toHaveAttribute('tabindex', '-1');
    expect(statusTab).toHaveAttribute('aria-selected', 'true');
  });

  it('moves selection with ArrowRight / ArrowLeft / Home / End', async () => {
    const user = userEvent.setup();
    // Navigating onto the Outcomes tab mounts TabOutcomes, which fetches on
    // mount — give it a benign empty response so no request rejects unhandled.
    vi.stubGlobal('fetch', vi.fn(async () => jsonResponse([])));
    render(
      <AuthProvider>
        <SessionDetailDrawer owner="shining" name="lab" session={session()} onClose={() => {}} />
      </AuthProvider>
    );

    const statusTab = screen.getByRole('tab', { name: 'Status' });
    statusTab.focus();

    await user.keyboard('{ArrowRight}');
    const packagesTab = screen.getByRole('tab', { name: 'Packages' });
    expect(packagesTab).toHaveAttribute('aria-selected', 'true');
    expect(packagesTab).toHaveFocus();
    // The shared panel is now labelled by the newly-selected tab.
    expect(screen.getByRole('tabpanel')).toHaveAttribute('aria-labelledby', packagesTab.id);

    await user.keyboard('{ArrowLeft}');
    expect(statusTab).toHaveAttribute('aria-selected', 'true');
    expect(statusTab).toHaveFocus();

    await user.keyboard('{End}');
    expect(screen.getByRole('tab', { name: 'Outcomes' })).toHaveAttribute('aria-selected', 'true');

    await user.keyboard('{Home}');
    expect(screen.getByRole('tab', { name: 'Status' })).toHaveAttribute('aria-selected', 'true');
  });

  it('renders the FULL session id (not truncated) with a working copy button', async () => {
    const user = userEvent.setup();
    render(
      <AuthProvider>
        <SessionDetailDrawer
          owner="shining"
          name="lab"
          session={session({ session_id: 'sess-1234567890abcdef' })}
          onClose={() => {}}
        />
      </AuthProvider>
    );

    // Full id verbatim — the old header sliced it to the first 8 chars.
    expect(screen.getByText('sess-1234567890abcdef')).toBeInTheDocument();

    // The copy affordance carries the localized label and confirms on success
    // (userEvent installs a clipboard stub, so writeText resolves).
    await user.click(screen.getByRole('button', { name: 'Copy session ID' }));
    expect(await screen.findByText('Copied')).toBeInTheDocument();
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
