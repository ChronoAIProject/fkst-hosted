import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { AuthProvider } from '@/lib/auth/github-auth';
import { ToastProvider } from '@/components/ui/toast';
import type { IssueDetail, RepoSessionsResponse, SessionDetail } from '@/lib/api/types';
import { RepoWorkspace } from './repo-workspace';

const issue = (
  over: Partial<IssueDetail> & Pick<IssueDetail, 'number' | 'title'>
): IssueDetail => ({
  state: 'open',
  author: 'shining',
  labels: [],
  html_url: `https://github.com/shining/lab/issues/${over.number}`,
  created_at: '2026-07-01T02:00:00Z',
  updated_at: '2026-07-02T03:00:00Z',
  closed_at: null,
  ...over,
});

const session = (over: Partial<SessionDetail>): SessionDetail => ({
  session_id: 'f00dfeed-1111-2222-3333-444455556666',
  name: 'nightly',
  work_label: 'fkst-work',
  auto_merge: true,
  environment: 'staging',
  packages: ['ChronoAIProject/fkst-packages@fkst-hosted:codex/base'],
  invalid_reason: null,
  status_labels: ['fkst-substrate-active'],
  trigger: issue({ number: 7, title: 'nightly session' }),
  work_issues: [],
  log_url: null,
  liveness: 'live',
  prs: [],
  ...over,
});

const body = (sessions: SessionDetail[], installed = true): RepoSessionsResponse => ({
  owner: 'shining',
  name: 'lab',
  installed,
  sessions,
});

const alpha = session({
  session_id: 'aaaaaaaa-0000-0000-0000-000000000000',
  name: 'alpha',
  trigger: issue({ number: 1, title: 'a-trig' }),
});
const beta = session({
  session_id: 'bbbbbbbb-1111-1111-1111-111111111111',
  name: 'beta',
  trigger: issue({ number: 2, title: 'b-trig' }),
});

function renderWorkspace(props: Partial<Parameters<typeof RepoWorkspace>[0]> = {}) {
  return render(
    <MemoryRouter>
      <ToastProvider>
        <AuthProvider>
          <RepoWorkspace
            owner="shining"
            name="lab"
            data={body([alpha, beta])}
            loadFailed={false}
            onChanged={() => {}}
            {...props}
          />
        </AuthProvider>
      </ToastProvider>
    </MemoryRouter>
  );
}

describe('RepoWorkspace', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
    // The detail's default (status) tab issues no network on mount; stub fetch
    // anyway so any incidental call never hits the real network.
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>(() => {})));
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('renders the rail plus the first session detail by default', () => {
    renderWorkspace();

    // Rail: both sessions are selectable rows (the compact card is a button
    // named "Open details for session <name>").
    expect(
      screen.getByRole('button', { name: 'Open details for session alpha' })
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Open details for session beta' })
    ).toBeInTheDocument();

    // Detail pane: the inline SessionDetailView headings the FIRST session
    // (its name is the level-2 heading, distinct from the rail's <span>).
    expect(screen.getByRole('heading', { level: 2, name: 'alpha' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { level: 2, name: 'beta' })).not.toBeInTheDocument();
  });

  it('swaps the inline detail when another session is selected', async () => {
    const user = userEvent.setup();
    renderWorkspace();

    expect(screen.getByRole('heading', { level: 2, name: 'alpha' })).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Open details for session beta' }));

    // Selection is by stable key, so the detail pane now heads the second
    // session and no longer the first.
    expect(screen.getByRole('heading', { level: 2, name: 'beta' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { level: 2, name: 'alpha' })).not.toBeInTheDocument();
  });

  it('shows a placeholder and no detail heading when the repo has no sessions', () => {
    renderWorkspace({ data: body([]) });

    expect(screen.getByText('No fkst sessions in this repository.')).toBeInTheDocument();
    expect(screen.queryByRole('heading', { level: 2 })).not.toBeInTheDocument();
  });

  it('offers a Retry on a no-data load failure', async () => {
    const user = userEvent.setup();
    const onChanged = vi.fn();
    renderWorkspace({ data: null, loadFailed: true, onChanged });

    expect(
      screen.getByText('Could not load the sessions of this repository. Please try again.')
    ).toBeInTheDocument();

    // The rail's Retry re-fetches immediately rather than waiting on the poll.
    await user.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onChanged).toHaveBeenCalledTimes(1);
  });
});
