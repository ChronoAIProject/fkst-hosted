import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { AuthProvider } from '@/lib/auth/github-auth';
import type { IssueDetail, RepoSessionsResponse, SessionDetail } from '@/lib/api/types';
import { Level2Sidebar } from './level2';

function jsonResponse(body: unknown, status = 200): Response {
  return { ok: status >= 200 && status < 300, status, json: async () => body } as Response;
}

const issue = (over: Partial<IssueDetail> & Pick<IssueDetail, 'number' | 'title'>): IssueDetail => ({
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
  work_issues: [issue({ number: 9, title: 'implement the thing', state: 'closed' })],
  log_url: 'https://api.example.test/api/v1/logs/f00dfeed-1111-2222-3333-444455556666',
  liveness: 'live',
  prs: [
    {
      number: 12,
      title: 'feat: the thing',
      html_url: 'https://github.com/shining/lab/pull/12',
      state: 'closed',
      merged: true,
      work_issue: 9,
    },
  ],
  ...over,
});

const body = (sessions: SessionDetail[], installed = true): RepoSessionsResponse => ({
  owner: 'shining',
  name: 'lab',
  installed,
  sessions,
});

function renderLevel2(props: Partial<Parameters<typeof Level2Sidebar>[0]> = {}) {
  return render(
    <AuthProvider>
      <Level2Sidebar
        owner="shining"
        name="lab"
        data={body([session({})])}
        loadFailed={false}
        onChanged={() => {}}
        {...props}
      />
    </AuthProvider>
  );
}

describe('Level2Sidebar', () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.location.hash = '';
    window.localStorage.setItem('fkst-gh-access', 'ghu_x');
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('renders a session card with config metadata, liveness, log link and outcomes', () => {
    renderLevel2();

    // View description + legend are always present and name the repo.
    expect(screen.getByText(/shining\/lab/)).toBeInTheDocument();
    expect(screen.getByText('Legend')).toBeInTheDocument();

    // Session header: name, status label chip, liveness, auto-merge.
    expect(screen.getByText('nightly')).toBeInTheDocument();
    expect(screen.getByText('fkst-substrate-active')).toBeInTheDocument();
    expect(screen.getByText('live')).toBeInTheDocument();
    expect(screen.getByText('auto-merge')).toBeInTheDocument();

    // Config metadata: work label + environment + packages.
    expect(screen.getByText('fkst-work')).toBeInTheDocument();
    expect(screen.getByText('staging')).toBeInTheDocument();
    expect(
      screen.getByText('ChronoAIProject/fkst-packages@fkst-hosted:codex/base')
    ).toBeInTheDocument();

    // Timestamps from the trigger issue (SGT-rendered).
    expect(screen.getByText(/created .*SGT/)).toBeInTheDocument();
    expect(screen.getByText(/updated .*SGT/)).toBeInTheDocument();

    // Log download link goes straight at log_url.
    const log = screen.getByRole('link', { name: /Download logs/ });
    expect(log).toHaveAttribute(
      'href',
      'https://api.example.test/api/v1/logs/f00dfeed-1111-2222-3333-444455556666'
    );

    // Trigger + work issue links.
    expect(screen.getByRole('link', { name: '#7' })).toHaveAttribute(
      'href',
      'https://github.com/shining/lab/issues/7'
    );
    expect(screen.getByRole('link', { name: '#9' })).toHaveAttribute(
      'href',
      'https://github.com/shining/lab/issues/9'
    );

    // PR outcome row: link, merged chip, work-issue backlink.
    expect(screen.getByRole('link', { name: '#12' })).toHaveAttribute(
      'href',
      'https://github.com/shining/lab/pull/12'
    );
    expect(screen.getByText('merged')).toBeInTheDocument();
    expect(screen.getByText('for #9')).toBeInTheDocument();
  });

  it('shows the invalid reason verbatim and the not-installed note', () => {
    renderLevel2({
      data: body(
        [session({ invalid_reason: 'Packages: line 2 is unreachable.', name: null })],
        false
      ),
    });
    expect(screen.getByText('Invalid trigger')).toBeInTheDocument();
    expect(screen.getByText('Packages: line 2 is unreachable.')).toBeInTheDocument();
    expect(
      screen.getByText('The App is not installed on this repository, so sessions cannot run here.')
    ).toBeInTheDocument();
  });

  it('shows empty and failure states', () => {
    renderLevel2({ data: body([]) });
    expect(screen.getByText('No fkst sessions in this repository.')).toBeInTheDocument();

    renderLevel2({ data: null, loadFailed: true });
    expect(
      screen.getByText('Could not load the sessions of this repository. Please try again.')
    ).toBeInTheDocument();
  });

  it('creates a trigger: POSTs the form, omits blank optionals, notifies', async () => {
    const user = userEvent.setup();
    const onChanged = vi.fn();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith('/api/v1/repos/shining/lab/sessions') && init?.method === 'POST') {
        return jsonResponse(
          { issue_number: 30, html_url: 'https://github.com/shining/lab/issues/30' },
          201
        );
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    renderLevel2({ onChanged });

    await user.click(screen.getByRole('button', { name: 'New session' }));
    const dialog = await screen.findByRole('dialog');
    const submit = within(dialog).getByRole('button', { name: 'Create trigger issue' });
    expect(submit).toBeDisabled(); // name + >=1 package required

    await user.type(within(dialog).getByLabelText('Session name'), 'workhorse');
    expect(submit).toBeDisabled(); // still no package

    await user.type(within(dialog).getByLabelText('Packages 1'), 'o/p@main:pkg/a');
    // A second package row can be added and removed.
    await user.click(within(dialog).getByRole('button', { name: 'Add package' }));
    await user.type(within(dialog).getByLabelText('Packages 2'), 'o/p@main:pkg/b');
    await user.click(within(dialog).getByRole('button', { name: 'Remove package 2' }));

    await user.type(within(dialog).getByLabelText('Work label (optional)'), 'lab-work');
    await user.click(within(dialog).getByRole('checkbox', { name: 'Auto-merge' }));
    await user.click(submit);

    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    const post = fetchMock.mock.calls.find(([, init]) => init?.method === 'POST')!;
    expect(JSON.parse(String(post[1]!.body))).toEqual({
      name: 'workhorse',
      packages: ['o/p@main:pkg/a'],
      work_label: 'lab-work',
      auto_merge: true,
    });
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('surfaces the create 400 message verbatim and keeps the dialog open', async () => {
    const user = userEvent.setup();
    const message = 'Session Name: must be a single DNS-label-ish line.';
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => jsonResponse({ error: 'invalid_trigger', message }, 400))
    );
    renderLevel2();

    await user.click(screen.getByRole('button', { name: 'New session' }));
    const dialog = await screen.findByRole('dialog');
    await user.type(within(dialog).getByLabelText('Session name'), 'Bad Name');
    await user.type(within(dialog).getByLabelText('Packages 1'), 'o/p@main:pkg');
    await user.click(within(dialog).getByRole('button', { name: 'Create trigger issue' }));

    expect(await within(dialog).findByText(message)).toBeInTheDocument();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('stops a session through the confirm dialog (DELETE) and notifies', async () => {
    const user = userEvent.setup();
    const onChanged = vi.fn();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      if (init?.method === 'DELETE') return jsonResponse(null, 204);
      throw new Error(`unexpected fetch: ${String(input)}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    renderLevel2({ onChanged });

    await user.click(screen.getByRole('button', { name: 'Stop session nightly' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('Stop session nightly?')).toBeInTheDocument();
    expect(within(dialog).getByText(/#7/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Stop session' }));

    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    const del = fetchMock.mock.calls.find(([, init]) => init?.method === 'DELETE')!;
    expect(String(del[0])).toMatch(/\/api\/v1\/repos\/shining\/lab\/sessions\/7$/);
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('keeps the stop dialog open with the envelope message on failure', async () => {
    const user = userEvent.setup();
    const message = 'GitHub said no: not authorized to close this issue.';
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => jsonResponse({ error: 'forbidden', message }, 403))
    );
    renderLevel2();

    await user.click(screen.getByRole('button', { name: 'Stop session nightly' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: 'Stop session' }));

    expect(await within(dialog).findByText(message)).toBeInTheDocument();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('offers no Stop button on a closed trigger', () => {
    renderLevel2({
      data: body([
        session({
          trigger: issue({ number: 7, title: 'done', state: 'closed', closed_at: '2026-07-03T00:00:00Z' }),
          status_labels: [],
          liveness: null,
        }),
      ]),
    });
    expect(screen.queryByRole('button', { name: /Stop session/ })).not.toBeInTheDocument();
    expect(screen.getByText(/closed .*SGT/)).toBeInTheDocument();
  });
});
